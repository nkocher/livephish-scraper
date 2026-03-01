use std::collections::BTreeMap;
use std::path::Path;

use inquire::{InquireError, Select};

use crate::align;
use crate::archive_org::{self, ArchiveRecording, ArchiveTrack};
use crate::artwork;
use crate::manifest::{ArchiveMatch, Manifest, TrackRecord, TrackStatus};
use crate::scanner::{self, LocalShow};
use crate::tagger;

const AUTO_CONFIDENCE_THRESHOLD: f64 = 0.80;

/// Run the interactive TUI browser.
pub fn run(dir: &Path) -> anyhow::Result<()> {
    println!("Scanning {}...", dir.display());
    let shows = scanner::scan_shows(dir);
    if shows.is_empty() {
        println!("No show folders found in {}", dir.display());
        return Ok(());
    }

    let mut manifest = Manifest::load(dir);
    let client = archive_org::build_client();
    let cache_dir = cache_base_dir();

    println!(
        "Found {} shows ({} needing fixes, {} with cover art)",
        shows.len(),
        shows.iter().filter(|s| s.needs_fixing && !manifest.is_fully_fixed(&s.date)).count(),
        shows.iter().filter(|s| s.has_cover).count(),
    );

    loop {
        let needs_fixing: Vec<&LocalShow> = shows
            .iter()
            .filter(|s| s.needs_fixing && !manifest.is_fully_fixed(&s.date))
            .collect();

        let options = build_main_menu(&shows, &needs_fixing);
        let choice = match Select::new("tapetag", options).prompt() {
            Ok(c) => c,
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => break,
            Err(e) => return Err(e.into()),
        };

        match choice.as_str() {
            s if s.starts_with("Shows needing fixes") => {
                browse_needs_fixing(&needs_fixing, &client, &cache_dir, &mut manifest, dir)?;
            }
            s if s.starts_with("Browse all") => {
                browse_by_year(&shows, &client, &cache_dir, &mut manifest, dir)?;
            }
            s if s.starts_with("Batch: fix date sort") => {
                batch_albumsort(&shows, &mut manifest, dir)?;
            }
            s if s.starts_with("Batch: re-embed artwork") => {
                batch_artwork(&shows, &mut manifest, dir)?;
            }
            s if s.starts_with("Batch: fix disc totals") => {
                batch_disc_totals(&shows)?;
            }
            "Quit" => break,
            _ => {}
        }
    }

    Ok(())
}

fn build_main_menu(shows: &[LocalShow], needs_fixing: &[&LocalShow]) -> Vec<String> {
    let mut options = Vec::new();
    options.push(format!(
        "Shows needing fixes ({} shows)",
        needs_fixing.len()
    ));
    options.push(format!("Browse all shows ({} total)", shows.len()));
    options.push("Batch: fix date sort tags (ALBUMSORT)".to_string());
    let artwork_count = shows.iter().filter(|s| s.has_cover).count();
    options.push(format!(
        "Batch: re-embed artwork ({} with covers)",
        artwork_count
    ));
    options.push(format!(
        "Batch: fix disc totals ({} multi-disc)",
        shows.iter().filter(|s| s.disc_count > 1).count()
    ));
    options.push("Quit".to_string());
    options
}

fn browse_needs_fixing(
    shows: &[&LocalShow],
    client: &reqwest::blocking::Client,
    cache_dir: &Path,
    manifest: &mut Manifest,
    root_dir: &Path,
) -> anyhow::Result<()> {
    if shows.is_empty() {
        println!("No shows need fixing!");
        return Ok(());
    }

    let options: Vec<String> = shows
        .iter()
        .map(|s| format!("{} {} ({} tracks)", s.date, s.artist, s.tracks.len()))
        .collect();

    let idx = match Select::new("Select show", options).prompt() {
        Ok(choice) => shows
            .iter()
            .position(|s| {
                format!("{} {} ({} tracks)", s.date, s.artist, s.tracks.len()) == choice
            })
            .unwrap_or(0),
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => return Ok(()),
        Err(e) => return Err(e.into()),
    };

    fix_show(shows[idx], client, cache_dir, manifest, root_dir)?;

    // "Apply + Next" loop: after fixing, offer to continue to next show
    let remaining: Vec<&&LocalShow> = shows
        .iter()
        .filter(|s| s.needs_fixing && !manifest.is_fully_fixed(&s.date))
        .collect();

    if !remaining.is_empty() {
        for show in remaining.iter() {
            let opts = vec![
                "Apply + Next".to_string(),
                "Back to menu".to_string(),
            ];
            match Select::new(
                &format!("Next: {} ({} remaining)", show.date, remaining.len()),
                opts,
            )
            .prompt()
            {
                Ok(c) if c == "Apply + Next" => {
                    fix_show(show, client, cache_dir, manifest, root_dir)?;
                }
                _ => break,
            }
        }
    }

    Ok(())
}

fn browse_by_year(
    shows: &[LocalShow],
    client: &reqwest::blocking::Client,
    cache_dir: &Path,
    manifest: &mut Manifest,
    root_dir: &Path,
) -> anyhow::Result<()> {
    // Group by year
    let mut by_year: BTreeMap<String, Vec<&LocalShow>> = BTreeMap::new();
    for show in shows {
        let year = show.date.get(..4).unwrap_or("????").to_string();
        by_year.entry(year).or_default().push(show);
    }

    let year_options: Vec<String> = by_year
        .iter()
        .map(|(year, shows)| format!("{} ({} shows)", year, shows.len()))
        .collect();

    let year_choice = match Select::new("Select year", year_options).prompt() {
        Ok(c) => c,
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => return Ok(()),
        Err(e) => return Err(e.into()),
    };

    let year = &year_choice[..4];
    let year_shows = &by_year[year];

    let show_options: Vec<String> = year_shows
        .iter()
        .map(|s| {
            let status = if manifest.is_fully_fixed(&s.date) {
                " [fixed]"
            } else if s.needs_fixing {
                " [needs fix]"
            } else {
                ""
            };
            format!("{} {}{}", s.date, s.folder_name, status)
        })
        .collect();

    let show_choice = match Select::new("Select show", show_options).prompt() {
        Ok(c) => c,
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => return Ok(()),
        Err(e) => return Err(e.into()),
    };

    let show_idx = year_shows
        .iter()
        .position(|s| show_choice.starts_with(&s.date))
        .unwrap_or(0);

    fix_show(year_shows[show_idx], client, cache_dir, manifest, root_dir)?;
    Ok(())
}

fn fix_show(
    show: &LocalShow,
    client: &reqwest::blocking::Client,
    cache_dir: &Path,
    manifest: &mut Manifest,
    root_dir: &Path,
) -> anyhow::Result<()> {
    println!("\nSearching archive.org for {} {}...", show.artist, show.date);

    let recordings = archive_org::search_recordings(client, &show.date, &show.artist, cache_dir)?;
    if recordings.is_empty() {
        println!("No archive.org recordings found for {}", show.date);
        return Ok(());
    }

    let (recording, tracks) = select_recording(show, &recordings, client, cache_dir)?;
    let recording = match recording {
        Some(r) => r,
        None => return Ok(()),
    };
    let tracks = tracks.unwrap_or_default();

    if tracks.is_empty() {
        println!("No audio tracks found in {}", recording.identifier);
        return Ok(());
    }

    let alignments = align::align_tracks(&show.tracks, &tracks);
    let overall = align::show_confidence(&alignments);

    print_alignment_table(show, &alignments);
    println!(
        "\nOverall confidence: {:.0}% ({})",
        overall * 100.0,
        recording.identifier
    );

    let action = select_action(overall)?;
    match action.as_str() {
        "Apply all" | "Apply + Next" => {
            apply_fixes(show, &alignments, recording, overall, manifest, root_dir)?;
        }
        "Skip" => {
            println!("Skipped {}", show.date);
        }
        _ => {}
    }

    Ok(())
}

fn select_recording<'a>(
    show: &LocalShow,
    recordings: &'a [ArchiveRecording],
    client: &reqwest::blocking::Client,
    cache_dir: &Path,
) -> anyhow::Result<(Option<&'a ArchiveRecording>, Option<Vec<ArchiveTrack>>)> {
    let local_duration: f64 = show.tracks.iter().map(|t| t.duration_secs).sum();

    // Score all candidates
    let mut scored: Vec<(usize, f64, Vec<ArchiveTrack>)> = Vec::new();
    for (i, rec) in recordings.iter().enumerate() {
        match archive_org::fetch_recording_tracks(client, &rec.identifier, cache_dir) {
            Ok(tracks) => {
                let score = archive_org::score_candidate(show.tracks.len(), local_duration, &tracks);
                scored.push((i, score, tracks));
            }
            Err(e) => {
                eprintln!("  Warning: failed to fetch tracks for {}: {}", rec.identifier, e);
            }
        }
    }

    if scored.is_empty() {
        return Ok((None, None));
    }

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let best_idx = scored[0].0;
    let best_score = scored[0].1;
    let second_score = scored.get(1).map(|s| s.1).unwrap_or(0.0);

    // Auto-select if top candidate is clearly the best
    if best_score > AUTO_CONFIDENCE_THRESHOLD && second_score < best_score - 0.1 {
        let tracks = scored.remove(0).2;
        let rec = &recordings[best_idx];
        println!(
            "Auto-selected: {} (score: {:.0}%, {} tracks)",
            rec.identifier,
            best_score * 100.0,
            tracks.len()
        );
        return Ok((Some(rec), Some(tracks)));
    }

    // Ambiguous: show candidate list
    let options: Vec<String> = scored
        .iter()
        .map(|(i, score, tracks)| {
            let rec = &recordings[*i];
            format!(
                "{} — {:.0}% — {} tracks — {}",
                rec.identifier,
                score * 100.0,
                tracks.len(),
                rec.source.as_deref().unwrap_or("?")
            )
        })
        .chain(std::iter::once("Skip".to_string()))
        .collect();

    let choice = match Select::new(
        &format!("{} candidates for {}", options.len() - 1, show.date),
        options,
    )
    .prompt()
    {
        Ok(c) => c,
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            return Ok((None, None))
        }
        Err(e) => return Err(e.into()),
    };

    if choice == "Skip" {
        return Ok((None, None));
    }

    // Find which recording was selected
    let selected_id = choice.split(" — ").next().unwrap_or("");
    for rec in recordings {
        if rec.identifier == selected_id {
            let tracks = archive_org::fetch_recording_tracks(client, &rec.identifier, cache_dir)?;
            return Ok((Some(rec), Some(tracks)));
        }
    }

    // No recording matched the selection — fall back to the top-scored candidate
    let (fallback_idx, _, fallback_tracks) = scored.remove(0);
    Ok((Some(&recordings[fallback_idx]), Some(fallback_tracks)))
}

fn print_alignment_table(show: &LocalShow, alignments: &[align::TrackAlignment]) {
    let date = &show.date;
    let folder = &show.folder_name;
    println!("\n{}", "=".repeat(70));
    println!(" {} — {}", date, folder);
    println!("{}", "=".repeat(70));
    println!(
        " {:>2}  {:<25} {:<25} {:>4}",
        "#", "LOCAL", "ARCHIVE.ORG", "CONF"
    );
    println!("{}", "-".repeat(70));

    for (i, a) in alignments.iter().enumerate() {
        let local_title = if i < show.tracks.len() {
            let t = &show.tracks[i].title;
            let dur = show.tracks[i].duration_secs;
            if t.is_empty() {
                format!("(untitled) ({:.0}s)", dur)
            } else {
                let display = if t.len() > 20 { &t[..20] } else { t };
                format!("{} ({:.0}s)", display, dur)
            }
        } else {
            "(?)".to_string()
        };

        let archive_title = if a.proposed_title.is_empty() {
            "—".to_string()
        } else {
            let t = &a.proposed_title;
            if t.len() > 22 {
                format!("{}...", &t[..19])
            } else {
                t.clone()
            }
        };

        let conf = if a.archive_idx.is_some() {
            format!("{:.0}%", a.confidence * 100.0)
        } else {
            "—".to_string()
        };

        println!(
            " {:>2}  {:<25} → {:<22} {:>4}",
            i + 1,
            truncate(&local_title, 25),
            archive_title,
            conf
        );
    }
    println!("{}", "=".repeat(70));
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max - 3])
    } else {
        s.to_string()
    }
}

fn select_action(confidence: f64) -> anyhow::Result<String> {
    let default = if confidence > AUTO_CONFIDENCE_THRESHOLD {
        0 // Default to "Apply all"
    } else {
        2 // Default to "Skip"
    };

    let options = vec![
        "Apply all".to_string(),
        "Apply + Next".to_string(),
        "Skip".to_string(),
        "Back".to_string(),
    ];

    match Select::new("Action", options)
        .with_starting_cursor(default)
        .prompt()
    {
        Ok(c) => Ok(c),
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
            Ok("Back".to_string())
        }
        Err(e) => Err(e.into()),
    }
}

fn apply_fixes(
    show: &LocalShow,
    alignments: &[align::TrackAlignment],
    recording: &ArchiveRecording,
    overall: f64,
    manifest: &mut Manifest,
    root_dir: &Path,
) -> anyhow::Result<()> {
    let mut track_records = Vec::new();
    let mut applied = 0u32;

    for a in alignments {
        if a.local_idx >= show.tracks.len() {
            continue;
        }
        let local_track = &show.tracks[a.local_idx];

        let status = if a.archive_idx.is_some() && !a.proposed_title.is_empty() {
            // Apply title fix
            if let Err(e) = tagger::patch_tags(&local_track.path, Some(&a.proposed_title), None, None) {
                eprintln!(
                    "  Failed to tag {}: {}",
                    local_track.path.display(),
                    e
                );
                TrackStatus::Skipped
            } else {
                applied += 1;
                TrackStatus::Applied
            }
        } else {
            TrackStatus::Unmatched
        };

        track_records.push(TrackRecord {
            file: local_track
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            original_title: local_track.title.clone(),
            applied_title: if status == TrackStatus::Applied {
                a.proposed_title.clone()
            } else {
                String::new()
            },
            archive_title: a.proposed_title.clone(),
            confidence: a.confidence,
            duration_local: local_track.duration_secs,
            duration_archive: a.archive_duration,
            status,
        });
    }

    manifest.record_show(
        show.date.clone(),
        show.folder_name.clone(),
        Some(ArchiveMatch {
            identifier: recording.identifier.clone(),
            overall_confidence: overall,
            match_method: if overall > AUTO_CONFIDENCE_THRESHOLD {
                "auto".to_string()
            } else {
                "manual".to_string()
            },
        }),
        track_records,
    );

    manifest.save(root_dir)?;
    println!("  Applied {} title fixes for {}", applied, show.date);
    Ok(())
}

fn batch_albumsort(
    shows: &[LocalShow],
    manifest: &mut Manifest,
    root_dir: &Path,
) -> anyhow::Result<()> {
    println!("Applying ALBUMSORT tags for chronological ordering...");
    let mut count = 0u32;

    for show in shows {
        // ALBUMSORT = date so shows sort chronologically
        let sort_value = &show.date;

        for track in &show.tracks {
            if let Err(e) = tagger::patch_tags(&track.path, None, Some(sort_value), None) {
                eprintln!("  Warning: {}: {}", track.path.display(), e);
            }
        }

        // Ensure show exists in manifest before marking
        if !manifest.shows.contains_key(&show.date) {
            manifest.record_show(show.date.clone(), show.folder_name.clone(), None, vec![]);
        }
        manifest.mark_albumsort(&show.date);
        count += 1;
    }

    manifest.save(root_dir)?;
    println!("Applied ALBUMSORT to {} shows", count);
    Ok(())
}

fn batch_artwork(
    shows: &[LocalShow],
    manifest: &mut Manifest,
    root_dir: &Path,
) -> anyhow::Result<()> {
    let with_covers: Vec<&LocalShow> = shows.iter().filter(|s| s.has_cover).collect();
    println!("Re-embedding artwork for {} shows...", with_covers.len());
    let mut count = 0u32;

    for show in with_covers {
        match artwork::reembed_artwork(&show.path) {
            Ok(n) => {
                let cover = scanner::find_cover_file(&show.path)
                    .unwrap_or_default()
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                if !manifest.shows.contains_key(&show.date) {
                    manifest.record_show(
                        show.date.clone(),
                        show.folder_name.clone(),
                        None,
                        vec![],
                    );
                }
                manifest.record_artwork(&show.date, cover, n);
                count += 1;
                println!("  {} — embedded in {} files", show.date, n);
            }
            Err(e) => {
                eprintln!("  {} — failed: {}", show.date, e);
            }
        }
    }

    manifest.save(root_dir)?;
    println!("Artwork embedded for {} shows", count);
    Ok(())
}

fn batch_disc_totals(shows: &[LocalShow]) -> anyhow::Result<()> {
    let multi_disc: Vec<&LocalShow> = shows.iter().filter(|s| s.disc_count > 1).collect();
    println!(
        "Fixing disc totals for {} multi-disc shows...",
        multi_disc.len()
    );
    let mut count = 0u32;

    for show in multi_disc {
        for track in &show.tracks {
            if let Err(e) = tagger::patch_tags(&track.path, None, None, Some(show.disc_count)) {
                eprintln!("  Warning: {}: {}", track.path.display(), e);
            }
        }
        count += 1;
    }

    println!("Fixed disc totals for {} shows", count);
    Ok(())
}

fn cache_base_dir() -> std::path::PathBuf {
    directories::ProjectDirs::from("", "", "nugs")
        .map(|dirs| dirs.cache_dir().to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}
