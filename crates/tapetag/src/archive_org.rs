use std::path::{Path, PathBuf};
use std::time::Duration;

use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

/// An archive.org recording (search result).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveRecording {
    pub identifier: String,
    pub title: String,
    pub date: String,
    pub source: Option<String>,
    pub taper: Option<String>,
    pub venue: Option<String>,
    pub coverage: Option<String>,
}

/// A single track from an archive.org recording.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveTrack {
    pub name: String,
    pub title: String,
    pub track: String,
    pub length: Option<f64>,
    pub disc: u32,
}

/// Build a configured HTTP client for archive.org.
pub fn build_client() -> Client {
    Client::builder()
        .user_agent("tapetag/0.1")
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to build HTTP client")
}

/// Search archive.org for recordings of an artist on a given date.
pub fn search_recordings(
    client: &Client,
    date: &str,
    artist: &str,
    cache_dir: &Path,
) -> anyhow::Result<Vec<ArchiveRecording>> {
    // Check cache first
    let cache_key = format!("search_{}_{}", artist_collection(artist), date);
    if let Some(cached) = load_cache::<Vec<ArchiveRecording>>(cache_dir, &cache_key) {
        return Ok(cached);
    }

    let collection = artist_collection(artist);
    let query = format!(
        "collection:{} AND date:{} AND mediatype:audio",
        collection, date
    );

    let url = "https://archive.org/advancedsearch.php";
    let query_params = [
        ("q", query.as_str()),
        ("fl[]", "identifier,title,date,source,taper,venue,coverage"),
        ("rows", "50"),
        ("output", "json"),
    ];

    let resp = match client.get(url).query(&query_params).send() {
        Ok(r) => r,
        Err(e) if e.status().is_some_and(|s| s.is_server_error()) => {
            // Retry once on 5xx
            std::thread::sleep(Duration::from_secs(2));
            client.get(url).query(&query_params).send()?
        }
        Err(e) => return Err(e.into()),
    };

    let body: SearchResponse = resp.json()?;
    let recordings: Vec<ArchiveRecording> = body
        .response
        .docs
        .into_iter()
        .map(|doc| ArchiveRecording {
            identifier: doc.identifier,
            title: doc.title.unwrap_or_default(),
            date: doc.date.unwrap_or_default(),
            source: doc.source,
            taper: doc.taper,
            venue: doc.venue,
            coverage: doc.coverage,
        })
        .collect();

    save_cache(cache_dir, &cache_key, &recordings);
    Ok(recordings)
}

/// Fetch tracks for a specific archive.org recording.
pub fn fetch_recording_tracks(
    client: &Client,
    identifier: &str,
    cache_dir: &Path,
) -> anyhow::Result<Vec<ArchiveTrack>> {
    let cache_key = format!("tracks_{}", identifier);
    if let Some(cached) = load_cache::<Vec<ArchiveTrack>>(cache_dir, &cache_key) {
        return Ok(cached);
    }

    let url = format!("https://archive.org/metadata/{}/files", identifier);
    let resp = client.get(&url).send()?;
    let body: FilesResponse = resp.json()?;

    let mut tracks: Vec<ArchiveTrack> = body
        .result
        .into_iter()
        .filter(|f| is_audio_file(&f.name, f.source.as_deref()))
        .map(|f| {
            let disc = DISC_RE
                .captures(&f.name)
                .and_then(|c| c[1].parse().ok())
                .unwrap_or(1);

            let length = f.length.as_deref().and_then(parse_length);

            let title = sanitize_title(
                f.title
                    .unwrap_or_else(|| stem_from_filename(&f.name)),
            );

            let track = f.track.unwrap_or_default();

            ArchiveTrack {
                name: f.name,
                title,
                track,
                length,
                disc,
            }
        })
        .collect();

    // Sort by disc then track number
    tracks.sort_by(|a, b| {
        let ta = parse_track_num(&a.track);
        let tb = parse_track_num(&b.track);
        (a.disc, ta).cmp(&(b.disc, tb))
    });

    save_cache(cache_dir, &cache_key, &tracks);
    Ok(tracks)
}

/// Score a candidate recording against local tracks (0.0–1.0).
/// Higher = better match. Based on track count similarity and total duration proximity.
pub fn score_candidate(local_count: usize, local_duration: f64, tracks: &[ArchiveTrack]) -> f64 {
    let archive_count = tracks.len();
    if archive_count == 0 {
        return 0.0;
    }

    // Track count similarity (0.0–1.0)
    let count_ratio = local_count.min(archive_count) as f64 / local_count.max(archive_count) as f64;

    // Duration similarity if available
    let archive_duration: f64 = tracks.iter().filter_map(|t| t.length).sum();
    let dur_score = if archive_duration > 0.0 && local_duration > 0.0 {
        let delta = (local_duration - archive_duration).abs();
        if delta < 60.0 {
            1.0
        } else if delta < 300.0 {
            0.8
        } else if delta < 600.0 {
            0.5
        } else {
            0.2
        }
    } else {
        0.5 // Unknown duration
    };

    // Weighted average: count matters more
    count_ratio * 0.6 + dur_score * 0.4
}

// ---- Compiled regexes -------------------------------------------------------

static DISC_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)[sd](\d+)t").unwrap());

static LENGTH_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d+):(\d+)(?:\.(\d+))?$").unwrap());

// ---- Internals --------------------------------------------------------------

fn artist_collection(artist: &str) -> &str {
    if artist.contains("Garcia") || artist.contains("JGB") || artist.contains("jgb") {
        "JerryGarcia"
    } else {
        "GratefulDead"
    }
}

fn is_audio_file(name: &str, source: Option<&str>) -> bool {
    let lower = name.to_lowercase();
    let is_audio = lower.ends_with(".flac")
        || lower.ends_with(".shn")
        || lower.ends_with(".mp3")
        || lower.ends_with(".ogg");

    if !is_audio {
        return false;
    }

    // Prefer original source files; include all if no "original" found
    match source {
        Some(s) => s == "original",
        None => true,
    }
}

fn parse_length(s: &str) -> Option<f64> {
    // Try MM:SS or MM:SS.ms format first
    if let Some(caps) = LENGTH_RE.captures(s) {
        let mins: f64 = caps[1].parse().ok()?;
        let secs: f64 = caps[2].parse().ok()?;
        return Some(mins * 60.0 + secs);
    }
    // Try plain seconds
    s.parse::<f64>().ok()
}

fn parse_track_num(track: &str) -> u32 {
    track
        .split('/')
        .next()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn stem_from_filename(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_string()
}

fn sanitize_title(title: String) -> String {
    let cleaned: String = title.chars().filter(|c| !c.is_control()).collect();
    if cleaned.len() > 200 {
        cleaned[..200].to_string()
    } else {
        cleaned
    }
}

// ---- Cache ------------------------------------------------------------------

fn cache_dir_for(base: &Path) -> PathBuf {
    let dir = base.join("tapetag").join("cache");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn load_cache<T: serde::de::DeserializeOwned>(cache_dir: &Path, key: &str) -> Option<T> {
    let path = cache_dir_for(cache_dir).join(format!("{}.json", sanitize_cache_key(key)));
    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

fn save_cache<T: serde::Serialize>(cache_dir: &Path, key: &str, value: &T) {
    let dir = cache_dir_for(cache_dir);
    let path = dir.join(format!("{}.json", sanitize_cache_key(key)));
    if let Ok(json) = serde_json::to_string_pretty(value) {
        let _ = std::fs::write(path, json);
    }
}

fn sanitize_cache_key(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

// ---- API response types -----------------------------------------------------

#[derive(Deserialize)]
struct SearchResponse {
    response: SearchResponseInner,
}

#[derive(Deserialize)]
struct SearchResponseInner {
    docs: Vec<SearchDoc>,
}

#[derive(Deserialize)]
struct SearchDoc {
    identifier: String,
    title: Option<String>,
    date: Option<String>,
    source: Option<String>,
    taper: Option<String>,
    venue: Option<String>,
    coverage: Option<String>,
}

#[derive(Deserialize)]
struct FilesResponse {
    result: Vec<FileDoc>,
}

#[derive(Deserialize)]
struct FileDoc {
    name: String,
    source: Option<String>,
    title: Option<String>,
    track: Option<String>,
    length: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_artist_collection() {
        assert_eq!(artist_collection("Grateful Dead"), "GratefulDead");
        assert_eq!(artist_collection("Jerry Garcia Band"), "JerryGarcia");
    }

    #[test]
    fn test_is_audio_file() {
        assert!(is_audio_file("gd77-05-08d1t01.flac", Some("original")));
        assert!(is_audio_file("track01.shn", None));
        assert!(!is_audio_file("cover.jpg", None));
        assert!(!is_audio_file("track01.flac", Some("derivative")));
    }

    #[test]
    fn test_parse_length() {
        assert_eq!(parse_length("5:24"), Some(324.0));
        assert_eq!(parse_length("0:39"), Some(39.0));
        assert_eq!(parse_length("324.5"), Some(324.5));
    }

    #[test]
    fn test_parse_track_num() {
        assert_eq!(parse_track_num("01"), 1);
        assert_eq!(parse_track_num("02/23"), 2);
        assert_eq!(parse_track_num(""), 0);
    }

    #[test]
    fn test_sanitize_title() {
        assert_eq!(sanitize_title("Scarlet Begonias".to_string()), "Scarlet Begonias");
        assert_eq!(sanitize_title("Bad\x00Title".to_string()), "BadTitle");
    }

    #[test]
    fn test_score_candidate_equal_counts() {
        let tracks = vec![
            ArchiveTrack {
                name: "t01.flac".to_string(),
                title: "Song 1".to_string(),
                track: "1".to_string(),
                length: Some(300.0),
                disc: 1,
            },
            ArchiveTrack {
                name: "t02.flac".to_string(),
                title: "Song 2".to_string(),
                track: "2".to_string(),
                length: Some(300.0),
                disc: 1,
            },
        ];
        let score = score_candidate(2, 600.0, &tracks);
        assert!(score > 0.9, "Expected high score, got {}", score);
    }

    #[test]
    fn test_score_candidate_different_counts() {
        let tracks = vec![ArchiveTrack {
            name: "t01.flac".to_string(),
            title: "Song 1".to_string(),
            track: "1".to_string(),
            length: Some(300.0),
            disc: 1,
        }];
        let score = score_candidate(10, 3000.0, &tracks);
        assert!(score < 0.5, "Expected low score, got {}", score);
    }

    #[test]
    fn test_cache_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let data = vec!["hello".to_string(), "world".to_string()];
        save_cache(tmp.path(), "test_key", &data);
        let loaded: Option<Vec<String>> = load_cache(tmp.path(), "test_key");
        assert_eq!(loaded.unwrap(), data);
    }

    #[test]
    fn test_sanitize_cache_key() {
        assert_eq!(sanitize_cache_key("search_GratefulDead_1977-05-08"), "search_GratefulDead_1977-05-08");
        assert_eq!(sanitize_cache_key("has spaces!"), "has_spaces_");
    }
}
