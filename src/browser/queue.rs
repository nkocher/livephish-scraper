use indexmap::IndexMap;

use std::time::Duration;

use crate::config::{expand_tilde, Config};
use crate::download::download_show_with_retry;
use crate::models::show::DisplayLocation;
use crate::models::{CatalogShow, FormatCode};
use crate::service::router::ServiceRouter;
use crate::service::Service;

use super::prompt::{styled_confirm, styled_fuzzy, styled_select, PromptResult};
use super::resolve::{print_resolution_warnings, prompt_postprocess, resolve_tracks};
use super::style::{clear_screen, dim, format_show_label, print_section, MIDDOT};

/// Return the display date for a show, or "Unknown date" if empty.
fn date_or_unknown(show: &CatalogShow) -> &str {
    let d = show.display_date();
    if d.is_empty() { "Unknown date" } else { d }
}

/// Return the artist name for a show, or "Unknown artist" if empty.
fn artist_or_unknown(show: &CatalogShow) -> &str {
    if show.artist_name.is_empty() { "Unknown artist" } else { &show.artist_name }
}

/// Queue management UI: display queue, download all, remove/clear.
pub async fn manage_queue(
    queue: &mut IndexMap<i64, CatalogShow>,
    router: &mut ServiceRouter,
    config: &mut Config,
) {
    if queue.is_empty() {
        println!("\x1b[38;5;214mQueue is empty.\x1b[0m");
        pause();
        return;
    }

    loop {
        if queue.is_empty() {
            println!("\x1b[38;5;214mQueue is now empty.\x1b[0m");
            pause();
            return;
        }

        clear_screen();
        print_section(
            "Download Queue",
            Some(&format!(
                "{} show{}",
                queue.len(),
                if queue.len() != 1 { "s" } else { "" }
            )),
        );

        // Print numbered list
        for (i, show) in queue.values().enumerate() {
            let date = date_or_unknown(show);
            let artist = artist_or_unknown(show);
            let venue = if show.venue_name.is_empty() {
                "Unknown venue"
            } else {
                &show.venue_name
            };
            let loc_short = show.display_location_short();
            let loc_part = if loc_short.is_empty() {
                String::new()
            } else {
                format!(" {MIDDOT} {}", dim(&loc_short))
            };
            println!(
                "  \x1b[38;5;214m{:>2}.\x1b[0m {date} {MIDDOT} {artist} {MIDDOT} {venue}{loc_part}",
                i + 1
            );
        }
        println!();

        let choices = vec![
            "Download all".to_string(),
            "Remove a show".to_string(),
            "Clear queue".to_string(),
        ];

        match styled_select("Queue", choices) {
            PromptResult::Choice(choice) => match choice.as_str() {
                "Download all" => {
                    download_queued_shows(queue, router, config).await;
                    return;
                }
                "Remove a show" => {
                    remove_from_queue(queue);
                }
                "Clear queue" => {
                    if let PromptResult::Choice(true) = styled_confirm("Clear entire queue?", false)
                    {
                        queue.clear();
                        println!("\x1b[38;5;214mQueue cleared.\x1b[0m");
                        pause();
                        return;
                    }
                }
                _ => {}
            },
            PromptResult::Back | PromptResult::Interrupted => return,
        }
    }
}

/// Prompt user to pick a show to remove from the queue.
fn remove_from_queue(queue: &mut IndexMap<i64, CatalogShow>) {
    let labels: Vec<String> = queue
        .values()
        .map(|s| format_show_label(s, "", true))
        .collect();

    match styled_fuzzy("Remove which show?", labels.clone()) {
        PromptResult::Choice(label) => {
            if let Some(pos) = labels.iter().position(|l| *l == label) {
                if let Some((&container_id, _)) = queue.get_index(pos) {
                    queue.shift_remove(&container_id);
                    println!("\x1b[38;5;214mRemoved.\x1b[0m");
                }
            }
        }
        PromptResult::Back | PromptResult::Interrupted => {}
    }
}

/// Download all queued shows sequentially.
pub async fn download_queued_shows(
    queue: &mut IndexMap<i64, CatalogShow>,
    router: &mut ServiceRouter,
    config: &mut Config,
) {
    // Prompt for postprocess codec
    let codec = match prompt_postprocess(config) {
        Some(c) => c,
        None => return,
    };

    let output_dir = expand_tilde(&config.output_dir);
    if let Err(e) = std::fs::create_dir_all(&output_dir) {
        eprintln!("Failed to create output directory: {e}");
        pause();
        return;
    }

    let format_code = match FormatCode::from_name(&config.format) {
        Some(fc) => fc,
        None => {
            eprintln!("Unknown format: {}", config.format);
            pause();
            return;
        }
    };

    let total_shows = queue.len();
    let mut downloaded_shows = 0usize;
    let mut skipped_shows = 0usize;
    let mut any_failures = false;

    // Collect show data up front to avoid borrow issues during iteration
    let shows: Vec<(i64, CatalogShow)> =
        queue.iter().map(|(&id, show)| (id, show.clone())).collect();

    for (i, (_container_id, catalog_show)) in shows.iter().enumerate() {
        let date = date_or_unknown(catalog_show);
        let artist = artist_or_unknown(catalog_show);
        let location = catalog_show.display_location();
        let location_str = if location.is_empty() {
            "Unknown venue"
        } else {
            &location
        };

        println!(
            "\n\x1b[1;38;5;214m[{}/{}]\x1b[0m \x1b[1m{date} {MIDDOT} {artist} {MIDDOT} {location_str}\x1b[0m",
            i + 1,
            total_shows
        );

        // Fetch full show detail + resolve tracks (branch on service)
        let (mut show, tracks_with_urls, flac_convert) = if catalog_show.service == Service::Bman {
            let bman = match router.bman_api() {
                Some(b) => b,
                None => {
                    println!("  \x1b[31mBman API not available\x1b[0m");
                    skipped_shows += 1;
                    continue;
                }
            };

            let show = match crate::bman::download::fetch_bman_show_detail(bman, catalog_show)
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    println!("  \x1b[31mError fetching Bman show: {e}\x1b[0m");
                    skipped_shows += 1;
                    continue;
                }
            };

            let twu = crate::bman::download::resolve_bman_tracks(&show, bman);
            let fc = crate::bman::download::bman_flac_convert(&config.flac_convert).to_string();
            (show, twu, fc)
        } else {
            let show = match router
                .api_for(catalog_show.service)
                .get_show_detail(catalog_show.container_id)
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    println!("  \x1b[31mError fetching show: {e}\x1b[0m");
                    skipped_shows += 1;
                    continue;
                }
            };

            let api = router.api_for(catalog_show.service);
            let (twu, stats) = resolve_tracks(&show, api, format_code).await;
            print_resolution_warnings(&stats, "  ");
            (show, twu, config.flac_convert.clone())
        };

        if tracks_with_urls.is_empty() {
            println!("  \x1b[38;5;214mNo downloadable tracks found.\x1b[0m");
            skipped_shows += 1;
            continue;
        }

        // Bman: enrich metadata before download
        if catalog_show.service == Service::Bman {
            crate::bman::download::bman_enrich_metadata(
                &mut show,
                &output_dir,
                &config.bman.setlistfm_api_key,
            )
            .await;
        }

        let outcome = download_show_with_retry(
            &show,
            &tracks_with_urls,
            &output_dir,
            &codec,
            &flac_convert,
            catalog_show.service,
            format_code,
            Duration::from_secs(30),
        )
        .await;

        // Bman: download artwork + select best cover + embed after download
        if catalog_show.service == Service::Bman && outcome.completed {
            if let Some(bman) = router.bman_api() {
                crate::bman::download::bman_save_cover_art(&show, &output_dir, bman).await;
            }
        }

        if !outcome.completed {
            // User cancelled — stop remaining shows
            break;
        }
        downloaded_shows += 1;
        if outcome.failed_count > 0 {
            any_failures = true;
        }

        // Inter-show cooldown (skip after last show)
        if i + 1 < shows.len() {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    // Summary
    let mut parts = vec![format!(
        "\x1b[1;38;5;113mDone!\x1b[0m {downloaded_shows}/{total_shows} shows downloaded"
    )];
    if skipped_shows > 0 {
        parts.push(format!("\x1b[38;5;214m{skipped_shows} skipped\x1b[0m"));
    }
    println!("{}", parts.join(", "));

    // Clear queue only if all shows completed with no residual failures
    if skipped_shows == 0 && downloaded_shows == total_shows && !any_failures {
        queue.clear();
    }

    pause();
}

/// Pause for user to read a message before returning to menu.
fn pause() {
    use std::io::{self, Read};
    println!("\nPress Enter to continue...");
    let _ = io::stdin().read(&mut [0u8]);
}
