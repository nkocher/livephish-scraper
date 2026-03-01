pub mod escape;
pub mod progress;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use indicatif::ProgressBar;
use tokio::sync::Semaphore;
use tracing::{error, info, warn};

use crate::models::sanitize::sanitize_filename;
use crate::models::{FormatCode, Quality, Show, Track};
use crate::service::Service;
use crate::transcode::{
    check_ffmpeg, compute_final_path, effective_flac_target, is_already_converted, postprocess_aac,
    postprocess_flac_to_aac, postprocess_flac_to_alac,
};

use self::escape::{disable_cbreak, enable_cbreak, new_cancel_flag, spawn_escape_watcher};
use self::progress::make_overall_bar;

const ESCAPE_CHECK_INTERVAL: usize = 64;
const MAX_CONCURRENT: usize = 3;
const DOWNLOAD_MAX_RETRIES: u32 = 3; // 4 total attempts
const DOWNLOAD_RETRY_BASE_SECS: u64 = 5; // backoff: 5s, 10s, 15s

/// Tracks with their resolved stream URLs and quality info.
pub type TracksWithUrls = Vec<(Track, String, Quality)>;

/// Outcome of downloading a show.
pub struct DownloadOutcome {
    /// false = user cancelled via Escape.
    pub completed: bool,
    /// Number of tracks that failed to download.
    pub failed_count: usize,
}

/// How a downloaded track should be post-processed.
#[derive(Clone, Copy, PartialEq)]
enum ConversionMode {
    /// No conversion needed.
    None,
    /// AAC→FLAC or AAC→ALAC (controlled by postprocess_codec config).
    AacPostprocess,
    /// FLAC→ALAC (when ALAC requested but API serves FLAC).
    FlacToAlac,
    /// FLAC→AAC 256 kbps (explicit flac_convert="aac" setting).
    FlacToAac,
}

impl ConversionMode {
    /// Run the appropriate postprocess conversion for this mode.
    /// Returns `(final_path, error)` -- error is None on success.
    fn postprocess(self, source: &Path, codec: &str) -> (PathBuf, Option<String>) {
        match self {
            Self::AacPostprocess => postprocess_aac(source, codec),
            Self::FlacToAlac => postprocess_flac_to_alac(source),
            Self::FlacToAac => postprocess_flac_to_aac(source),
            Self::None => (source.to_path_buf(), None),
        }
    }
}

/// Format a track filename with leading number.
pub fn make_track_filename(track_num: i64, title: &str, extension: &str) -> String {
    format!(
        "{:02}. {}{}",
        track_num,
        sanitize_filename(title, 200),
        extension
    )
}

/// Format a disc-aware track filename: `{disc}-{track:02}. {title}{ext}`.
///
/// Used for multi-disc shows to prevent filename collisions when different
/// discs have the same track number (e.g., d1t01 and d2t01 both = track 1).
pub fn make_track_filename_with_disc(disc: i64, track_num: i64, title: &str, extension: &str) -> String {
    format!(
        "{}-{:02}. {}{}",
        disc,
        track_num,
        sanitize_filename(title, 200),
        extension
    )
}

/// Check if a track is already fully processed and should be skipped.
fn should_skip_track(
    final_dest: &Path,
    download_dest: &Path,
    conversion: ConversionMode,
) -> bool {
    if !final_dest.exists() {
        return false;
    }
    // For same-extension conversions, verify codec to avoid treating AAC .m4a as done
    match conversion {
        ConversionMode::AacPostprocess if final_dest == download_dest => {
            is_already_converted(final_dest, "alac")
        }
        ConversionMode::FlacToAlac => is_already_converted(final_dest, "alac"),
        ConversionMode::FlacToAac => is_already_converted(final_dest, "aac"),
        _ => true,
    }
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

/// Check whether an error from `download_track` is worth retrying.
///
/// Retries transport failures (connect, timeout, body, decode) and server
/// errors (5xx, 429). Fails fast on other 4xx — those indicate bad URLs or
/// auth failures that won't resolve with a retry.
fn is_retriable(err: &anyhow::Error) -> bool {
    if let Some(re) = err.downcast_ref::<reqwest::Error>() {
        if re.is_connect() || re.is_timeout() || re.is_body() || re.is_decode() {
            return true;
        }
        if let Some(status) = re.status() {
            return status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS;
        }
    }
    false
}

/// Wrap `download_track` with exponential backoff + jitter.
///
/// On transient failures the same URL is retried — `__gda__` tokens are
/// valid for days, so the issue is CDN rate-limiting, not stale URLs.
/// The semaphore permit is held during backoff sleep, which naturally
/// throttles concurrent activity against the rate-limited CDN.
#[allow(clippy::too_many_arguments)]
async fn download_track_with_retry(
    url: &str,
    dest: &Path,
    cancel: &AtomicBool,
    client: &reqwest::Client,
    user_agent: &str,
    referer: &str,
    track_title: &str,
    overall: &ProgressBar,
    base_backoff: Duration,
) -> Result<PathBuf, anyhow::Error> {
    let mut last_err = None;

    for attempt in 0..=DOWNLOAD_MAX_RETRIES {
        if attempt > 0 {
            // Jittered sleep: base * attempt ± 50%
            let base_ms = base_backoff.as_millis() as u64 * attempt as u64;
            let jitter_range = (base_ms / 2).max(1);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos() as u64;
            let jitter = (nanos % (jitter_range * 2)) as i64 - jitter_range as i64;
            let sleep_ms = (base_ms as i64 + jitter).max(0) as u64;
            tokio::time::sleep(Duration::from_millis(sleep_ms)).await;

            if cancel.load(Ordering::Relaxed) {
                return Err(anyhow::anyhow!("download cancelled"));
            }
        }

        let hidden_pb = ProgressBar::hidden();
        match download_track(url, dest, &hidden_pb, cancel, client, user_agent, referer).await {
            Ok(path) => return Ok(path),
            Err(e) => {
                if e.to_string().contains("cancelled") {
                    return Err(e);
                }
                if !is_retriable(&e) || attempt == DOWNLOAD_MAX_RETRIES {
                    return Err(e);
                }
                warn!(
                    "Attempt {}/{} failed for {}: {e}",
                    attempt + 1,
                    DOWNLOAD_MAX_RETRIES + 1,
                    track_title,
                );
                overall.println(format!(
                    "  \x1b[2m\u{21bb} Retrying {}...\x1b[0m",
                    track_title
                ));
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("download failed")))
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
/// Returns a `DownloadOutcome` with completion status and failure count.
pub async fn download_show(
    show: &Show,
    tracks_with_urls: &TracksWithUrls,
    output_dir: &Path,
    postprocess_codec: &str,
    flac_convert: &str,
    service: Service,
    requested_format: FormatCode,
) -> DownloadOutcome {
    let show_dir = output_dir.join(show.folder_name());
    if let Err(e) = std::fs::create_dir_all(&show_dir) {
        error!("Failed to create show directory: {e}");
        return DownloadOutcome { completed: true, failed_count: 0 };
    }

    let has_ffmpeg = check_ffmpeg();

    let effective_codec = if postprocess_codec != "none" && !has_ffmpeg {
        warn!("ffmpeg not found — AAC tracks will not be converted");
        "none"
    } else {
        postprocess_codec
    };

    let flac_target = effective_flac_target(flac_convert, requested_format.name());
    let mut warned_no_ffmpeg_flac = false;

    // Detect multi-disc shows for disc-prefixed filenames
    let max_disc = tracks_with_urls.iter().map(|(t, _, _)| t.disc_num).max().unwrap_or(1);

    // ── Phase 1: Pre-filter (skip/resume) ───────────────────────────
    let mut to_download: Vec<(Track, String, Quality, PathBuf, ConversionMode)> = Vec::new();
    let mut pre_completed = 0usize;
    let mut pre_skipped_names: Vec<String> = Vec::new();

    for (track, url, quality) in tracks_with_urls {
        if quality.code == "hls" || url.contains(".m3u8") {
            warn!("Skipping HLS-only track: {}", track.song_title);
            continue;
        }

        let filename = if max_disc > 1 {
            make_track_filename_with_disc(track.disc_num, track.track_num, &track.song_title, quality.extension)
        } else {
            make_track_filename(track.track_num, &track.song_title, quality.extension)
        };
        let download_dest = show_dir.join(&filename);

        let conversion = if effective_codec != "none" && quality.code == "aac" {
            ConversionMode::AacPostprocess
        } else if quality.code == "flac" && flac_target != "none" {
            if !has_ffmpeg {
                if !warned_no_ffmpeg_flac {
                    warn!("ffmpeg not found — FLAC tracks will not be converted");
                    warned_no_ffmpeg_flac = true;
                }
                ConversionMode::None
            } else {
                match flac_target {
                    "aac" => ConversionMode::FlacToAac,
                    _ => ConversionMode::FlacToAlac,
                }
            }
        } else {
            ConversionMode::None
        };

        let effective_flac_target = match conversion {
            ConversionMode::FlacToAlac | ConversionMode::FlacToAac => flac_target,
            _ => "none",
        };
        let final_dest = compute_final_path(
            &download_dest,
            quality.code,
            effective_codec,
            effective_flac_target,
        );

        // Already fully processed
        if should_skip_track(&final_dest, &download_dest, conversion) {
            pre_skipped_names.push(track.song_title.clone());
            pre_completed += 1;
            continue;
        }

        // Resume interrupted conversion
        if conversion != ConversionMode::None && download_dest.exists() {
            let (actual, err) = conversion.postprocess(&download_dest, effective_codec);
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
            conversion,
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
        return DownloadOutcome { completed: true, failed_count: 0 };
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
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap();

    let mut handles = Vec::new();

    for (track, url, _quality, download_dest, conversion) in to_download {
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

            match download_track_with_retry(
                &url,
                &download_dest,
                &cancel,
                &client_ref,
                stream_ua,
                referer,
                &track.song_title,
                &overall,
                Duration::from_secs(DOWNLOAD_RETRY_BASE_SECS),
            )
            .await
            {
                Ok(_) => {
                    // Post-process based on conversion mode
                    let actual_dest = {
                        let (actual, err) = conversion.postprocess(&download_dest, &codec_ref);
                        if let Some(msg) = err {
                            warn!("Conversion failed: {} — {msg}", track.song_title);
                            download_dest
                        } else {
                            actual
                        }
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

    DownloadOutcome {
        completed: !cancelled,
        failed_count: failed.len(),
    }
}

/// Retry wrapper around `download_show` for bulk downloads.
///
/// Calls `download_show` up to 3 times total. Between passes, tracks that
/// succeeded on earlier passes are automatically skipped by `should_skip_track`.
/// Aborts early if no progress is made (same failure count as previous pass).
#[allow(clippy::too_many_arguments)]
pub async fn download_show_with_retry(
    show: &Show,
    tracks_with_urls: &TracksWithUrls,
    output_dir: &Path,
    postprocess_codec: &str,
    flac_convert: &str,
    service: Service,
    requested_format: FormatCode,
    pass_cooldown: Duration,
) -> DownloadOutcome {
    const MAX_SHOW_RETRIES: u32 = 2; // 3 total passes

    let mut prev_failed = usize::MAX;

    for pass in 0..=MAX_SHOW_RETRIES {
        let outcome = download_show(
            show,
            tracks_with_urls,
            output_dir,
            postprocess_codec,
            flac_convert,
            service,
            requested_format,
        )
        .await;

        if !outcome.completed {
            return outcome; // user cancelled
        }

        if outcome.failed_count == 0 {
            return outcome; // all tracks succeeded
        }

        // No progress — same or more failures than last pass
        if outcome.failed_count >= prev_failed {
            return outcome;
        }

        prev_failed = outcome.failed_count;

        if pass < MAX_SHOW_RETRIES {
            println!(
                "  \x1b[38;5;214m\u{21bb} Retrying {} failed track{} in {}s...\x1b[0m",
                outcome.failed_count,
                if outcome.failed_count != 1 { "s" } else { "" },
                pass_cooldown.as_secs(),
            );
            tokio::time::sleep(pass_cooldown).await;
        } else {
            return outcome;
        }
    }

    // Unreachable, but satisfies the compiler
    DownloadOutcome { completed: true, failed_count: prev_failed }
}

#[cfg(test)]
mod tests;
