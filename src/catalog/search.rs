use std::collections::HashMap;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use once_cell::sync::Lazy;

use crate::models::CatalogShow;

/// US states for search corpus expansion (state code → full name).
pub static US_STATES: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    HashMap::from([
        ("AL", "Alabama"),
        ("AK", "Alaska"),
        ("AZ", "Arizona"),
        ("AR", "Arkansas"),
        ("CA", "California"),
        ("CO", "Colorado"),
        ("CT", "Connecticut"),
        ("DE", "Delaware"),
        ("FL", "Florida"),
        ("GA", "Georgia"),
        ("HI", "Hawaii"),
        ("ID", "Idaho"),
        ("IL", "Illinois"),
        ("IN", "Indiana"),
        ("IA", "Iowa"),
        ("KS", "Kansas"),
        ("KY", "Kentucky"),
        ("LA", "Louisiana"),
        ("ME", "Maine"),
        ("MD", "Maryland"),
        ("MA", "Massachusetts"),
        ("MI", "Michigan"),
        ("MN", "Minnesota"),
        ("MS", "Mississippi"),
        ("MO", "Missouri"),
        ("MT", "Montana"),
        ("NE", "Nebraska"),
        ("NV", "Nevada"),
        ("NH", "New Hampshire"),
        ("NJ", "New Jersey"),
        ("NM", "New Mexico"),
        ("NY", "New York"),
        ("NC", "North Carolina"),
        ("ND", "North Dakota"),
        ("OH", "Ohio"),
        ("OK", "Oklahoma"),
        ("OR", "Oregon"),
        ("PA", "Pennsylvania"),
        ("RI", "Rhode Island"),
        ("SC", "South Carolina"),
        ("SD", "South Dakota"),
        ("TN", "Tennessee"),
        ("TX", "Texas"),
        ("UT", "Utah"),
        ("VT", "Vermont"),
        ("VA", "Virginia"),
        ("WA", "Washington"),
        ("WV", "West Virginia"),
        ("WI", "Wisconsin"),
        ("WY", "Wyoming"),
        ("DC", "District of Columbia"),
    ])
});

/// Generate abbreviation from first letters of multi-word venue names.
///
/// "Madison Square Garden" -> "MSG", "Red Rocks" -> "RR".
/// Only generates abbreviation for 2+ word names.
pub fn abbreviate(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    if words.len() < 2 {
        return String::new();
    }
    words.iter().filter_map(|w| w.chars().next()).collect()
}

/// Build search corpus text for a single show.
///
/// Includes artist name, venue name + abbreviation, city, state code + full name,
/// dates, container info, and song list — all lowercased for matching.
pub fn build_corpus_entry(show: &CatalogShow) -> String {
    let state_full = if !show.venue_state.is_empty() {
        US_STATES
            .get(show.venue_state.to_uppercase().as_str())
            .copied()
            .unwrap_or("")
    } else {
        ""
    };
    let venue_abbrev = abbreviate(&show.venue_name);

    [
        show.artist_name.as_str(),
        show.venue_name.as_str(),
        venue_abbrev.as_str(),
        show.venue_city.as_str(),
        show.venue_state.as_str(),
        state_full,
        show.performance_date.as_str(),
        show.performance_date_formatted.as_str(),
        show.container_info.as_str(),
        show.song_list.as_str(),
    ]
    .iter()
    .filter(|s| !s.is_empty())
    .copied()
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase()
}

/// Fuzzy search shows using nucleo-matcher.
///
/// Uses `Pattern::parse` which splits multi-word queries into atoms (AND logic).
/// Returns matching shows sorted by score (highest first), limited to `limit`.
pub fn search_shows(
    query: &str,
    corpus: &[(i64, String)],
    show_lookup: &HashMap<i64, usize>,
    shows: &[CatalogShow],
    limit: usize,
) -> Vec<CatalogShow> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }

    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);

    let mut scored: Vec<(i64, u32)> = Vec::new();
    let mut char_buf: Vec<char> = Vec::new();

    for (container_id, text) in corpus {
        let haystack = if text.is_ascii() {
            Utf32Str::Ascii(text.as_bytes())
        } else {
            char_buf.clear();
            char_buf.extend(text.chars());
            Utf32Str::Unicode(&char_buf)
        };

        if let Some(score) = pattern.score(haystack, &mut matcher) {
            scored.push((*container_id, score));
        }
    }

    scored.sort_by(|a, b| b.1.cmp(&a.1));
    scored.truncate(limit);

    scored
        .iter()
        .filter_map(|(cid, _)| show_lookup.get(cid).map(|&idx| shows[idx].clone()))
        .collect()
}

/// Fuzzy search within a single artist's shows (by artist_id).
pub fn search_artist_shows(
    query: &str,
    artist_id: i64,
    corpus: &[(i64, String)],
    show_lookup: &HashMap<i64, usize>,
    shows: &[CatalogShow],
    limit: usize,
) -> Vec<CatalogShow> {
    // Filter corpus to just this artist's shows
    let artist_corpus: Vec<(i64, String)> = corpus
        .iter()
        .filter(|(cid, _)| {
            show_lookup
                .get(cid)
                .map(|&idx| shows[idx].artist_id == artist_id)
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    search_shows(query, &artist_corpus, show_lookup, shows, limit)
}
