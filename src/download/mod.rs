pub mod escape;
pub mod progress;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_util::StreamExt;
use indicatif::ProgressBar;
use tokio::sync::Semaphore;
use tracing::{error, info, warn};

use crate::models::sanitize::sanitize_filename;
use crate::models::{Quality, Show, Track};
use crate::service::Service;
use crate::transcode::{check_ffmpeg, compute_final_path, is_already_converted, postprocess_aac};

use self::escape::{disable_cbreak, enable_cbreak, new_cancel_flag, spawn_escape_watcher};
use self::progress::make_overall_bar;

const ESCAPE_CHECK_INTERVAL: usize = 64;
const MAX_CONCURRENT: usize = 3;

/// Tracks with their resolved stream URLs and quality info.
pub type TracksWithUrls = Vec<(Track, String, Quality)>;

/// Format a track filename with leading number.
pub fn make_track_filename(track_num: i64, title: &str, extension: &str) -> String {
    format!(
        "{:02}. {}{}",
        track_num,
        sanitize_filename(title, 200),
        extension
    )
}

/// Check if a track is already fully processed and should be skipped.
pub fn should_skip_track(
    final_dest: &Path,
    download_dest: &Path,
    needs_convert: bool,
    postprocess_codec: &str,
) -> bool {
    if !final_dest.exists() {
        return false;
    }
    if needs_convert && final_dest == download_dest && postprocess_codec == "alac" {
        return is_already_converted(final_dest, "alac");
    }
    true
}

/// Return the .part path for a destination file.
fn part_path(dest: &Path) -> PathBuf {
    let mut ext = dest.extension().unwrap_or_default().to_os_string();
    ext.push(".part");
    dest.with_extension(ext)
}

/// Download a single track to disk with progress bar and cancellation.
///
/// The caller provides the ProgressBar (for progress updates or hidden)
/// and the cancel flag (shared AtomicBool for Escape detection).
/// Writes to a `.part` file and atomically renames on success.
pub async fn download_track(
    url: &str,
    dest: &Path,
    pb: &ProgressBar,
    cancel: &AtomicBool,
    client: &reqwest::Client,
    user_agent: &str,
    referer: &str,
) -> Result<PathBuf, anyhow::Error> {
    let part = part_path(dest);
    let _ = std::fs::remove_file(&part);

    let response = client
        .get(url)
        .header("Referer", referer)
        .header("User-Agent", user_agent)
        .header("Range", "bytes=0-")
        .send()
        .await?;

    response.error_for_status_ref()?;

    let total = response.content_length().unwrap_or(0);
    pb.set_length(total);

    let mut stream = response.bytes_stream();
    let mut file = std::fs::File::create(&part)?;
    let mut chunk_count: usize = 0;

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result?;
        std::io::Write::write_all(&mut file, &chunk)?;
        pb.inc(chunk.len() as u64);
        chunk_count += 1;

        if chunk_count.is_multiple_of(ESCAPE_CHECK_INTERVAL) && cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = std::fs::remove_file(&part);
            pb.finish_and_clear();
            return Err(anyhow::anyhow!("download cancelled"));
        }
    }

    pb.finish_and_clear();
    drop(file);

    std::fs::rename(&part, dest)?;
    info!(
        "Downloaded: {}",
        dest.file_name().unwrap_or_default().to_string_lossy()
    );

    Ok(dest.to_path_buf())
}

/// Result of downloading + processing a single track.
enum TrackResult {
    /// Track completed successfully.
    Done { tag_failed: bool },
    /// Track download or processing failed.
    Failed(String),
    /// Download was cancelled by user (Escape key).
    Cancelled,
}

/// Download all tracks in a show with parallel progress.
///
/// Downloads up to 3 tracks concurrently. Uses a single overall progress
/// bar with `pb.println()` for per-track completion messages. This avoids
/// MultiProgress rendering issues with concurrent bar insertion/removal.
/// Returns true if completed normally, false if user cancelled via Escape.
pub async fn download_show(
    show: &Show,
    tracks_with_urls: &TracksWithUrls,
    output_dir: &Path,
    postprocess_codec: &str,
    service: Service,
) -> bool {
    let show_dir = output_dir.join(show.folder_name());
    if let Err(e) = std::fs::create_dir_all(&show_dir) {
        error!("Failed to create show directory: {e}");
        return true;
    }

    let effective_codec = if postprocess_codec != "none" && !check_ffmpeg() {
        warn!("ffmpeg not found — AAC tracks will not be converted");
        "none"
    } else {
        postprocess_codec
    };

    // ── Phase 1: Pre-filter (skip/resume) ───────────────────────────
    let mut to_download: Vec<(Track, String, Quality, PathBuf, bool)> = Vec::new();
    let mut pre_completed = 0usize;
    let mut pre_skipped_names: Vec<String> = Vec::new();

    for (track, url, quality) in tracks_with_urls {
        if quality.code == "hls" || url.contains(".m3u8") {
            warn!("Skipping HLS-only track: {}", track.song_title);
            continue;
        }

        let filename = make_track_filename(track.track_num, &track.song_title, quality.extension);
        let download_dest = show_dir.join(&filename);
        let needs_convert = effective_codec != "none" && quality.code == "aac";
        let final_dest = if needs_convert {
            compute_final_path(&download_dest, quality.code, effective_codec)
        } else {
            download_dest.clone()
        };

        // Already fully processed
        if should_skip_track(&final_dest, &download_dest, needs_convert, effective_codec) {
            pre_skipped_names.push(track.song_title.clone());
            pre_completed += 1;
            continue;
        }

        // Resume interrupted conversion
        if needs_convert && download_dest.exists() {
            let (actual, err) = postprocess_aac(&download_dest, effective_codec);
            if let Some(msg) = err {
                warn!("Conversion resume failed: {} — {msg}", track.song_title);
            } else if let Err(e) = crate::tagger::tag_track(&actual, show, track) {
                warn!("Failed to tag {}: {e}", track.song_title);
            }
            pre_completed += 1;
            continue;
        }

        to_download.push((
            track.clone(),
            url.clone(),
            quality.clone(),
            download_dest,
            needs_convert,
        ));
    }

    if !pre_skipped_names.is_empty() {
        println!(
            "  \x1b[2m{} track{} already downloaded\x1b[0m",
            pre_skipped_names.len(),
            if pre_skipped_names.len() != 1 {
                "s"
            } else {
                ""
            }
        );
    }

    if to_download.is_empty() {
        return true;
    }

    // ── Phase 2: Parallel download ──────────────────────────────────
    let download_count = to_download.len();
    let concurrent = MAX_CONCURRENT.min(download_count);

    println!(
        "  Downloading {} track{} ({} concurrent)",
        download_count,
        if download_count != 1 { "s" } else { "" },
        concurrent,
    );

    // Single overall progress bar — track completions print above it
    let overall_pb = make_overall_bar(download_count);

    let svc_config = service.config();
    let stream_ua: &'static str = svc_config.stream_user_agent;
    let referer: &'static str = svc_config.player_url;

    enable_cbreak();
    let cancel_flag = new_cancel_flag();
    let escape_handle = spawn_escape_watcher(cancel_flag.clone());
    let semaphore = Arc::new(Semaphore::new(concurrent));
    let show_arc = Arc::new(show.clone());
    let codec = effective_codec.to_string();
    let client = reqwest::Client::new();

    let mut handles = Vec::new();

    for (track, url, _quality, download_dest, needs_convert) in to_download {
        let sem = semaphore.clone();
        let cancel = cancel_flag.clone();
        let overall = overall_pb.clone();
        let show_ref = show_arc.clone();
        let codec_ref = codec.clone();
        let client_ref = client.clone();

        let handle = tokio::spawn(async move {
            // Check cancellation before waiting for a permit so queued
            // tasks exit immediately after Escape instead of cycling
            // through the semaphore one by one.
            if cancel.load(Ordering::Relaxed) {
                return TrackResult::Cancelled;
            }

            let _permit = match sem.acquire().await {
                Ok(p) => p,
                Err(_) => return TrackResult::Failed(track.song_title.clone()),
            };

            if cancel.load(Ordering::Relaxed) {
                return TrackResult::Cancelled;
            }

            // Use a hidden bar for download_track's internal progress/cancel checks.
            // Per-track byte progress is not displayed — the overall bar + completion
            // messages provide the user-facing feedback.
            let hidden_pb = ProgressBar::hidden();

            match download_track(
                &url,
                &download_dest,
                &hidden_pb,
                &cancel,
                &client_ref,
                stream_ua,
                referer,
            )
            .await
            {
                Ok(_) => {
                    // Post-process if AAC conversion configured
                    let actual_dest = if needs_convert {
                        let (actual, err) = postprocess_aac(&download_dest, &codec_ref);
                        if let Some(msg) = err {
                            warn!("Conversion failed: {} — {msg}", track.song_title);
                            download_dest
                        } else {
                            actual
                        }
                    } else {
                        download_dest
                    };

                    // Tag
                    let tag_failed =
                        crate::tagger::tag_track(&actual_dest, &show_ref, &track).is_err();
                    if tag_failed {
                        warn!("Failed to tag: {}", track.song_title);
                    }

                    overall.println(format!(
                        "  \x1b[38;5;113m\u{2713}\x1b[0m {}",
                        track.song_title
                    ));
                    overall.inc(1);
                    TrackResult::Done { tag_failed }
                }
                Err(e) => {
                    if e.to_string().contains("cancelled") {
                        // Don't set cancel flag here — the escape watcher
                        // already set it. Re-setting on error string match
                        // would misfire on network errors containing "cancelled".
                        TrackResult::Cancelled
                    } else {
                        error!("Failed to download {}: {e}", track.song_title);
                        overall.println(format!("  \x1b[31m\u{2717}\x1b[0m {}", track.song_title));
                        overall.inc(1);
                        TrackResult::Failed(track.song_title.clone())
                    }
                }
            }
        });

        handles.push(handle);
    }

    // ── Collect results ─────────────────────────────────────────────
    let mut dl_completed = 0usize;
    let mut failed: Vec<String> = Vec::new();
    let mut tag_failures = 0usize;
    let mut cancelled = false;

    for handle in handles {
        match handle.await {
            Ok(TrackResult::Done { tag_failed }) => {
                dl_completed += 1;
                if tag_failed {
                    tag_failures += 1;
                }
            }
            Ok(TrackResult::Failed(name)) => {
                failed.push(name);
            }
            Ok(TrackResult::Cancelled) => {
                cancelled = true;
            }
            Err(e) => {
                error!("Task panicked: {e}");
            }
        }
    }

    // Stop escape watcher and restore terminal
    cancel_flag.store(true, Ordering::Relaxed);
    let _ = escape_handle.await;
    disable_cbreak();

    overall_pb.finish_and_clear();

    // ── Summary ─────────────────────────────────────────────────────
    let total_completed = pre_completed + dl_completed;
    let total_tracks = tracks_with_urls.len();

    if cancelled {
        println!(
            "\n  \x1b[38;5;214mEscape pressed — {total_completed}/{total_tracks} tracks saved.\x1b[0m"
        );
    } else if failed.is_empty() {
        println!(
            "  \x1b[1;38;5;113m\u{2713}\x1b[0m {total_completed}/{total_tracks} tracks downloaded"
        );
    } else {
        println!(
            "  \x1b[38;5;214m{dl_completed}/{download_count} downloaded, {} failed\x1b[0m",
            failed.len()
        );
    }

    if tag_failures > 0 {
        println!(
            "  \x1b[2m{tag_failures} tag warning{}\x1b[0m",
            if tag_failures != 1 { "s" } else { "" }
        );
    }

    !cancelled
}

#[cfg(test)]
mod tests;
