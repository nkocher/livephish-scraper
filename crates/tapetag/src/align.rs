use crate::archive_org::ArchiveTrack;
use crate::scanner::LocalTrack;

/// Alignment result for a single track.
#[derive(Debug, Clone)]
pub struct TrackAlignment {
    pub local_idx: usize,
    pub archive_idx: Option<usize>,
    pub proposed_title: String,
    pub confidence: f64,
    #[allow(dead_code)]
    pub duration_delta_secs: f64,
    pub archive_duration: Option<f64>,
}

/// Align local tracks with archive.org tracks.
/// Uses positional matching with duration-based confidence scoring.
pub fn align_tracks(local: &[LocalTrack], archive: &[ArchiveTrack]) -> Vec<TrackAlignment> {
    let min_len = local.len().min(archive.len());
    let mut alignments = Vec::with_capacity(local.len());

    for (i, local_track) in local.iter().enumerate() {
        if i < min_len {
            let arch = &archive[i];
            let (confidence, delta) = duration_confidence(local_track.duration_secs, arch.length);
            alignments.push(TrackAlignment {
                local_idx: i,
                archive_idx: Some(i),
                proposed_title: arch.title.clone(),
                confidence,
                duration_delta_secs: delta,
                archive_duration: arch.length,
            });
        } else {
            // Extra local tracks with no archive match
            alignments.push(TrackAlignment {
                local_idx: i,
                archive_idx: None,
                proposed_title: String::new(),
                confidence: 0.0,
                duration_delta_secs: 0.0,
                archive_duration: None,
            });
        }
    }

    alignments
}

/// Compute overall show confidence as geometric mean of track confidences.
pub fn show_confidence(alignments: &[TrackAlignment]) -> f64 {
    if alignments.is_empty() {
        return 0.0;
    }

    let matched: Vec<f64> = alignments
        .iter()
        .filter(|a| a.archive_idx.is_some())
        .map(|a| a.confidence)
        .collect();

    if matched.is_empty() {
        return 0.0;
    }

    // Geometric mean: exp(mean(ln(x)))
    let log_sum: f64 = matched.iter().map(|c| c.max(0.001).ln()).sum();
    (log_sum / matched.len() as f64).exp()
}

fn duration_confidence(local_secs: f64, archive_secs: Option<f64>) -> (f64, f64) {
    let archive_secs = match archive_secs {
        Some(s) if s > 0.0 => s,
        _ => return (0.60, 0.0), // Unknown archive duration
    };

    if local_secs <= 0.0 {
        return (0.60, 0.0); // Unknown local duration
    }

    let delta = (local_secs - archive_secs).abs();
    let confidence = if delta < 3.0 {
        0.99
    } else if delta < 10.0 {
        0.90
    } else if delta < 20.0 {
        0.70
    } else if delta < 30.0 {
        0.50
    } else {
        0.20
    };

    (confidence, delta)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_local(duration: f64) -> LocalTrack {
        LocalTrack {
            path: std::path::PathBuf::from("test.flac"),
            title: "Track 01".to_string(),
            track_num: 1,
            disc_num: 1,
            duration_secs: duration,
        }
    }

    fn make_archive(title: &str, length: Option<f64>) -> ArchiveTrack {
        ArchiveTrack {
            name: "t01.flac".to_string(),
            title: title.to_string(),
            track: "1".to_string(),
            length,
            disc: 1,
        }
    }

    #[test]
    fn test_equal_counts_high_confidence() {
        let local = vec![make_local(324.0), make_local(180.0)];
        let archive = vec![
            make_archive("Scarlet Begonias", Some(325.0)),
            make_archive("Fire on the Mountain", Some(181.0)),
        ];

        let aligned = align_tracks(&local, &archive);
        assert_eq!(aligned.len(), 2);
        assert_eq!(aligned[0].proposed_title, "Scarlet Begonias");
        assert!(aligned[0].confidence > 0.95);
        assert_eq!(aligned[1].proposed_title, "Fire on the Mountain");
    }

    #[test]
    fn test_more_local_than_archive() {
        let local = vec![make_local(100.0), make_local(200.0), make_local(300.0)];
        let archive = vec![make_archive("Song 1", Some(100.0))];

        let aligned = align_tracks(&local, &archive);
        assert_eq!(aligned.len(), 3);
        assert!(aligned[0].archive_idx.is_some());
        assert!(aligned[1].archive_idx.is_none());
        assert!(aligned[2].archive_idx.is_none());
    }

    #[test]
    fn test_missing_archive_duration() {
        let local = vec![make_local(300.0)];
        let archive = vec![make_archive("Song 1", None)];

        let aligned = align_tracks(&local, &archive);
        assert_eq!(aligned[0].confidence, 0.60);
    }

    #[test]
    fn test_large_duration_delta() {
        let local = vec![make_local(300.0)];
        let archive = vec![make_archive("Song 1", Some(600.0))];

        let aligned = align_tracks(&local, &archive);
        assert!(aligned[0].confidence < 0.30);
    }

    #[test]
    fn test_show_confidence_all_high() {
        let alignments = vec![
            TrackAlignment {
                local_idx: 0,
                archive_idx: Some(0),
                proposed_title: "A".to_string(),
                confidence: 0.99,
                duration_delta_secs: 1.0,
                archive_duration: Some(100.0),
            },
            TrackAlignment {
                local_idx: 1,
                archive_idx: Some(1),
                proposed_title: "B".to_string(),
                confidence: 0.99,
                duration_delta_secs: 1.0,
                archive_duration: Some(200.0),
            },
        ];
        assert!(show_confidence(&alignments) > 0.95);
    }

    #[test]
    fn test_show_confidence_one_low() {
        let alignments = vec![
            TrackAlignment {
                local_idx: 0,
                archive_idx: Some(0),
                proposed_title: "A".to_string(),
                confidence: 0.99,
                duration_delta_secs: 1.0,
                archive_duration: Some(100.0),
            },
            TrackAlignment {
                local_idx: 1,
                archive_idx: Some(1),
                proposed_title: "B".to_string(),
                confidence: 0.20,
                duration_delta_secs: 100.0,
                archive_duration: Some(500.0),
            },
        ];
        let conf = show_confidence(&alignments);
        assert!(conf < 0.99 && conf > 0.10);
    }

    #[test]
    fn test_show_confidence_empty() {
        assert_eq!(show_confidence(&[]), 0.0);
    }

    #[test]
    fn test_show_confidence_no_matches() {
        let alignments = vec![TrackAlignment {
            local_idx: 0,
            archive_idx: None,
            proposed_title: String::new(),
            confidence: 0.0,
            duration_delta_secs: 0.0,
            archive_duration: None,
        }];
        assert_eq!(show_confidence(&alignments), 0.0);
    }
}
