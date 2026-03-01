use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Top-level manifest stored per music directory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub shows: BTreeMap<String, ShowRecord>,
}

/// Record of fixes applied to a single show.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowRecord {
    pub folder: String,
    pub archive_match: Option<ArchiveMatch>,
    pub tracks: Vec<TrackRecord>,
    pub artwork: Option<ArtworkRecord>,
    pub albumsort_applied: bool,
    pub updated_at: String,
}

/// Which archive.org recording was matched.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveMatch {
    pub identifier: String,
    pub overall_confidence: f64,
    pub match_method: String, // "auto" / "manual"
}

/// Per-track fix record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackRecord {
    pub file: String,
    pub original_title: String,
    pub applied_title: String,
    pub archive_title: String,
    pub confidence: f64,
    pub duration_local: f64,
    pub duration_archive: Option<f64>,
    pub status: TrackStatus,
}

/// Status of a track fix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TrackStatus {
    Applied,
    Skipped,
    Manual,
    Unmatched,
}

/// Record of artwork embedding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtworkRecord {
    pub source_file: String,
    pub embedded_at: String,
    pub file_count: u32,
}

const MANIFEST_FILE: &str = "tapetag_manifest.json";

impl Manifest {
    /// Load manifest from a directory. Returns empty manifest if not found.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(MANIFEST_FILE);
        match std::fs::read_to_string(&path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save manifest to a directory.
    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        let path = dir.join(MANIFEST_FILE);
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Check if a show has all tracks applied (fully fixed).
    pub fn is_fully_fixed(&self, date: &str) -> bool {
        match self.shows.get(date) {
            Some(record) => {
                !record.tracks.is_empty()
                    && record.tracks.iter().all(|t| t.status == TrackStatus::Applied)
            }
            None => false,
        }
    }

    /// Get count of shows that still need fixing.
    #[allow(dead_code)]
    pub fn unfixed_count(&self, dates_needing_fixes: &[&str]) -> usize {
        dates_needing_fixes
            .iter()
            .filter(|d| !self.is_fully_fixed(d))
            .count()
    }

    /// Record a show fix.
    pub fn record_show(
        &mut self,
        date: String,
        folder: String,
        archive_match: Option<ArchiveMatch>,
        tracks: Vec<TrackRecord>,
    ) {
        let record = ShowRecord {
            folder,
            archive_match,
            tracks,
            artwork: None,
            albumsort_applied: false,
            updated_at: now_iso(),
        };
        self.shows.insert(date, record);
    }

    /// Record artwork embedding for a show.
    pub fn record_artwork(&mut self, date: &str, source_file: String, file_count: u32) {
        if let Some(record) = self.shows.get_mut(date) {
            record.artwork = Some(ArtworkRecord {
                source_file,
                embedded_at: now_iso(),
                file_count,
            });
        }
    }

    /// Mark album sort tag as applied for a show.
    pub fn mark_albumsort(&mut self, date: &str) {
        if let Some(record) = self.shows.get_mut(date) {
            record.albumsort_applied = true;
            record.updated_at = now_iso();
        }
    }
}

fn now_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_save_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut manifest = Manifest::default();
        manifest.record_show(
            "1977-05-08".to_string(),
            "1977-05-08 Barton Hall".to_string(),
            Some(ArchiveMatch {
                identifier: "gd77-05-08.sbd.hicks.4982.sbeok.shnf".to_string(),
                overall_confidence: 0.95,
                match_method: "auto".to_string(),
            }),
            vec![TrackRecord {
                file: "01. Minglewood Blues.flac".to_string(),
                original_title: "Track 01".to_string(),
                applied_title: "Minglewood Blues".to_string(),
                archive_title: "Minglewood Blues".to_string(),
                confidence: 0.99,
                duration_local: 324.0,
                duration_archive: Some(325.0),
                status: TrackStatus::Applied,
            }],
        );

        manifest.save(tmp.path()).unwrap();
        let loaded = Manifest::load(tmp.path());
        assert!(loaded.shows.contains_key("1977-05-08"));
        assert!(loaded.is_fully_fixed("1977-05-08"));
    }

    #[test]
    fn test_is_fully_fixed() {
        let mut manifest = Manifest::default();
        assert!(!manifest.is_fully_fixed("1977-05-08"));

        manifest.record_show(
            "1977-05-08".to_string(),
            "test".to_string(),
            None,
            vec![
                TrackRecord {
                    file: "01.flac".to_string(),
                    original_title: "Track 01".to_string(),
                    applied_title: "Song".to_string(),
                    archive_title: "Song".to_string(),
                    confidence: 0.99,
                    duration_local: 100.0,
                    duration_archive: Some(100.0),
                    status: TrackStatus::Applied,
                },
                TrackRecord {
                    file: "02.flac".to_string(),
                    original_title: "Track 02".to_string(),
                    applied_title: "".to_string(),
                    archive_title: "Song 2".to_string(),
                    confidence: 0.5,
                    duration_local: 200.0,
                    duration_archive: None,
                    status: TrackStatus::Skipped,
                },
            ],
        );
        assert!(!manifest.is_fully_fixed("1977-05-08"));
    }

    #[test]
    fn test_unfixed_count() {
        let mut manifest = Manifest::default();
        manifest.record_show(
            "1977-05-08".to_string(),
            "test".to_string(),
            None,
            vec![TrackRecord {
                file: "01.flac".to_string(),
                original_title: "".to_string(),
                applied_title: "Song".to_string(),
                archive_title: "Song".to_string(),
                confidence: 0.99,
                duration_local: 100.0,
                duration_archive: Some(100.0),
                status: TrackStatus::Applied,
            }],
        );

        let dates = vec!["1977-05-08", "1978-01-01"];
        assert_eq!(manifest.unfixed_count(&dates), 1); // 1978-01-01 not fixed
    }

    #[test]
    fn test_load_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = Manifest::load(tmp.path());
        assert!(manifest.shows.is_empty());
    }

    #[test]
    fn test_record_artwork() {
        let mut manifest = Manifest::default();
        manifest.record_show("1977-05-08".to_string(), "test".to_string(), None, vec![]);
        manifest.record_artwork("1977-05-08", "cover.jpg".to_string(), 15);
        let record = &manifest.shows["1977-05-08"];
        assert_eq!(record.artwork.as_ref().unwrap().file_count, 15);
    }

    #[test]
    fn test_mark_albumsort() {
        let mut manifest = Manifest::default();
        manifest.record_show("1977-05-08".to_string(), "test".to_string(), None, vec![]);
        manifest.mark_albumsort("1977-05-08");
        assert!(manifest.shows["1977-05-08"].albumsort_applied);
    }
}
