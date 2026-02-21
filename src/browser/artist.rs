use std::collections::BTreeMap;

use indexmap::IndexMap;

use crate::catalog::{ArtistTarget, Catalog};
use crate::config::recents::{load_recents, record_recent};
use crate::config::Config;
use crate::models::CatalogShow;
use crate::service::router::ServiceRouter;

use super::prompt::{styled_confirm, styled_fuzzy, styled_select, styled_text, PromptResult};
use super::queue::download_queued_shows;
use super::show::show_list;
use super::style::{clear_screen, dim, print_section};

/// Browse all artists via fuzzy selection.
pub async fn browse_by_artist(
    catalog: &mut Catalog,
    router: &mut ServiceRouter,
    config: &mut Config,
    queue: &mut IndexMap<i64, CatalogShow>,
) {
    loop {
        clear_screen();
        print_section("All Artists", None);

        // Ensure artist discovery has run
        if !catalog.has_discovered() {
            println!("  {}", dim("Discovering artists..."));
        }
        catalog.discover_if_needed(router).await;

        let artist_choices = catalog.get_all_artist_choices();
        if artist_choices.is_empty() {
            println!("No artists found. Try refreshing the catalog.");
            return;
        }

        // Build choice labels: "Artist Name"
        let labels: Vec<String> = artist_choices
            .iter()
            .map(|(_, name)| name.clone())
            .collect();

        match styled_fuzzy("Select artist:", labels) {
            PromptResult::Choice(name) => {
                // Find the artist_id for the selected name
                if let Some(&(artist_id, _)) = artist_choices.iter().find(|(_, n)| *n == name) {
                    println!("  {}", dim("Loading catalog..."));
                    match catalog
                        .load_artist(router, ArtistTarget::Id(artist_id), true)
                        .await
                    {
                        Some(resolved_id) => {
                            browse_artist(resolved_id, catalog, router, config, queue).await;
                        }
                        None => {
                            println!("Could not load artist catalog.");
                        }
                    }
                }
            }
            PromptResult::Back | PromptResult::Interrupted => return,
        }
    }
}

/// Artist sub-menu with browse by year and search.
async fn browse_artist(
    artist_id: i64,
    catalog: &mut Catalog,
    router: &mut ServiceRouter,
    config: &mut Config,
    queue: &mut IndexMap<i64, CatalogShow>,
) {
    let shows = catalog.get_shows_by_artist_id(artist_id);
    if shows.is_empty() {
        println!("No shows found for this artist.");
        return;
    }

    // Record as recent (after guard, matching Python behavior)
    record_recent(artist_id);

    let artist_name = shows
        .first()
        .map(|s| s.artist_name.clone())
        .unwrap_or_else(|| {
            catalog
                .get_artist_name(artist_id)
                .unwrap_or("Unknown Artist")
                .to_string()
        });

    loop {
        let show_count = catalog.get_shows_by_artist_id(artist_id).len();

        clear_screen();
        print_section(&artist_name, Some(&format!("{show_count} shows")));

        let choices = vec![
            format!("Browse by year ({show_count} shows)"),
            "Search shows".to_string(),
            format!("Download all ({show_count} shows)"),
        ];

        match styled_select("", choices) {
            PromptResult::Choice(choice) => {
                if choice.starts_with("Browse by year") {
                    browse_artist_years(artist_id, &artist_name, catalog, router, config, queue)
                        .await;
                } else if choice == "Search shows" {
                    search_artist_shows(artist_id, &artist_name, catalog, router, config, queue)
                        .await;
                } else if choice.starts_with("Download all") {
                    download_all_artist(artist_id, &artist_name, catalog, router, config).await;
                }
            }
            PromptResult::Back | PromptResult::Interrupted => return,
        }
    }
}

/// Year list scoped to one artist.
async fn browse_artist_years(
    artist_id: i64,
    artist_name: &str,
    catalog: &mut Catalog,
    router: &mut ServiceRouter,
    config: &mut Config,
    queue: &mut IndexMap<i64, CatalogShow>,
) {
    loop {
        clear_screen();

        let shows = catalog.get_shows_by_artist_id(artist_id);
        let mut by_year: BTreeMap<String, Vec<CatalogShow>> = BTreeMap::new();
        for show in &shows {
            let year = if show.performance_date_year.is_empty() {
                "Unknown"
            } else {
                &show.performance_date_year
            };
            by_year.entry(year.to_string()).or_default().push(show.clone());
        }

        if by_year.is_empty() {
            println!("No shows available.");
            return;
        }

        // Sort years descending
        let years: Vec<String> = by_year.keys().rev().cloned().collect();

        print_section(
            &format!("{artist_name} \u{00b7} Years"),
            Some(&format!(
                "{} year{}",
                years.len(),
                if years.len() != 1 { "s" } else { "" }
            )),
        );

        let labels: Vec<String> = years
            .iter()
            .map(|y| {
                let count = by_year[y].len();
                format!("{y} ({count} show{})", if count != 1 { "s" } else { "" })
            })
            .collect();

        match styled_fuzzy("Select year:", labels.clone()) {
            PromptResult::Choice(label) => {
                if let Some(pos) = labels.iter().position(|l| *l == label) {
                    let year = &years[pos];
                    let mut year_shows = by_year.remove(year).unwrap_or_default();
                    year_shows.sort_by(|a, b| b.performance_date.cmp(&a.performance_date));
                    let title = format!("{artist_name} \u{2014} {year}");
                    show_list(&year_shows, router, config, queue, &title).await;
                }
            }
            PromptResult::Back | PromptResult::Interrupted => return,
        }
    }
}

/// Search scoped to one artist.
async fn search_artist_shows(
    artist_id: i64,
    artist_name: &str,
    catalog: &mut Catalog,
    router: &mut ServiceRouter,
    config: &mut Config,
    queue: &mut IndexMap<i64, CatalogShow>,
) {
    clear_screen();
    print_section(&format!("Search {artist_name}"), None);

    let query = match styled_text(&format!(
        "Search {} {}",
        artist_name,
        dim("(venue, city, date, song...)")
    )) {
        PromptResult::Choice(text) => text,
        PromptResult::Back | PromptResult::Interrupted => return,
    };

    let trimmed = query.trim();
    if trimmed.is_empty() {
        return;
    }

    let results = catalog.search_artist(trimmed, artist_id, 50);
    if results.is_empty() {
        println!("  \x1b[38;5;214mNo results found.\x1b[0m");
        return;
    }

    let title = format!("{artist_name} \u{2014} \"{trimmed}\"");
    show_list(&results, router, config, queue, &title).await;
}

/// Browse recently accessed artists (loops until back).
pub async fn recents(
    catalog: &mut Catalog,
    router: &mut ServiceRouter,
    config: &mut Config,
    queue: &mut IndexMap<i64, CatalogShow>,
) {
    loop {
        clear_screen();
        print_section("Recents", None);

        let recent_map = load_recents();
        if recent_map.is_empty() {
            println!("No recent artists yet.");
            return;
        }

        // Sort by timestamp descending (most recent first)
        let mut entries: Vec<(i64, f64)> = recent_map.into_iter().collect();
        entries.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Build choice labels with artist names
        let filtered: Vec<(i64, String)> = entries
            .iter()
            .filter_map(|&(artist_id, _)| {
                let name = catalog.get_artist_name(artist_id)?;
                catalog.artist_has_data(artist_id).then(|| (artist_id, name.to_string()))
            })
            .collect();

        let labels: Vec<String> = filtered.iter().map(|(_, name)| name.clone()).collect();
        let ids: Vec<i64> = filtered.iter().map(|(id, _)| *id).collect();

        if labels.is_empty() {
            println!("No recent artists with cached data.");
            return;
        }

        match styled_fuzzy("Select artist:", labels.clone()) {
            PromptResult::Choice(name) => {
                if let Some(pos) = labels.iter().position(|l| *l == name) {
                    let artist_id = ids[pos];
                    println!("  {}", dim("Loading catalog..."));
                    match catalog
                        .load_artist(router, ArtistTarget::Id(artist_id), true)
                        .await
                    {
                        Some(resolved_id) => {
                            browse_artist(resolved_id, catalog, router, config, queue).await;
                        }
                        None => {
                            println!("Could not load artist catalog.");
                        }
                    }
                }
            }
            PromptResult::Back | PromptResult::Interrupted => return,
        }
    }
}

/// Download all shows for an artist.
async fn download_all_artist(
    artist_id: i64,
    artist_name: &str,
    catalog: &Catalog,
    router: &mut ServiceRouter,
    config: &mut Config,
) {
    let shows = catalog.get_shows_by_artist_id(artist_id);
    if shows.is_empty() {
        println!("No shows found for this artist.");
        return;
    }

    let count = shows.len();
    let msg = format!("Download all {count} shows for {artist_name}?");
    match styled_confirm(&msg, false) {
        PromptResult::Choice(true) => {}
        _ => return,
    }

    // Build temporary queue from the artist's catalog
    let mut temp_queue: IndexMap<i64, CatalogShow> = IndexMap::new();
    for show in shows {
        temp_queue.insert(show.container_id, show);
    }

    download_queued_shows(&mut temp_queue, router, config).await;
}
