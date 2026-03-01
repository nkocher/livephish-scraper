/// 5-step metadata enrichment pipeline for Bman (Google Drive) shows.
///
/// Runs BEFORE `download_show()` so enriched titles are used for both
/// filenames and audio tags. Steps that need downloaded files (Vorbis
/// comments) are no-ops at this stage. Progressively enriches track
/// titles in `Show.tracks` from lowest to highest authority, never
/// overwriting a higher-ranked source.
use std::collections::HashMap;
use std::path::Path;

use lofty::prelude::*;
use lofty::probe::Probe;
use once_cell::sync::Lazy;
use regex::Regex;
use tracing::{debug, warn};

use crate::bman::setlistfm::{self, SetlistFmKeys, SfmCache, SfmStatus};
use crate::models::Show;

// ---- Title source ranking ---------------------------------------------------

/// Ranking of title sources (higher = more authoritative).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TitleSource {
    Fallback = 0,
    Filename = 1,
    InfoFile = 2,
    VorbisComment = 3,
    SetlistFm = 4,
}

// ---- Compiled regexes -------------------------------------------------------

/// Matches info-file track lines: `1. Title`, `01-Title`, `1) Title`, `01 Title`.
/// The alternation handles punctuation separators (dash/dot/paren) and space-only.
static INFO_TRACK_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\s*(\d+)(?:\s*[-.)]\s*|\s+)(\S.*)$").unwrap()
});

/// Matches info-file section headers to skip.
/// e.g. "Set 1:", "Encore:", "Disk 1:", "Disc 2:"
static SECTION_HEADER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*(set\s*\d*|encore|disk\s*\d*|disc\s*\d*)\s*:?\s*$").unwrap()
});

// ---- Public entry point -----------------------------------------------------

/// Enrich track titles in `show.tracks` using a 5-step pipeline.
///
/// Steps (in ascending authority):
/// 1. Filename-based titles — already set from parser (source=Filename), no-op
/// 2. Vorbis comments — reads TITLE from each `.flac` file
/// 3. Info file `.txt` — scans show_dir for a track listing
/// 4. setlist.fm — queries by date+artist if `sfm_keys` is non-empty
/// 5. Fallback — fills any still-empty titles with "Track 01", "Track 02", …
pub async fn resolve_metadata(
    show_dir: &Path,
    show: &mut Show,
    sfm_keys: &SetlistFmKeys,
    sfm_cache: &mut SfmCache,
) -> Result<(), String> {
    if show.tracks.is_empty() {
        return Ok(());
    }

    // Track current source ranking per track_id.
    // Tracks start at Filename (parser already populated song_title for "nice"
    // filenames; etree filenames leave title empty which is treated as Filename
    // source with an empty value — still overrideable by anything above).
    let mut sources: HashMap<i64, TitleSource> = show
        .tracks
        .iter()
        .map(|t| (t.track_id, TitleSource::Filename))
        .collect();

    // Step 2 — Vorbis comments
    apply_vorbis_comments(show_dir, show, &mut sources);

    // Step 3 — Info file
    apply_info_file(show_dir, show, &mut sources);

    // Step 4 — setlist.fm (async, graceful failure) — uses cache
    if !sfm_keys.is_empty() {
        apply_setlistfm(show, &mut sources, sfm_keys, sfm_cache).await;
    }

    // Step 5 — Fallback for any tracks still without a title
    apply_fallback(show, &mut sources);

    Ok(())
}

// ---- Shared helpers ---------------------------------------------------------

/// Return show track indices sorted by (disc_num, track_num).
fn sorted_track_indices(show: &Show) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..show.tracks.len()).collect();
    indices.sort_by_key(|&i| (show.tracks[i].disc_num, show.tracks[i].track_num));
    indices
}

// ---- Step 2: Vorbis comments ------------------------------------------------

fn apply_vorbis_comments(
    show_dir: &Path,
    show: &mut Show,
    sources: &mut HashMap<i64, TitleSource>,
) {
    // Align sorted FLAC files with sorted tracks by position. Track filenames
    // are not stored in Track, so we rely on sort order matching.
    let flac_files = collect_flac_files(show_dir);
    let sorted_indices = sorted_track_indices(show);

    for (file_pos, &track_idx) in sorted_indices.iter().enumerate() {
        let Some(path) = flac_files.get(file_pos) else {
            break;
        };

        let track_id = show.tracks[track_idx].track_id;
        let current_source = sources.get(&track_id).copied().unwrap_or(TitleSource::Filename);

        if current_source >= TitleSource::VorbisComment {
            continue;
        }

        if let Some(title) = read_vorbis_title(path).filter(|t| !t.is_empty()) {
            show.tracks[track_idx].song_title = title;
            sources.insert(track_id, TitleSource::VorbisComment);
        }
    }
}

/// Read the TITLE Vorbis comment from a FLAC file.
fn read_vorbis_title(path: &Path) -> Option<String> {
    let tagged_file = Probe::open(path).ok()?.read().ok()?;
    let tag = tagged_file.primary_tag().or_else(|| tagged_file.first_tag())?;
    tag.title().map(|s| s.into_owned())
}

fn is_flac_file(path: &std::path::Path) -> bool {
    path.is_file()
        && path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("flac"))
}

/// Collect sorted .flac file paths from a single directory.
fn collect_sorted_flacs(dir: &Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut flacs: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_flac_file(p))
        .collect();
    flacs.sort();
    flacs
}

/// Recursively collect .flac files from show_dir (including disc subdirs),
/// sorted by (relative_path, name) to match the order tracks were collected.
fn collect_flac_files(show_dir: &Path) -> Vec<std::path::PathBuf> {
    let top_flacs = collect_sorted_flacs(show_dir);
    if !top_flacs.is_empty() {
        return top_flacs;
    }

    // Multi-disc: gather from sorted subdirectories
    let Ok(entries) = std::fs::read_dir(show_dir) else {
        return Vec::new();
    };
    let mut subdirs: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    subdirs.sort();

    subdirs
        .iter()
        .flat_map(|dir| collect_sorted_flacs(dir))
        .collect()
}

// ---- Step 3: Info file ------------------------------------------------------

fn apply_info_file(
    show_dir: &Path,
    show: &mut Show,
    sources: &mut HashMap<i64, TitleSource>,
) {
    let Some(txt_path) = find_best_info_file(show_dir) else {
        return;
    };

    let Ok(contents) = std::fs::read_to_string(&txt_path) else {
        warn!("Could not read info file: {}", txt_path.display());
        return;
    };

    let track_titles = parse_info_file(&contents);
    if track_titles.is_empty() {
        return;
    }

    debug!(
        "Info file {} yielded {} track titles",
        txt_path.display(),
        track_titles.len()
    );

    let sorted_indices = sorted_track_indices(show);

    for (pos, &track_idx) in sorted_indices.iter().enumerate() {
        let Some(title) = track_titles.get(pos) else {
            break;
        };
        let track_id = show.tracks[track_idx].track_id;
        let current_source = sources.get(&track_id).copied().unwrap_or(TitleSource::Filename);

        if current_source >= TitleSource::InfoFile {
            continue;
        }

        if !title.is_empty() {
            show.tracks[track_idx].song_title = title.clone();
            sources.insert(track_id, TitleSource::InfoFile);
        }
    }
}

/// Find the best `.txt` file in show_dir. If multiple, prefer the one with
/// the most parseable track lines.
fn find_best_info_file(show_dir: &Path) -> Option<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(show_dir) else {
        return None;
    };

    let txt_files: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("txt"))
        })
        .collect();

    if txt_files.len() <= 1 {
        return txt_files.into_iter().next();
    }
    // Multiple candidates: pick the file with the most parseable track lines.
    txt_files
        .into_iter()
        .max_by_key(|p| {
            std::fs::read_to_string(p)
                .map(|c| count_track_lines(&c))
                .unwrap_or(0)
        })
}

/// Count lines that look like track entries (used for file selection heuristic).
fn count_track_lines(contents: &str) -> usize {
    contents
        .lines()
        .filter(|l| INFO_TRACK_RE.is_match(l))
        .count()
}

/// Parse an info file and return ordered track titles.
///
/// Accepts formats: "1. Title", "01 Title" (when followed by a paren-group),
/// "1) Title". Skips section headers ("Set 1:", "Encore:", "Disk 1:", …).
pub(crate) fn parse_info_file(contents: &str) -> Vec<String> {
    let mut tracks: Vec<(u64, String)> = Vec::new();

    for line in contents.lines() {
        // Skip section headers
        if SECTION_HEADER_RE.is_match(line) {
            continue;
        }

        if let Some(cap) = INFO_TRACK_RE.captures(line) {
            let num: u64 = cap[1].parse().unwrap_or(0);
            let title = cap[2].trim().to_string();
            if num > 0 && !title.is_empty() {
                tracks.push((num, title));
            }
        }
    }

    // Sort by track number and deduplicate (keep first occurrence of each num).
    tracks.sort_by_key(|(n, _)| *n);
    tracks.dedup_by_key(|(n, _)| *n);

    tracks.into_iter().map(|(_, t)| t).collect()
}

// ---- Step 4: setlist.fm -----------------------------------------------------

async fn apply_setlistfm(
    show: &mut Show,
    sources: &mut HashMap<i64, TitleSource>,
    sfm_keys: &SetlistFmKeys,
    sfm_cache: &mut SfmCache,
) {
    let date = show.performance_date.clone();
    let artist = show.artist_name.clone();

    if date.is_empty() || artist.is_empty() {
        return;
    }

    // Check cache first
    if let Some(status) = sfm_cache.lookup(&artist, &date) {
        match status {
            SfmStatus::Found { songs, .. } => {
                if !songs.is_empty() {
                    debug!(
                        "setlist.fm cache hit: {} songs for {} {}",
                        songs.len(), artist, date
                    );
                    let songs = songs.clone();
                    apply_titles_by_position(show, sources, &songs, TitleSource::SetlistFm);
                }
            }
            SfmStatus::NotFound => {
                debug!("setlist.fm cache: not found for {} on {}", artist, date);
            }
        }
        return;
    }

    // Cache miss — make API call
    let client = reqwest::Client::new();
    match setlistfm::fetch_setlist_full(&client, sfm_keys, &artist, &date).await {
        Ok(Some(result)) => {
            debug!(
                "setlist.fm returned {} songs for {} {}",
                result.songs.len(), artist, date
            );
            apply_titles_by_position(show, sources, &result.songs, TitleSource::SetlistFm);
            sfm_cache.insert(&artist, &date, SfmStatus::Found {
                venue: result.venue,
                songs: result.songs,
            });
        }
        Ok(None) => {
            debug!("setlist.fm: no results for {} on {}", artist, date);
            sfm_cache.insert(&artist, &date, SfmStatus::NotFound);
        }
        Err(e) => {
            warn!("setlist.fm fetch failed (non-fatal): {}", e);
            // Don't cache errors — they're transient
        }
    }
}

/// Apply a slice of titles to tracks by sequential position, sorted by
/// (disc_num, track_num). Higher-ranked sources are not overwritten.
fn apply_titles_by_position(
    show: &mut Show,
    sources: &mut HashMap<i64, TitleSource>,
    titles: &[String],
    source: TitleSource,
) {
    let sorted_indices = sorted_track_indices(show);

    for (pos, &track_idx) in sorted_indices.iter().enumerate() {
        let Some(title) = titles.get(pos) else {
            break;
        };
        let track_id = show.tracks[track_idx].track_id;
        let current_source = sources.get(&track_id).copied().unwrap_or(TitleSource::Filename);

        if current_source >= source {
            continue;
        }

        if !title.is_empty() {
            show.tracks[track_idx].song_title = title.clone();
            sources.insert(track_id, source);
        }
    }
}

// ---- Step 5: Fallback -------------------------------------------------------

fn apply_fallback(show: &mut Show, sources: &mut HashMap<i64, TitleSource>) {
    for track in &mut show.tracks {
        if track.song_title.is_empty() {
            track.song_title = format!("Track {:02}", track.track_num);
            sources.insert(track.track_id, TitleSource::Fallback);
        }
    }
}

// ---- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- TitleSource ordering -----------------------------------------------

    #[test]
    fn test_title_source_ordering() {
        assert!(TitleSource::Fallback < TitleSource::Filename);
        assert!(TitleSource::Filename < TitleSource::InfoFile);
        assert!(TitleSource::InfoFile < TitleSource::VorbisComment);
        assert!(TitleSource::VorbisComment < TitleSource::SetlistFm);
    }

    // ---- parse_info_file ----------------------------------------------------

    #[test]
    fn test_parse_info_file_dot_format() {
        let contents = "1. Scarlet Begonias\n2. Fire on the Mountain\n3. Eyes of the World";
        let titles = parse_info_file(contents);
        assert_eq!(titles, vec!["Scarlet Begonias", "Fire on the Mountain", "Eyes of the World"]);
    }

    #[test]
    fn test_parse_info_file_paren_format() {
        let contents = "1) Dark Star\n2) St. Stephen\n3) The Eleven";
        let titles = parse_info_file(contents);
        assert_eq!(titles, vec!["Dark Star", "St. Stephen", "The Eleven"]);
    }

    #[test]
    fn test_parse_info_file_leading_zeros() {
        let contents = "01. Song A\n02. Song B\n10. Song J";
        let titles = parse_info_file(contents);
        assert_eq!(titles.len(), 3);
        assert_eq!(titles[0], "Song A");
        assert_eq!(titles[1], "Song B");
        assert_eq!(titles[2], "Song J");
    }

    #[test]
    fn test_parse_info_file_skips_section_headers() {
        let contents = "Set 1:\n1. Song A\n2. Song B\nEncore:\n3. Song C";
        let titles = parse_info_file(contents);
        assert_eq!(titles, vec!["Song A", "Song B", "Song C"]);
    }

    #[test]
    fn test_parse_info_file_skips_disc_headers() {
        let contents = "Disk 1:\n1. Song A\nDisc 2:\n2. Song B";
        let titles = parse_info_file(contents);
        assert_eq!(titles, vec!["Song A", "Song B"]);
    }

    #[test]
    fn test_parse_info_file_ignores_blank_lines() {
        let contents = "\n1. Song A\n\n2. Song B\n";
        let titles = parse_info_file(contents);
        assert_eq!(titles, vec!["Song A", "Song B"]);
    }

    #[test]
    fn test_parse_info_file_ignores_non_track_lines() {
        let contents = "Grateful Dead Live\nDate: 1977-05-08\n1. Scarlet Begonias\nRecorded by: Bob";
        let titles = parse_info_file(contents);
        assert_eq!(titles, vec!["Scarlet Begonias"]);
    }

    #[test]
    fn test_parse_info_file_deduplicates_track_numbers() {
        // If track 1 appears twice, keep first occurrence
        let contents = "1. First Title\n1. Duplicate\n2. Second";
        let titles = parse_info_file(contents);
        assert_eq!(titles, vec!["First Title", "Second"]);
    }

    #[test]
    fn test_parse_info_file_empty() {
        let titles = parse_info_file("");
        assert!(titles.is_empty());
    }

    #[test]
    fn test_parse_info_file_only_headers() {
        let titles = parse_info_file("Set 1:\nEncore:\nDisc 2:");
        assert!(titles.is_empty());
    }

    #[test]
    fn test_parse_info_file_dash_separator() {
        let contents = "01-Song Title\n02-Another Song";
        let titles = parse_info_file(contents);
        assert_eq!(titles, vec!["Song Title", "Another Song"]);
    }

    #[test]
    fn test_parse_info_file_space_only_separator() {
        let contents = "01 Song Title\n02 Another Song";
        let titles = parse_info_file(contents);
        assert_eq!(titles, vec!["Song Title", "Another Song"]);
    }

    #[test]
    fn test_parse_info_file_dash_no_space() {
        let contents = "1-Dark Star\n2-St. Stephen";
        let titles = parse_info_file(contents);
        assert_eq!(titles, vec!["Dark Star", "St. Stephen"]);
    }

    // ---- count_track_lines --------------------------------------------------

    #[test]
    fn test_count_track_lines() {
        let contents = "Set 1:\n1. Song A\n2. Song B\nRecorded by: Bob";
        assert_eq!(count_track_lines(contents), 2);
    }

    // ---- apply_titles_by_position -------------------------------------------

    fn make_track(track_id: i64, track_num: i64, disc_num: i64, title: &str) -> crate::models::Track {
        crate::models::Track {
            track_id,
            song_id: 0,
            song_title: title.to_string(),
            track_num,
            disc_num,
            set_num: 1,
            duration_seconds: 0,
            duration_display: String::new(),
        }
    }

    fn make_show_with_tracks(tracks: Vec<crate::models::Track>) -> Show {
        Show {
            container_id: 1,
            artist_name: "Grateful Dead".to_string(),
            container_info: "1977-05-08 Cornell".to_string(),
            venue_name: "Barton Hall".to_string(),
            venue_city: "Ithaca".to_string(),
            venue_state: "NY".to_string(),
            performance_date: "1977-05-08".to_string(),
            performance_date_formatted: "05/08/1977".to_string(),
            performance_date_year: "1977".to_string(),
            artist_id: 461,
            total_duration_seconds: 0,
            total_duration_display: String::new(),
            tracks,
            image_url: String::new(),
        }
    }

    #[test]
    fn test_apply_titles_by_position_basic() {
        let tracks = vec![
            make_track(1, 1, 1, "Filename Title"),
            make_track(2, 2, 1, "Another Title"),
        ];
        let mut show = make_show_with_tracks(tracks);
        let mut sources: HashMap<i64, TitleSource> = HashMap::from([
            (1, TitleSource::Filename),
            (2, TitleSource::Filename),
        ]);

        let titles = vec!["Info Title 1".to_string(), "Info Title 2".to_string()];
        apply_titles_by_position(&mut show, &mut sources, &titles, TitleSource::InfoFile);

        assert_eq!(show.tracks[0].song_title, "Info Title 1");
        assert_eq!(show.tracks[1].song_title, "Info Title 2");
        assert_eq!(sources[&1], TitleSource::InfoFile);
        assert_eq!(sources[&2], TitleSource::InfoFile);
    }

    #[test]
    fn test_apply_titles_higher_source_not_overwritten() {
        let tracks = vec![
            make_track(1, 1, 1, "Vorbis Title"),
            make_track(2, 2, 1, "Filename Title"),
        ];
        let mut show = make_show_with_tracks(tracks);
        let mut sources: HashMap<i64, TitleSource> = HashMap::from([
            (1, TitleSource::VorbisComment),
            (2, TitleSource::Filename),
        ]);

        let titles = vec!["Info Title 1".to_string(), "Info Title 2".to_string()];
        apply_titles_by_position(&mut show, &mut sources, &titles, TitleSource::InfoFile);

        // Track 1 had VorbisComment > InfoFile, should not be overwritten
        assert_eq!(show.tracks[0].song_title, "Vorbis Title");
        // Track 2 had Filename < InfoFile, should be updated
        assert_eq!(show.tracks[1].song_title, "Info Title 2");
    }

    #[test]
    fn test_apply_titles_fewer_sources_than_tracks() {
        let tracks = vec![
            make_track(1, 1, 1, ""),
            make_track(2, 2, 1, ""),
            make_track(3, 3, 1, ""),
        ];
        let mut show = make_show_with_tracks(tracks);
        let mut sources: HashMap<i64, TitleSource> = HashMap::from([
            (1, TitleSource::Filename),
            (2, TitleSource::Filename),
            (3, TitleSource::Filename),
        ]);

        // Only 2 titles for 3 tracks
        let titles = vec!["Title 1".to_string(), "Title 2".to_string()];
        apply_titles_by_position(&mut show, &mut sources, &titles, TitleSource::InfoFile);

        assert_eq!(show.tracks[0].song_title, "Title 1");
        assert_eq!(show.tracks[1].song_title, "Title 2");
        assert_eq!(show.tracks[2].song_title, ""); // not updated
    }

    // ---- apply_fallback -----------------------------------------------------

    #[test]
    fn test_apply_fallback_fills_empty_titles() {
        let tracks = vec![
            make_track(1, 1, 1, ""),
            make_track(2, 2, 1, "Real Title"),
            make_track(3, 3, 1, ""),
        ];
        let mut show = make_show_with_tracks(tracks);
        let mut sources: HashMap<i64, TitleSource> = HashMap::from([
            (1, TitleSource::Filename),
            (2, TitleSource::InfoFile),
            (3, TitleSource::Filename),
        ]);

        apply_fallback(&mut show, &mut sources);

        assert_eq!(show.tracks[0].song_title, "Track 01");
        assert_eq!(show.tracks[1].song_title, "Real Title"); // unchanged
        assert_eq!(show.tracks[2].song_title, "Track 03");
    }

    #[test]
    fn test_apply_fallback_no_op_when_all_titled() {
        let tracks = vec![
            make_track(1, 1, 1, "Song A"),
            make_track(2, 2, 1, "Song B"),
        ];
        let mut show = make_show_with_tracks(tracks);
        let mut sources: HashMap<i64, TitleSource> = HashMap::from([
            (1, TitleSource::InfoFile),
            (2, TitleSource::InfoFile),
        ]);

        apply_fallback(&mut show, &mut sources);

        assert_eq!(show.tracks[0].song_title, "Song A");
        assert_eq!(show.tracks[1].song_title, "Song B");
    }
}
