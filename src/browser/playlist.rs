use crate::api::NugsApi;
use crate::config::{expand_tilde, Config};
use crate::download::{download_track, make_track_filename, should_skip_track};
use crate::models::{FormatCode, Playlist};
use crate::transcode::{check_ffmpeg, compute_final_path, postprocess_aac};

use super::prompt::{styled_select, PromptResult};
use super::resolve::{prompt_postprocess, resolve_track_url};
use super::style::{
    clear_screen, dim, dot_leader_col_width, dot_leader_line, format_duration, MIDDOT,
};

/// Display playlist detail panel and action menu.
pub async fn playlist_detail(playlist: &Playlist, api: &mut NugsApi, config: &mut Config) {
    loop {
        clear_screen();
        print_playlist_panel(playlist);

        let choices = vec!["Download playlist".to_string()];
        match styled_select("", choices) {
            PromptResult::Choice(choice) => {
                if choice == "Download playlist" {
                    download_playlist(playlist, api, config).await;
                    return;
                }
            }
            PromptResult::Back | PromptResult::Interrupted => return,
        }
    }
}

/// Print playlist detail panel to stdout.
fn print_playlist_panel(playlist: &Playlist) {
    println!();
    println!("  \x1b[1;38;5;214m{}\x1b[0m", playlist.playlist_name);
    println!(
        "  {}",
        dim(&format!(
            "{} track{}",
            playlist.items.len(),
            if playlist.items.len() != 1 { "s" } else { "" }
        ))
    );
    println!();

    if playlist.items.is_empty() {
        return;
    }

    // Calculate dot-leader column width
    let title_lengths: Vec<usize> = playlist
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let num = format!("{:2}.", i + 1);
            num.chars().count() + 1 + item.track.song_title.chars().count()
        })
        .collect();
    let col_width = dot_leader_col_width(&title_lengths);

    for (i, item) in playlist.items.iter().enumerate() {
        let num = format!("{:2}.", i + 1);
        let song = if item.track.song_title.is_empty() {
            "Unknown"
        } else {
            &item.track.song_title
        };
        let title = format!("{num} {song}");
        let dur = format_duration(item.track.duration_seconds);
        println!("    {}", dot_leader_line(&title, &dur, col_width));

        // Sub-line: artist · venue (dim)
        let mut sub_parts = Vec::new();
        if !item.artist_name.is_empty() {
            sub_parts.push(item.artist_name.as_str());
        }
        if !item.venue_name.is_empty() {
            sub_parts.push(item.venue_name.as_str());
        }
        if !sub_parts.is_empty() {
            println!("      {}", dim(&sub_parts.join(&format!(" {MIDDOT} "))));
        }
    }
    println!();
}

/// Download all tracks in a playlist to a single folder.
///
/// No tagging (tracks come from different shows). No queue interaction.
async fn download_playlist(playlist: &Playlist, api: &mut NugsApi, config: &mut Config) {
    if playlist.items.is_empty() {
        println!("  \x1b[38;5;214mPlaylist has no tracks.\x1b[0m");
        return;
    }

    let codec = match prompt_postprocess(config) {
        Some(c) => c,
        None => return,
    };

    let output_dir = expand_tilde(&config.output_dir);
    let playlist_dir = output_dir.join(playlist.folder_name());
    if let Err(e) = std::fs::create_dir_all(&playlist_dir) {
        eprintln!("Failed to create playlist directory: {e}");
        return;
    }

    let format_code = match FormatCode::from_name(&config.format) {
        Some(fc) => fc,
        None => {
            eprintln!("Unknown format: {}", config.format);
            return;
        }
    };

    let effective_codec = if codec != "none" && !check_ffmpeg() {
        println!(
            "\x1b[38;5;214mffmpeg not found \u{2014} AAC tracks will not be converted.\x1b[0m"
        );
        "none".to_string()
    } else {
        codec
    };

    let stream_params = match &api.stream_params {
        Some(sp) => sp.clone(),
        None => {
            println!("  \x1b[38;5;203mNo stream parameters — try logging in again.\x1b[0m");
            return;
        }
    };

    let total = playlist.items.len();
    let mut downloaded = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let client = reqwest::Client::new();

    // Probe first track to detect stale params
    let current_params = if let Some(first) = playlist.items.first() {
        if resolve_track_url(api, first.track.track_id, format_code, &stream_params)
            .await
            .is_none()
        {
            println!("  {}", dim("Refreshing session..."));
            let _ = api.refresh_session().await;
        }
        api.stream_params.clone().unwrap_or(stream_params)
    } else {
        stream_params
    };

    for (i, item) in playlist.items.iter().enumerate() {
        let track = &item.track;
        let song = if track.song_title.is_empty() {
            "Unknown"
        } else {
            &track.song_title
        };

        // Resolve stream URL
        let (url, quality) =
            match resolve_track_url(api, track.track_id, format_code, &current_params).await {
                Some(resolved) => resolved,
                None => {
                    println!(
                        "  \x1b[38;5;214m[{}/{}] No stream URL: {song}\x1b[0m",
                        i + 1,
                        total
                    );
                    failed += 1;
                    continue;
                }
            };

        if quality.code == "hls" || url.contains(".m3u8") {
            println!(
                "  \x1b[38;5;214m[{}/{}] Skipping HLS: {song}\x1b[0m",
                i + 1,
                total
            );
            skipped += 1;
            continue;
        }

        let filename = make_track_filename((i + 1) as i64, &track.song_title, quality.extension);
        let download_dest = playlist_dir.join(&filename);
        let needs_convert = effective_codec != "none" && quality.code == "aac";
        let final_dest = compute_final_path(&download_dest, quality.code, &effective_codec);

        if should_skip_track(&final_dest, &download_dest, needs_convert, &effective_codec) {
            println!(
                "  \x1b[2m[{}/{}] Already downloaded: {song}\x1b[0m",
                i + 1,
                total
            );
            skipped += 1;
            continue;
        }

        // Resume interrupted conversion
        if needs_convert && download_dest.exists() {
            let (_, err) = postprocess_aac(&download_dest, &effective_codec);
            if let Some(msg) = err {
                println!(
                    "  \x1b[38;5;214m[{}/{}] Conversion failed: {song} — {msg}\x1b[0m",
                    i + 1,
                    total
                );
            }
            downloaded += 1;
            continue;
        }

        println!("  \x1b[38;5;214m[{}/{}]\x1b[0m {song}", i + 1, total);

        let pb = indicatif::ProgressBar::hidden();
        let cancel = std::sync::atomic::AtomicBool::new(false);

        // Playlists are nugs-only
        let svc = crate::service::Service::Nugs.config();
        match download_track(
            &url,
            &download_dest,
            &pb,
            &cancel,
            &client,
            svc.stream_user_agent,
            svc.player_url,
        )
        .await
        {
            Ok(_) => {
                if needs_convert {
                    let (_, err) = postprocess_aac(&download_dest, &effective_codec);
                    if let Some(msg) = err {
                        println!("  \x1b[38;5;214mConversion warning: {msg}\x1b[0m");
                    }
                }
                // No tagging for playlists
                downloaded += 1;
            }
            Err(e) => {
                println!(
                    "  \x1b[31m[{}/{}] Failed: {song} — {e}\x1b[0m",
                    i + 1,
                    total
                );
                failed += 1;
            }
        }
    }

    // Summary
    let mut parts = vec![format!(
        "\x1b[1;38;5;113mDone!\x1b[0m {downloaded}/{total} tracks downloaded"
    )];
    if skipped > 0 {
        parts.push(format!("{skipped} skipped"));
    }
    if failed > 0 {
        parts.push(format!("\x1b[38;5;214m{failed} failed\x1b[0m"));
    }
    println!("  {}", parts.join(", "));
}
