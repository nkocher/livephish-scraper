use std::collections::HashSet;

use indexmap::IndexMap;
use once_cell::sync::Lazy;
use regex::Regex;

use crate::api::NugsApi;
use crate::catalog::{ArtistTarget, Catalog};
use crate::config::Config;
use crate::models::CatalogShow;
use crate::service::router::ServiceRouter;

use super::playlist::playlist_detail;
use super::prompt::{styled_text, PromptResult};
use super::show::{show_detail, show_list};
use super::style::{clear_screen, dim, print_section};

// ── Static regexes for URL parsing ──────────────────────────────────

static RELEASE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"play\.nugs\.net/release/(\d+)").unwrap());
static PLAYLIST_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"play\.nugs\.net/(?:#/playlists|library)/playlist/(\d+)").unwrap());
static SHORTLINK_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"2nu\.gs/([a-zA-Z\d]+)").unwrap());
static ARTIST_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"play\.nugs\.net/artist/(\d+)").unwrap());
static BARE_ID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d+$").unwrap());

/// Parsed URL type and ID.
#[derive(Debug, Clone, PartialEq)]
enum UrlType {
    Release,
    Playlist,
    Shortlink,
    Artist,
}

impl UrlType {
    fn key_suffix(&self) -> &str {
        match self {
            UrlType::Release => "release",
            UrlType::Playlist => "playlist",
            UrlType::Shortlink => "shortlink",
            UrlType::Artist => "artist",
        }
    }
}

#[derive(Debug, Clone)]
struct ParsedUrl {
    id: String,
    url_type: UrlType,
}

/// Parse text input into a list of (id, type) pairs.
///
/// Supports:
/// - play.nugs.net/release/{id}
/// - play.nugs.net/(#/playlists|library)/playlist/{id}
/// - 2nu.gs/{id}
/// - play.nugs.net/artist/{id}
/// - Bare numeric IDs (treated as releases)
fn parse_urls(text: &str) -> Vec<ParsedUrl> {
    let patterns: &[(&Regex, UrlType)] = &[
        (&RELEASE_RE, UrlType::Release),
        (&PLAYLIST_RE, UrlType::Playlist),
        (&SHORTLINK_RE, UrlType::Shortlink),
        (&ARTIST_RE, UrlType::Artist),
    ];

    let mut results = Vec::new();
    let mut seen = HashSet::new();

    // Match URL patterns
    for (regex, url_type) in patterns {
        for cap in regex.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                let id = m.as_str().to_string();
                let key = format!("{}:{}", id, url_type.key_suffix());
                if seen.insert(key) {
                    results.push(ParsedUrl {
                        id,
                        url_type: url_type.clone(),
                    });
                }
            }
        }
    }

    // Check for bare numeric IDs (split on whitespace/commas)
    for token in text.split(|c: char| c.is_whitespace() || c == ',') {
        let token = token.trim();
        if BARE_ID_RE.is_match(token) {
            let key = format!("{token}:release");
            if seen.insert(key) {
                results.push(ParsedUrl {
                    id: token.to_string(),
                    url_type: UrlType::Release,
                });
            }
        }
    }

    results
}

/// Import URL flow — text input, parse, route by type.
pub async fn import_url(
    catalog: &mut Catalog,
    router: &mut ServiceRouter,
    config: &mut Config,
    queue: &mut IndexMap<i64, CatalogShow>,
) {
    clear_screen();
    print_section("Import URL", None);

    let input = match styled_text("URL or container ID:") {
        PromptResult::Choice(text) => text,
        PromptResult::Back | PromptResult::Interrupted => return,
    };

    let parsed = parse_urls(&input);
    if parsed.is_empty() {
        println!("  \x1b[38;5;214mNo valid URLs or IDs found.\x1b[0m");
        return;
    }

    let total = parsed.len();
    for (i, item) in parsed.iter().enumerate() {
        if total > 1 {
            clear_screen();
            println!("\n  \x1b[1;38;5;214m[{}/{}]\x1b[0m", i + 1, total);
        }

        match item.url_type {
            UrlType::Release | UrlType::Shortlink => {
                handle_release(&item.id, catalog, router, config, queue).await
            }
            UrlType::Artist => handle_artist(&item.id, catalog, router, config, queue).await,
            // Playlists are nugs-only
            UrlType::Playlist => handle_playlist(&item.id, &mut router.nugs, config).await,
        }
    }
}

/// Handle a release ID: fetch show detail, display.
async fn handle_release(
    id: &str,
    catalog: &mut Catalog,
    router: &mut ServiceRouter,
    config: &mut Config,
    queue: &mut IndexMap<i64, CatalogShow>,
) {
    let container_id: i64 = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            println!("  \x1b[38;5;214mInvalid ID: {id}\x1b[0m");
            return;
        }
    };

    println!("  {}", dim("Fetching show..."));
    // Import URLs are nugs.net URLs; always use nugs API
    match router.nugs.get_show_detail(container_id).await {
        Ok(show) => {
            catalog.register_artist(show.artist_id, &show.artist_name);
            let catalog_show = CatalogShow::from_show(&show);
            show_detail(&catalog_show, router, config, queue, Some(show)).await;
        }
        Err(e) => {
            println!("  \x1b[38;5;203mFailed to fetch release {id}: {e}\x1b[0m");
        }
    }
}

/// Handle an artist ID: load catalog, browse shows.
async fn handle_artist(
    id: &str,
    catalog: &mut Catalog,
    router: &mut ServiceRouter,
    config: &mut Config,
    queue: &mut IndexMap<i64, CatalogShow>,
) {
    let artist_id: i64 = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            println!("  \x1b[38;5;214mInvalid artist ID: {id}\x1b[0m");
            return;
        }
    };

    println!("  {}", dim("Loading artist catalog..."));
    match catalog
        .load_artist(router, ArtistTarget::Id(artist_id), true)
        .await
    {
        Some(resolved_id) => {
            let shows = catalog.get_shows_by_artist_id(resolved_id);
            if shows.is_empty() {
                println!("  No shows found for this artist.");
                return;
            }
            let name = shows[0].artist_name.clone();
            show_list(&shows, router, config, queue, &name).await;
        }
        None => {
            println!("  \x1b[38;5;203mCould not load artist {id}.\x1b[0m");
        }
    }
}

/// Handle a playlist ID: resolve GUID, fetch, display.
async fn handle_playlist(id: &str, api: &mut NugsApi, config: &mut Config) {
    println!("  {}", dim("Resolving playlist..."));
    let pl_guid = match api.resolve_playlist_id(id).await {
        Ok(guid) => guid,
        Err(e) => {
            println!("  \x1b[38;5;203mFailed to resolve playlist {id}: {e}\x1b[0m");
            return;
        }
    };

    match api.get_playlist(&pl_guid, true).await {
        Ok(playlist) => {
            playlist_detail(&playlist, api, config).await;
        }
        Err(e) => {
            println!("  \x1b[38;5;203mFailed to fetch playlist: {e}\x1b[0m");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_release_url() {
        let results = parse_urls("https://play.nugs.net/release/12345");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "12345");
        assert_eq!(results[0].url_type, UrlType::Release);
    }

    #[test]
    fn test_parse_playlist_url() {
        let results = parse_urls("https://play.nugs.net/#/playlists/playlist/67890");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "67890");
        assert_eq!(results[0].url_type, UrlType::Playlist);
    }

    #[test]
    fn test_parse_library_playlist_url() {
        let results = parse_urls("https://play.nugs.net/library/playlist/67890");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "67890");
        assert_eq!(results[0].url_type, UrlType::Playlist);
    }

    #[test]
    fn test_parse_shortlink() {
        let results = parse_urls("https://2nu.gs/abc123");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "abc123");
        assert_eq!(results[0].url_type, UrlType::Shortlink);
    }

    #[test]
    fn test_parse_artist_url() {
        let results = parse_urls("https://play.nugs.net/artist/196");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "196");
        assert_eq!(results[0].url_type, UrlType::Artist);
    }

    #[test]
    fn test_parse_bare_id() {
        let results = parse_urls("12345");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "12345");
        assert_eq!(results[0].url_type, UrlType::Release);
    }

    #[test]
    fn test_parse_multiple_bare_ids() {
        let results = parse_urls("12345, 67890");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_parse_invalid_input() {
        let results = parse_urls("not a url");
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_parse_deduplicates() {
        let results = parse_urls("https://play.nugs.net/release/12345 12345");
        // URL match + bare ID, but same release ID — should dedup
        assert_eq!(results.len(), 1);
    }
}
