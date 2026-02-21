use indexmap::IndexMap;
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Matcher, Utf32Str};

use crate::catalog::{ArtistTarget, Catalog};
use crate::config::Config;
use crate::models::CatalogShow;
use crate::service::router::ServiceRouter;

use super::prompt::{styled_select, styled_text, PromptResult};
use super::show::{show_detail, show_list};
use super::style::{clear_screen, dim, format_show_label, print_section};

/// Score an artist name against the query using nucleo fuzzy matching.
fn score_artist(query: &str, name: &str) -> Option<u32> {
    let pattern = Pattern::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);
    let mut buf = Vec::new();
    let hay = Utf32Str::new(name, &mut buf);
    pattern.score(hay, &mut matcher)
}

/// Minimum nucleo score for artist name matches.
/// Calibrated: "dead" matches "Dead & Company", "phish" matches "Phish",
/// "xyz" matches nothing. Nucleo scores differ from rapidfuzz WRatio.
const ARTIST_MATCH_THRESHOLD: u32 = 80;
const MAX_ARTIST_MATCHES: usize = 5;
const MAX_SHOW_RESULTS: usize = 50;

/// Search shows and artists — dual-mode search with text input.
pub async fn search_shows(
    catalog: &mut Catalog,
    router: &mut ServiceRouter,
    config: &mut Config,
    queue: &mut IndexMap<i64, CatalogShow>,
) {
    loop {
        clear_screen();
        print_section("Search", None);

        let query = match styled_text("Search:") {
            PromptResult::Choice(q) => q,
            PromptResult::Back | PromptResult::Interrupted => return,
        };

        // Score artist names
        let artist_choices = catalog.get_all_artist_choices();
        let mut artist_matches: Vec<(i64, String, u32)> = artist_choices
            .iter()
            .filter_map(|(id, name)| {
                score_artist(&query, name)
                    .filter(|&score| score >= ARTIST_MATCH_THRESHOLD)
                    .map(|score| (*id, name.clone(), score))
            })
            .collect();
        artist_matches.sort_by(|a, b| b.2.cmp(&a.2));
        artist_matches.truncate(MAX_ARTIST_MATCHES);

        // Search shows
        let show_results = catalog.search(&query, MAX_SHOW_RESULTS);

        if artist_matches.is_empty() && show_results.is_empty() {
            println!("  No results found.");
            continue;
        }

        // Build combined choice list
        let mut labels: Vec<String> = Vec::new();
        let mut actions: Vec<SearchAction> = Vec::new();

        // Artist matches first
        for (artist_id, name, _) in &artist_matches {
            labels.push(format!("\u{2192} {}", dim_artist_label(name)));
            actions.push(SearchAction::Artist(*artist_id));
        }

        // Show results
        for show in &show_results {
            let prefix = if queue.contains_key(&show.container_id) {
                "\u{2713} "
            } else {
                "  "
            };
            labels.push(format_show_label(show, prefix, true));
            actions.push(SearchAction::Show(show.container_id));
        }

        let selected = match styled_select("", labels.clone()) {
            PromptResult::Choice(label) => labels.iter().position(|l| *l == label),
            PromptResult::Back | PromptResult::Interrupted => return,
        };

        if let Some(pos) = selected {
            match &actions[pos] {
                SearchAction::Artist(artist_id) => {
                    println!("  {}", dim("Loading catalog..."));
                    if let Some(resolved_id) = catalog
                        .load_artist(router, ArtistTarget::Id(*artist_id), true)
                        .await
                    {
                        let shows = catalog.get_shows_by_artist_id(resolved_id);
                        if !shows.is_empty() {
                            let name = shows[0].artist_name.clone();
                            show_list(&shows, router, config, queue, &name).await;
                        }
                    }
                }
                SearchAction::Show(container_id) => {
                    if let Some(show) = show_results
                        .iter()
                        .find(|s| s.container_id == *container_id)
                    {
                        show_detail(show, router, config, queue, None).await;
                    }
                }
            }
        }
    }
}

enum SearchAction {
    Artist(i64),
    Show(i64),
}

/// Format an artist match label with dim suffix.
fn dim_artist_label(name: &str) -> String {
    format!("{name}  {}", dim("(artist catalog)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_artist_exact_match() {
        let score = score_artist("phish", "Phish");
        assert!(score.is_some());
        assert!(score.unwrap() >= ARTIST_MATCH_THRESHOLD);
    }

    #[test]
    fn test_score_artist_partial_match() {
        let score = score_artist("dead", "Dead & Company");
        assert!(score.is_some());
        assert!(score.unwrap() >= ARTIST_MATCH_THRESHOLD);
    }

    #[test]
    fn test_score_artist_no_match() {
        let score = score_artist("xyzzyqwert", "Dead & Company");
        assert!(score.is_none() || score.unwrap() < ARTIST_MATCH_THRESHOLD);
    }

    #[test]
    fn test_dim_artist_label() {
        let label = dim_artist_label("Phish");
        assert!(label.contains("Phish"));
        assert!(label.contains("artist catalog"));
    }
}
