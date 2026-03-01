use std::path::{Path, PathBuf};

use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use once_cell::sync::Lazy;
use regex::Regex;

use crate::parser;

/// A scanned show folder.
#[derive(Debug, Clone)]
pub struct LocalShow {
    pub path: PathBuf,
    pub date: String,
    pub artist: String,
    pub folder_name: String,
    pub tracks: Vec<LocalTrack>,
    pub needs_fixing: bool,
    pub has_cover: bool,
    pub disc_count: u32,
}

/// A scanned audio track.
#[derive(Debug, Clone)]
pub struct LocalTrack {
    pub path: PathBuf,
    pub title: String,
    pub track_num: u32,
    pub disc_num: u32,
    pub duration_secs: f64,
}

/// Scan a directory for show folders containing audio files.
pub fn scan_shows(dir: &Path) -> Vec<LocalShow> {
    let mut shows = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Cannot read directory {}: {}", dir.display(), e);
            return shows;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || path.is_symlink() {
            continue;
        }

        let folder_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        let info = match parser::parse_folder_name(&folder_name) {
            Some(i) => i,
            None => continue,
        };

        let (tracks, disc_count) = scan_tracks(&path);
        if tracks.is_empty() {
            continue;
        }

        let needs_fixing = tracks.iter().any(|t| is_bad_title(&t.title));
        let has_cover = find_cover_file(&path).is_some();

        shows.push(LocalShow {
            path,
            date: info.date,
            artist: info.artist,
            folder_name,
            tracks,
            needs_fixing,
            has_cover,
            disc_count,
        });
    }

    shows.sort_by(|a, b| a.date.cmp(&b.date));
    shows
}

/// Check if a title looks like it needs fixing.
fn is_bad_title(title: &str) -> bool {
    title.is_empty()
        || title.starts_with("Track ")
        || title.starts_with("track ")
        || title.starts_with("gd")
        || title.starts_with("jgb")
        || title.starts_with("jg")
}

/// Find a cover image file in a show directory.
pub fn find_cover_file(dir: &Path) -> Option<PathBuf> {
    let candidates = ["cover.jpg", "cover.jpeg", "cover.png", "folder.jpg", "folder.png"];
    for name in &candidates {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    // Case-insensitive fallback
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_lowercase();
            if (fname.starts_with("cover") || fname.starts_with("folder"))
                && (fname.ends_with(".jpg") || fname.ends_with(".jpeg") || fname.ends_with(".png"))
            {
                return Some(entry.path());
            }
        }
    }
    None
}

/// Scan audio tracks within a show directory (including disc subfolders).
fn scan_tracks(show_dir: &Path) -> (Vec<LocalTrack>, u32) {
    let mut tracks = Vec::new();
    let mut max_disc: u32 = 1;
    let mut has_disc_subfolders = false;

    // Check for disc subfolders first
    if let Ok(entries) = std::fs::read_dir(show_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if let Some(disc_num) = parse_disc_subfolder(&name) {
                has_disc_subfolders = true;
                max_disc = max_disc.max(disc_num);
                collect_audio_files(&path, disc_num, &mut tracks);
            }
        }
    }

    // Only collect from root if no disc subfolders found (avoids double-counting)
    if !has_disc_subfolders {
        collect_audio_files(show_dir, 1, &mut tracks);
    }

    tracks.sort_by(|a, b| (a.disc_num, a.track_num).cmp(&(b.disc_num, b.track_num)));
    (tracks, max_disc)
}

static DISC_SUBFOLDER_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^(?:disc|disk|cd|set|d)\s*(\d+)$").unwrap());

fn parse_disc_subfolder(name: &str) -> Option<u32> {
    DISC_SUBFOLDER_RE
        .captures(name)
        .and_then(|c| c[1].parse::<u32>().ok())
}

fn collect_audio_files(dir: &Path, disc_num: u32, tracks: &mut Vec<LocalTrack>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut seq: u32 = 0;
    let mut files: Vec<_> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            name.ends_with(".flac") || name.ends_with(".m4a")
        })
        .collect();

    files.sort_by_key(|a| a.file_name());

    for entry in files {
        seq += 1;
        let path = entry.path();
        match read_track_tags(&path, disc_num, seq) {
            Some(track) => tracks.push(track),
            None => {
                // Fallback: minimal info from filename
                tracks.push(LocalTrack {
                    path,
                    title: String::new(),
                    track_num: seq,
                    disc_num,
                    duration_secs: 0.0,
                });
            }
        }
    }
}

fn read_track_tags(path: &Path, disc_num: u32, fallback_num: u32) -> Option<LocalTrack> {
    let tagged_file = Probe::open(path).ok()?.read().ok()?;
    let tag = tagged_file.primary_tag().or_else(|| tagged_file.first_tag());

    let title = tag
        .and_then(|t| t.get_string(&ItemKey::TrackTitle).map(|s| s.to_string()))
        .unwrap_or_default();

    let track_num = tag
        .and_then(|t| t.get_string(&ItemKey::TrackNumber))
        .and_then(|s| s.split('/').next()?.parse::<u32>().ok())
        .unwrap_or(fallback_num);

    let disc = tag
        .and_then(|t| t.get_string(&ItemKey::DiscNumber))
        .and_then(|s| s.split('/').next()?.parse::<u32>().ok())
        .unwrap_or(disc_num);

    let duration_secs = tagged_file
        .properties()
        .duration()
        .as_secs_f64();

    Some(LocalTrack {
        path: path.to_path_buf(),
        title,
        track_num,
        disc_num: disc,
        duration_secs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_bad_title() {
        assert!(is_bad_title(""));
        assert!(is_bad_title("Track 01"));
        assert!(is_bad_title("track 05"));
        assert!(is_bad_title("gd77-04-23d1t01"));
        assert!(!is_bad_title("Scarlet Begonias"));
        assert!(!is_bad_title("Fire on the Mountain"));
    }

    #[test]
    fn test_parse_disc_subfolder() {
        assert_eq!(parse_disc_subfolder("disc 1"), Some(1));
        assert_eq!(parse_disc_subfolder("Disc2"), Some(2));
        assert_eq!(parse_disc_subfolder("CD 3"), Some(3));
        assert_eq!(parse_disc_subfolder("d1"), Some(1));
        assert_eq!(parse_disc_subfolder("set 2"), Some(2));
        assert_eq!(parse_disc_subfolder("random"), None);
    }

    #[test]
    fn test_find_cover_file_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(find_cover_file(tmp.path()).is_none());
    }

    #[test]
    fn test_find_cover_file_exists() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("cover.jpg"), b"fake").unwrap();
        assert!(find_cover_file(tmp.path()).is_some());
    }

    #[test]
    fn test_scan_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let shows = scan_shows(tmp.path());
        assert!(shows.is_empty());
    }
}
