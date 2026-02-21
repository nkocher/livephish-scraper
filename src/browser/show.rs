use indexmap::IndexMap;

use crate::config::{expand_tilde, Config};
use crate::download::download_show;
use crate::models::show::DisplayLocation;
use crate::models::{CatalogShow, FormatCode, Show};
use crate::service::router::ServiceRouter;

use super::prompt::{styled_fuzzy, styled_select, PromptResult};
use super::queue::download_queued_shows;
use super::resolve::{print_resolution_warnings, prompt_postprocess, resolve_tracks};
use super::style::{
    clear_screen, dim, dot_leader_col_width, dot_leader_line, format_duration, format_show_label,
    is_queue_action, is_sort_toggle, print_section, queue_action_label, sort_toggle_label, MIDDOT,
};

/// Sort modes for show lists.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SortMode {
    Newest,
    Oldest,
    Venue,
}

impl SortMode {
    /// Cycle to the next sort mode.
    fn next(self) -> Self {
        match self {
            SortMode::Newest => SortMode::Oldest,
            SortMode::Oldest => SortMode::Venue,
            SortMode::Venue => SortMode::Newest,
        }
    }

    /// Display name for the toggle label.
    fn label(self) -> &'static str {
        match self {
            SortMode::Newest => "Newest first",
            SortMode::Oldest => "Oldest first",
            SortMode::Venue => "Venue A\u{2013}Z",
        }
    }
}

/// Sort shows in-place by the given mode.
fn sort_shows(shows: &mut [CatalogShow], mode: SortMode) {
    match mode {
        SortMode::Newest => shows.sort_by(|a, b| b.performance_date.cmp(&a.performance_date)),
        SortMode::Oldest => shows.sort_by(|a, b| a.performance_date.cmp(&b.performance_date)),
        SortMode::Venue => shows.sort_by(|a, b| {
            a.venue_name
                .to_lowercase()
                .cmp(&b.venue_name.to_lowercase())
                .then(a.performance_date.cmp(&b.performance_date))
        }),
    }
}

/// Display a list of shows with fuzzy selection, then show detail on pick.
pub async fn show_list(
    shows: &[CatalogShow],
    router: &mut ServiceRouter,
    config: &mut Config,
    queue: &mut IndexMap<i64, CatalogShow>,
    title: &str,
) {
    if shows.is_empty() {
        println!("No shows to display.");
        return;
    }

    // Clone input so sorting doesn't mutate the caller's data
    let mut sorted_shows = shows.to_vec();
    let mut sort_mode = SortMode::Newest;
    sort_shows(&mut sorted_shows, sort_mode);

    // Detect single-artist mode
    let single_artist = shows
        .iter()
        .map(|s| s.artist_id)
        .collect::<std::collections::HashSet<_>>()
        .len()
        == 1;

    loop {
        clear_screen();
        if !title.is_empty() {
            print_section(title, Some(&format!("{} shows", shows.len())));
        }

        // Build choice labels: sort toggle + queue action, then shows
        let toggle = sort_toggle_label(sort_mode.label());

        let not_in_queue_count = sorted_shows
            .iter()
            .filter(|s| !queue.contains_key(&s.container_id))
            .count();
        let all_queued = not_in_queue_count == 0;
        let queue_label = if all_queued {
            let in_queue_count = sorted_shows.len();
            queue_action_label(&format!("Remove all from queue ({in_queue_count})"))
        } else {
            queue_action_label(&format!("Add all to queue ({not_in_queue_count})"))
        };

        let mut labels: Vec<String> = vec![toggle, queue_label];

        labels.extend(sorted_shows.iter().map(|s| {
            let prefix = if queue.contains_key(&s.container_id) {
                "\u{2713} " // checkmark
            } else {
                ""
            };
            format_show_label(s, prefix, !single_artist)
        }));

        match styled_fuzzy("Select show:", labels.clone()) {
            PromptResult::Choice(label) => {
                if is_sort_toggle(&label) {
                    // Cycle sort mode and re-render
                    sort_mode = sort_mode.next();
                    sort_shows(&mut sorted_shows, sort_mode);
                    continue;
                }
                if is_queue_action(&label) {
                    if all_queued {
                        let ids: std::collections::HashSet<i64> =
                            sorted_shows.iter().map(|s| s.container_id).collect();
                        let removed = ids.iter().filter(|id| queue.contains_key(*id)).count();
                        queue.retain(|id, _| !ids.contains(id));
                        println!(
                            "\x1b[38;5;214mRemoved {removed} show{} from queue ({} total).\x1b[0m",
                            if removed != 1 { "s" } else { "" },
                            queue.len(),
                        );
                    } else {
                        let mut added = 0usize;
                        let mut skipped = 0usize;
                        for s in &sorted_shows {
                            if queue.contains_key(&s.container_id) {
                                skipped += 1;
                            } else {
                                queue.insert(s.container_id, s.clone());
                                added += 1;
                            }
                        }
                        println!(
                            "\x1b[38;5;113mAdded {added} show{} to queue{} \u{00b7} {} total.\x1b[0m",
                            if added != 1 { "s" } else { "" },
                            if skipped > 0 {
                                format!(" ({skipped} already queued)")
                            } else {
                                String::new()
                            },
                            queue.len(),
                        );
                    }
                    continue;
                }
                // Find matching show by position (offset by 2 for toggle + queue action)
                if let Some(pos) = labels.iter().position(|l| *l == label) {
                    if pos > 1 {
                        if let Some(catalog_show) = sorted_shows.get(pos - 2) {
                            show_detail(catalog_show, router, config, queue, None).await;
                        }
                    }
                }
            }
            PromptResult::Back | PromptResult::Interrupted => return,
        }
    }
}

/// Show detail panel with action menu.
///
/// If `prefetched_show` is provided, skips the API fetch (used by Import URL
/// which already has the Show data).
pub(crate) async fn show_detail(
    catalog_show: &CatalogShow,
    router: &mut ServiceRouter,
    config: &mut Config,
    queue: &mut IndexMap<i64, CatalogShow>,
    prefetched_show: Option<Show>,
) {
    let show = match prefetched_show {
        Some(s) => s,
        None => {
            clear_screen();
            println!();
            println!("  \x1b[2mFetching show details...\x1b[0m");

            match router
                .api_for(catalog_show.service)
                .get_show_detail(catalog_show.container_id)
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("  Failed to fetch show: {e}");
                    return;
                }
            }
        }
    };

    loop {
        clear_screen();
        print_show_panel(&show);

        let in_queue = queue.contains_key(&catalog_show.container_id);
        let mut choices = vec![
            if in_queue {
                "Remove from queue"
            } else {
                "Add to queue"
            }
            .to_string(),
            "Download now".to_string(),
        ];

        // Conditional queue download shortcut (matches Python behavior)
        match (in_queue, queue.is_empty()) {
            (true, _) => {
                let q_count = queue.len();
                choices.push(format!(
                    "Download queue ({q_count} show{})",
                    if q_count != 1 { "s" } else { "" }
                ));
            }
            (false, false) => {
                let q_count = queue.len() + 1;
                choices.push(format!(
                    "Add + download queue ({q_count} show{})",
                    if q_count != 1 { "s" } else { "" }
                ));
            }
            _ => {}
        }

        match styled_select("", choices) {
            PromptResult::Choice(choice) => {
                if choice == "Add to queue" {
                    queue.insert(catalog_show.container_id, catalog_show.clone());
                    println!(
                        "\x1b[38;5;113mAdded to queue ({} show{}).\x1b[0m",
                        queue.len(),
                        if queue.len() != 1 { "s" } else { "" }
                    );
                } else if choice == "Remove from queue" {
                    queue.shift_remove(&catalog_show.container_id);
                    println!(
                        "\x1b[38;5;214mRemoved from queue ({} show{}).\x1b[0m",
                        queue.len(),
                        if queue.len() != 1 { "s" } else { "" }
                    );
                } else if choice == "Download now" {
                    download_single(&show, catalog_show.service, router, config).await;
                } else if choice.starts_with("Download queue") {
                    download_queued_shows(queue, router, config).await;
                    return;
                } else if choice.starts_with("Add + download queue") {
                    queue.insert(catalog_show.container_id, catalog_show.clone());
                    download_queued_shows(queue, router, config).await;
                    return;
                }
            }
            PromptResult::Back | PromptResult::Interrupted => return,
        }
    }
}

/// Download a single show immediately.
async fn download_single(
    show: &Show,
    service: crate::service::Service,
    router: &mut ServiceRouter,
    config: &mut Config,
) {
    let codec = match prompt_postprocess(config) {
        Some(c) => c,
        None => return,
    };

    let output_dir = expand_tilde(&config.output_dir);
    if let Err(e) = std::fs::create_dir_all(&output_dir) {
        eprintln!("Failed to create output directory: {e}");
        return;
    }

    let format_code = match FormatCode::from_name(&config.format) {
        Some(fc) => fc,
        None => {
            eprintln!("Unknown format: {}", config.format);
            return;
        }
    };

    let api = router.api_for(service);
    let (tracks_with_urls, stats) = resolve_tracks(show, api, format_code).await;

    print_resolution_warnings(&stats, "");

    if tracks_with_urls.is_empty() {
        println!("\x1b[38;5;203mNo downloadable tracks found.\x1b[0m");
        return;
    }

    let completed =
        download_show(show, &tracks_with_urls, &output_dir, &codec, service, format_code).await;
    if completed {
        println!("\x1b[1;38;5;113mDownload complete!\x1b[0m");
    }
}

/// Print the show detail panel to stdout.
///
/// Mirrors Python's `_print_show_panel()`:
/// - Title: "Artist · Date"
/// - Venue on its own line
/// - Summary: location_short · sets · tracks · duration
/// - Track listing grouped by set with dot-leader alignment
fn print_show_panel(show: &Show) {
    // Leading blank line for consistent vertical positioning
    println!();

    // Title — bold amber, indented to match section headers
    println!(
        "  \x1b[1;38;5;214m{} {MIDDOT} {}\x1b[0m",
        show.artist_name,
        show.display_date()
    );

    // Venue
    if !show.venue_name.is_empty() {
        println!("  {}", show.venue_name);
    }

    // Summary line
    let loc_short = show.display_location_short();
    let sets = show.sets_grouped();
    let set_count = sets.len();
    let track_count = show.tracks.len();
    let total_duration: i64 = show.tracks.iter().map(|t| t.duration_seconds).sum();

    let mut summary_parts = Vec::new();
    if !loc_short.is_empty() {
        summary_parts.push(loc_short);
    }
    if set_count > 0 {
        summary_parts.push(format!(
            "{set_count} set{}",
            if set_count != 1 { "s" } else { "" }
        ));
    }
    summary_parts.push(format!(
        "{track_count} track{}",
        if track_count != 1 { "s" } else { "" }
    ));
    if total_duration > 0 {
        summary_parts.push(format_duration(total_duration));
    }

    println!("  {}", dim(&summary_parts.join(&format!(" {MIDDOT} "))));
    println!();

    // Track listing
    let title_lengths: Vec<usize> = show
        .tracks
        .iter()
        .map(|t| t.song_title.chars().count() + 5) // "01. " prefix + padding
        .collect();
    let col_width = dot_leader_col_width(&title_lengths);

    let max_normal_set = sets
        .keys()
        .filter(|&&sn| sn > 0)
        .copied()
        .max()
        .unwrap_or(0);

    // Iterate sets in correct order: regular sets first, then encores.
    // BTreeMap sorts set 0 before 1,2,3 which would render encores first.
    let is_encore = |sn: i64| -> bool { sn == 0 || (max_normal_set > 0 && sn > max_normal_set) };
    let all_set_zero = max_normal_set == 0 && sets.contains_key(&0);

    // Regular sets first (ascending), then encore sets
    let ordered_keys: Vec<i64> = {
        let mut regular: Vec<i64> = sets.keys().copied().filter(|&sn| !is_encore(sn)).collect();
        let mut encores: Vec<i64> = sets.keys().copied().filter(|&sn| is_encore(sn)).collect();
        regular.sort();
        encores.sort();
        regular.extend(encores);
        regular
    };

    for set_num in &ordered_keys {
        let tracks = &sets[set_num];
        // Set label: if all tracks are set 0, treat as "Set 1" not "Encore"
        let set_label = if all_set_zero {
            "Set 1".to_string()
        } else if is_encore(*set_num) {
            "Encore".to_string()
        } else {
            format!("Set {set_num}")
        };
        println!("  \x1b[38;5;214m{set_label}\x1b[0m");

        for track in tracks {
            let num = format!("{:02}.", track.track_num);
            let song = if track.song_title.is_empty() {
                "Unknown"
            } else {
                &track.song_title
            };
            let title = format!("{num} {song}");
            let dur = format_duration(track.duration_seconds);
            println!("    {}", dot_leader_line(&title, &dur, col_width));
        }
        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CatalogShow;
    use serde_json::json;

    #[test]
    fn test_encore_renders_after_regular_sets() {
        // Build a show with sets [0, 1, 2] — encore should come last
        let data = json!({
            "containerID": 1, "artistName": "Test", "containerInfo": "Info",
            "venueName": "Venue", "venueCity": "City", "venueState": "ST",
            "performanceDate": "2024-01-01", "performanceDateFormatted": "01/01/2024",
            "performanceDateYear": "2024",
            "tracks": [
                {"trackID": 1, "setNum": 0, "songTitle": "Encore Song", "trackNum": 7},
                {"trackID": 2, "setNum": 1, "songTitle": "Set 1 Song", "trackNum": 1},
                {"trackID": 3, "setNum": 2, "songTitle": "Set 2 Song", "trackNum": 4}
            ]
        });
        let show = Show::from_json(&data);
        let sets = show.sets_grouped();

        let max_normal_set = sets
            .keys()
            .filter(|&&sn| sn > 0)
            .copied()
            .max()
            .unwrap_or(0);
        assert_eq!(max_normal_set, 2);

        let is_encore = |sn: i64| sn == 0 || (max_normal_set > 0 && sn > max_normal_set);

        let mut regular: Vec<i64> = sets.keys().copied().filter(|&sn| !is_encore(sn)).collect();
        let mut encores: Vec<i64> = sets.keys().copied().filter(|&sn| is_encore(sn)).collect();
        regular.sort();
        encores.sort();
        regular.extend(encores);

        // Regular sets first, then encore
        assert_eq!(regular, vec![1, 2, 0]);
    }

    fn make_catalog_show(date: &str, venue: &str) -> CatalogShow {
        CatalogShow {
            container_id: 1,
            artist_id: 1,
            artist_name: "Test".to_string(),
            container_info: String::new(),
            venue_name: venue.to_string(),
            venue_city: String::new(),
            venue_state: String::new(),
            performance_date: date.to_string(),
            performance_date_formatted: String::new(),
            performance_date_year: String::new(),
            song_list: String::new(),
            image_url: String::new(),
            service: Default::default(),
        }
    }

    #[test]
    fn test_sort_shows_newest() {
        let mut shows = vec![
            make_catalog_show("2023-01-01", "A Venue"),
            make_catalog_show("2024-06-15", "B Venue"),
            make_catalog_show("2023-07-04", "C Venue"),
        ];
        sort_shows(&mut shows, SortMode::Newest);
        assert_eq!(shows[0].performance_date, "2024-06-15");
        assert_eq!(shows[1].performance_date, "2023-07-04");
        assert_eq!(shows[2].performance_date, "2023-01-01");
    }

    #[test]
    fn test_sort_shows_oldest() {
        let mut shows = vec![
            make_catalog_show("2024-06-15", "B Venue"),
            make_catalog_show("2023-01-01", "A Venue"),
        ];
        sort_shows(&mut shows, SortMode::Oldest);
        assert_eq!(shows[0].performance_date, "2023-01-01");
        assert_eq!(shows[1].performance_date, "2024-06-15");
    }

    #[test]
    fn test_sort_shows_venue() {
        let mut shows = vec![
            make_catalog_show("2024-01-01", "Madison Square Garden"),
            make_catalog_show("2024-02-01", "Alpine Valley"),
            make_catalog_show("2024-03-01", "Red Rocks"),
        ];
        sort_shows(&mut shows, SortMode::Venue);
        assert_eq!(shows[0].venue_name, "Alpine Valley");
        assert_eq!(shows[1].venue_name, "Madison Square Garden");
        assert_eq!(shows[2].venue_name, "Red Rocks");
    }

    #[test]
    fn test_sort_mode_cycle() {
        assert_eq!(SortMode::Newest.next(), SortMode::Oldest);
        assert_eq!(SortMode::Oldest.next(), SortMode::Venue);
        assert_eq!(SortMode::Venue.next(), SortMode::Newest);
    }

    fn make_catalog_show_with_id(id: i64, date: &str, venue: &str) -> CatalogShow {
        CatalogShow {
            container_id: id,
            artist_id: 1,
            artist_name: "Test".to_string(),
            container_info: String::new(),
            venue_name: venue.to_string(),
            venue_city: String::new(),
            venue_state: String::new(),
            performance_date: date.to_string(),
            performance_date_formatted: String::new(),
            performance_date_year: String::new(),
            song_list: String::new(),
            image_url: String::new(),
            service: Default::default(),
        }
    }

    #[test]
    fn test_bulk_add_empty_queue() {
        let shows = vec![
            make_catalog_show_with_id(100, "2024-01-01", "Red Rocks"),
            make_catalog_show_with_id(200, "2024-02-01", "MSG"),
            make_catalog_show_with_id(300, "2024-03-01", "Alpine"),
        ];
        let mut queue: IndexMap<i64, CatalogShow> = IndexMap::new();

        // Simulate bulk add: insert all shows not already in queue
        for s in &shows {
            if !queue.contains_key(&s.container_id) {
                queue.insert(s.container_id, s.clone());
            }
        }

        assert_eq!(queue.len(), 3);
        assert!(queue.contains_key(&100));
        assert!(queue.contains_key(&200));
        assert!(queue.contains_key(&300));
    }

    #[test]
    fn test_bulk_add_partial_queue() {
        let shows = vec![
            make_catalog_show_with_id(100, "2024-01-01", "Red Rocks"),
            make_catalog_show_with_id(200, "2024-02-01", "MSG"),
            make_catalog_show_with_id(300, "2024-03-01", "Alpine"),
        ];
        let mut queue: IndexMap<i64, CatalogShow> = IndexMap::new();
        // Pre-queue show 200
        queue.insert(200, shows[1].clone());

        let mut added = 0usize;
        let mut skipped = 0usize;
        for s in &shows {
            if queue.contains_key(&s.container_id) {
                skipped += 1;
            } else {
                queue.insert(s.container_id, s.clone());
                added += 1;
            }
        }

        assert_eq!(added, 2);
        assert_eq!(skipped, 1);
        assert_eq!(queue.len(), 3);
        assert!(queue.contains_key(&100));
        assert!(queue.contains_key(&200));
        assert!(queue.contains_key(&300));
    }

    #[test]
    fn test_bulk_remove_all_queued() {
        let shows = vec![
            make_catalog_show_with_id(100, "2024-01-01", "Red Rocks"),
            make_catalog_show_with_id(200, "2024-02-01", "MSG"),
        ];
        let mut queue: IndexMap<i64, CatalogShow> = IndexMap::new();
        // Queue all shows + one extra from a different context
        for s in &shows {
            queue.insert(s.container_id, s.clone());
        }
        queue.insert(
            999,
            make_catalog_show_with_id(999, "2024-12-31", "Other Venue"),
        );
        assert_eq!(queue.len(), 3);

        // Simulate bulk remove: retain only IDs not in the current show list
        let ids: std::collections::HashSet<i64> = shows.iter().map(|s| s.container_id).collect();
        queue.retain(|id, _| !ids.contains(id));

        assert_eq!(queue.len(), 1);
        assert!(!queue.contains_key(&100));
        assert!(!queue.contains_key(&200));
        assert!(queue.contains_key(&999)); // Other context preserved
    }

    #[test]
    fn test_all_set_zero_treated_as_set_1() {
        let data = json!({
            "containerID": 1, "artistName": "Test", "containerInfo": "Info",
            "venueName": "V", "venueCity": "C", "venueState": "S",
            "performanceDate": "2024-01-01", "performanceDateFormatted": "",
            "performanceDateYear": "2024",
            "tracks": [
                {"trackID": 1, "setNum": 0, "songTitle": "A", "trackNum": 1},
                {"trackID": 2, "setNum": 0, "songTitle": "B", "trackNum": 2}
            ]
        });
        let show = Show::from_json(&data);
        let sets = show.sets_grouped();
        let max_normal_set = sets
            .keys()
            .filter(|&&sn| sn > 0)
            .copied()
            .max()
            .unwrap_or(0);

        // max_normal_set is 0, all tracks set 0 → should be treated as "Set 1"
        assert_eq!(max_normal_set, 0);
        assert!(sets.contains_key(&0));
    }
}
