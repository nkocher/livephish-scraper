use indexmap::IndexMap;

use crate::catalog::Catalog;
use crate::config::Config;
use crate::models::CatalogShow;
use crate::service::router::ServiceRouter;

use super::artist::{browse_by_artist, recents};
use super::import::import_url;
use super::prompt::{styled_fuzzy, styled_select, PromptResult};
use super::queue::manage_queue;
use super::search::search_shows;
use super::settings::edit_settings;
use super::show::show_list;
use super::style::{clear_screen, print_banner, print_section, section_header};

/// Actions available from the main menu.
#[derive(Debug, Clone, PartialEq)]
enum MainAction {
    Recents,
    AllArtists,
    BrowseByYear,
    SearchShows,
    ImportUrl,
    DownloadQueue,
    Settings,
    RefreshCatalog,
    Quit,
}

/// Display the main menu and return the chosen action.
fn main_menu(queue_count: usize) -> Option<MainAction> {
    clear_screen();
    print_banner(queue_count);

    let queue_label = if queue_count > 0 {
        format!("Download queue ({queue_count})")
    } else {
        "Download queue".to_string()
    };

    let choices = vec![
        section_header("Browse"),
        "Recents".to_string(),
        "All artists".to_string(),
        "Browse by year".to_string(),
        "Search shows".to_string(),
        section_header("Actions"),
        "Import URL".to_string(),
        queue_label.clone(),
        section_header("System"),
        "Settings".to_string(),
        "Refresh catalog".to_string(),
        "Quit".to_string(),
    ];

    match styled_select("", choices) {
        PromptResult::Choice(ref choice) => match choice.as_str() {
            "Recents" => Some(MainAction::Recents),
            "All artists" => Some(MainAction::AllArtists),
            "Browse by year" => Some(MainAction::BrowseByYear),
            "Search shows" => Some(MainAction::SearchShows),
            "Import URL" => Some(MainAction::ImportUrl),
            s if s.starts_with("Download queue") => Some(MainAction::DownloadQueue),
            "Settings" => Some(MainAction::Settings),
            "Refresh catalog" => Some(MainAction::RefreshCatalog),
            "Quit" => Some(MainAction::Quit),
            _ => None,
        },
        PromptResult::Back | PromptResult::Interrupted => Some(MainAction::Quit),
    }
}

/// Main browser loop — runs until user quits.
pub async fn run_browser(catalog: &mut Catalog, router: &mut ServiceRouter, config: &mut Config) {
    let mut queue: IndexMap<i64, CatalogShow> = IndexMap::new();

    loop {
        let action = match main_menu(queue.len()) {
            Some(a) => a,
            None => continue,
        };

        match action {
            MainAction::Recents => {
                recents(catalog, router, config, &mut queue).await;
            }
            MainAction::AllArtists => {
                browse_by_artist(catalog, router, config, &mut queue).await;
            }
            MainAction::BrowseByYear => {
                browse_by_year(catalog, router, config, &mut queue).await;
            }
            MainAction::SearchShows => {
                search_shows(catalog, router, config, &mut queue).await;
            }
            MainAction::ImportUrl => {
                import_url(catalog, router, config, &mut queue).await;
            }
            MainAction::DownloadQueue => {
                manage_queue(&mut queue, router, config).await;
            }
            MainAction::Settings => {
                edit_settings(config);
            }
            MainAction::RefreshCatalog => {
                println!("Refreshing catalog...");
                catalog.refresh(router).await;
                println!("Catalog refreshed.");
                pause();
            }
            MainAction::Quit => break,
        }
    }
}

/// Browse shows by year — year list with counts, then cross-artist show list.
async fn browse_by_year(
    catalog: &mut Catalog,
    router: &mut ServiceRouter,
    config: &mut Config,
    queue: &mut IndexMap<i64, CatalogShow>,
) {
    loop {
        clear_screen();
        let year_counts = catalog.year_show_counts();
        if year_counts.is_empty() {
            println!("No shows loaded. Try browsing an artist first.");
            pause();
            return;
        }

        print_section(
            "Browse by Year",
            Some(&format!(
                "{} year{}",
                year_counts.len(),
                if year_counts.len() != 1 { "s" } else { "" }
            )),
        );

        let labels: Vec<String> = year_counts
            .iter()
            .map(|(year, count)| {
                format!(
                    "{year} ({count} show{})",
                    if *count != 1 { "s" } else { "" }
                )
            })
            .collect();

        match styled_fuzzy("Select year:", labels.clone()) {
            PromptResult::Choice(label) => {
                if let Some(pos) = labels.iter().position(|l| *l == label) {
                    let year = &year_counts[pos].0;
                    let shows = catalog.get_shows_by_year(year);
                    show_list(&shows, router, config, queue, year).await;
                }
            }
            PromptResult::Back | PromptResult::Interrupted => return,
        }
    }
}

/// Pause for user to read a message before returning to menu.
fn pause() {
    use std::io::{self, Read};
    println!("\nPress Enter to continue...");
    let _ = io::stdin().read(&mut [0u8]);
}
