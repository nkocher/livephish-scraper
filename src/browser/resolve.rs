// Stream URL format fallback, retry with refresh, postprocess prompt

use std::collections::HashMap;

use indicatif::{ProgressBar, ProgressStyle};
use tracing::{info, warn};

use crate::api::NugsApi;
use crate::config::{save_config, Config};
use crate::download::TracksWithUrls;
use crate::models::{FormatCode, Quality, Show, StreamParams};
use crate::transcode::check_ffmpeg;

use super::prompt::{styled_select, PromptResult};
use super::style::dim;

/// Stats from resolving stream URLs for a show's tracks.
pub struct ResolveStats {
    /// Format code -> count of tracks that got a different format than requested.
    pub mismatch_counts: HashMap<String, usize>,
    /// Tracks that returned an empty stream URL (all formats exhausted).
    pub no_stream_url: usize,
}

/// Resolve a single track's stream URL with format fallback.
///
/// Walks the fallback chain (e.g., ALAC->FLAC->AAC) until a non-empty
/// URL is returned or the chain is exhausted.
pub async fn resolve_track_url(
    api: &mut NugsApi,
    track_id: i64,
    format_code: FormatCode,
    stream_params: &StreamParams,
) -> Option<(String, Quality)> {
    let mut current = Some(format_code);

    while let Some(fc) = current {
        match api
            .get_stream_url_for_service(track_id, fc.code(api.service), stream_params)
            .await
        {
            Ok(url) if !url.is_empty() => {
                if let Some(quality) = Quality::from_stream_url(&url) {
                    info!(
                        "Resolved track {} with format {} ({})",
                        track_id,
                        fc.name(),
                        quality.specs
                    );
                    return Some((url, quality));
                }
                // URL returned but couldn't detect quality — use format code as fallback
                let quality = Quality::from_format_code(fc);
                warn!(
                    "Could not detect quality from URL for track {}, using format {} ({})",
                    track_id,
                    fc.name(),
                    quality.specs
                );
                return Some((url, quality));
            }
            Ok(_) => {
                // Empty URL — format not available, try fallback
                info!(
                    "Empty stream URL for track {} with format {}, trying fallback",
                    track_id,
                    fc.name()
                );
            }
            Err(e) => {
                warn!(
                    "Error resolving track {} with format {}: {}",
                    track_id,
                    fc.name(),
                    e
                );
            }
        }
        current = fc.fallback();
    }

    None
}

/// Resolve stream URLs for all tracks in a show, tracking format mismatches.
///
/// Reads `api.stream_params` as the single source of truth.
pub async fn resolve_tracks(
    show: &Show,
    api: &mut NugsApi,
    format_code: FormatCode,
) -> (TracksWithUrls, ResolveStats) {
    let stream_params = match &api.stream_params {
        Some(sp) => sp.clone(),
        None => {
            return (
                Vec::new(),
                ResolveStats {
                    mismatch_counts: HashMap::new(),
                    no_stream_url: show.tracks.len(),
                },
            );
        }
    };

    let requested_format = format_code.name();
    let mut results: TracksWithUrls = Vec::new();
    let mut mismatch_counts: HashMap<String, usize> = HashMap::new();
    let mut no_stream_url: usize = 0;

    let total = show.tracks.len();
    let pb = ProgressBar::new(total as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "  \x1b[2mResolving\x1b[0m [{bar:20.yellow/dim}] \x1b[38;5;214m{pos}\x1b[0m/{len}",
        )
        .unwrap()
        .progress_chars("━╸─"),
    );

    for track in &show.tracks {
        match resolve_track_url(api, track.track_id, format_code, &stream_params).await {
            Some((url, quality)) => {
                if quality.code != requested_format {
                    *mismatch_counts.entry(quality.code.to_string()).or_default() += 1;
                }
                results.push((track.clone(), url, quality));
            }
            None => {
                no_stream_url += 1;
            }
        }
        pb.inc(1);
    }
    pb.finish_and_clear();

    let stats = ResolveStats {
        mismatch_counts,
        no_stream_url,
    };
    (results, stats)
}

/// Retry stream URL resolution after refreshing the session.
///
/// Called when initial resolution returns zero tracks with `no_stream_url > 0`,
/// which typically indicates stale StreamParams (subscription timestamps
/// embedded in URL query strings have expired).
pub async fn retry_resolve_with_refresh(
    show: &Show,
    api: &mut NugsApi,
    format_code: FormatCode,
    indent: &str,
) -> (TracksWithUrls, ResolveStats) {
    println!("{indent}\x1b[2mRefreshing session and retrying...\x1b[0m");

    match api.refresh_session().await {
        Ok(_) => resolve_tracks(show, api, format_code).await,
        Err(e) => {
            println!("{indent}\x1b[38;5;214mSession refresh failed: {e}\x1b[0m");
            println!("{indent}\x1b[2mTry restarting with: nugs -f\x1b[0m");
            (
                Vec::new(),
                ResolveStats {
                    mismatch_counts: HashMap::new(),
                    no_stream_url: show.tracks.len(),
                },
            )
        }
    }
}

/// Print warnings about format mismatches, unknown formats, and failed tracks.
pub fn print_resolution_warnings(stats: &ResolveStats, indent: &str) {
    if !stats.mismatch_counts.is_empty() {
        let details: Vec<String> = stats
            .mismatch_counts
            .iter()
            .map(|(code, count)| format!("{count}x {code}"))
            .collect();
        let total: usize = stats.mismatch_counts.values().sum();
        println!(
            "{indent}\x1b[38;5;214mFormat fallbacks: {} track{} served as {}\x1b[0m",
            total,
            if total != 1 { "s" } else { "" },
            details.join(", "),
        );
    }
    if stats.no_stream_url > 0 {
        println!(
            "{indent}\x1b[38;5;214m{} track{} returned no stream URL.\x1b[0m",
            stats.no_stream_url,
            if stats.no_stream_url != 1 { "s" } else { "" },
        );
    }
}

/// Prompt user for AAC post-processing codec before download.
///
/// Returns `Some(codec)` on selection ("none", "flac", "alac"),
/// or `None` if user pressed Escape (caller should abort download).
/// Saves choice to config if changed.
pub fn prompt_postprocess(config: &mut Config) -> Option<String> {
    let choices = vec![
        "No conversion (keep original)".to_string(),
        "Convert AAC to FLAC".to_string(),
        "Convert AAC to ALAC".to_string(),
    ];

    let choice = match styled_select(
        &format!("AAC conversion {}", dim("(esc to cancel download)")),
        choices,
    ) {
        PromptResult::Choice(c) => c,
        PromptResult::Back | PromptResult::Interrupted => return None,
    };

    let codec = if choice.contains("FLAC") {
        "flac"
    } else if choice.contains("ALAC") {
        "alac"
    } else {
        "none"
    };

    // Validate ffmpeg if conversion selected
    let effective_codec = if codec != "none" && !check_ffmpeg() {
        println!(
            "\x1b[38;5;214mffmpeg not found \u{2014} AAC tracks will not be converted.\x1b[0m"
        );
        "none"
    } else {
        codec
    };

    // Save to config if changed
    if effective_codec != config.postprocess_codec {
        config.postprocess_codec = effective_codec.to_string();
        save_config(config);
    }

    Some(effective_codec.to_string())
}
