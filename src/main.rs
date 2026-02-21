mod api;
mod browser;
mod catalog;
mod config;
mod download;
mod models;
mod service;
mod tagger;
mod transcode;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use tracing::info;

use crate::api::NugsApi;
use crate::browser::menu::run_browser;
use crate::browser::resolve::{
    print_resolution_warnings, resolve_tracks, retry_resolve_with_refresh,
};
use crate::catalog::Catalog;
use crate::config::credentials::{get_credentials, get_credentials_for_service};
use crate::config::{expand_tilde, load_config};
use crate::download::download_show;
use crate::models::show::DisplayLocation;
use crate::models::FormatCode;
use crate::service::router::ServiceRouter;
use crate::service::Service;

#[derive(Parser)]
#[command(
    name = "nugs",
    version,
    about = "Browse and download music from nugs.net"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Force fresh authentication (bypass session cache)
    #[arg(short, long)]
    force_login: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Configure credentials
    Config,
    /// Force-refresh catalog cache
    Refresh,
    /// Download a show by container ID (vertical slice)
    Download {
        /// Container ID of the show to download
        container_id: i64,

        /// Audio format (alac, flac, mqa, 360, aac)
        #[arg(short = 'F', long, default_value = "flac")]
        format: String,

        /// Output directory
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Download all shows for an artist
    DownloadAll {
        /// Artist name or numeric ID
        artist: String,

        /// Audio format (alac, flac, mqa, 360, aac)
        #[arg(short = 'F', long, default_value = "flac")]
        format: String,

        /// Output directory
        #[arg(short, long)]
        output: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Config) => {
            run_config()?;
        }
        Some(Commands::Refresh) => {
            run_refresh().await?;
        }
        Some(Commands::Download {
            container_id,
            format,
            output,
        }) => {
            run_download(container_id, &format, output.as_deref(), cli.force_login).await?;
        }
        Some(Commands::DownloadAll {
            artist,
            format,
            output,
        }) => {
            run_download_all(&artist, &format, output.as_deref(), cli.force_login).await?;
        }
        None => {
            run_interactive_browser(cli.force_login).await?;
        }
    }

    Ok(())
}

/// Authenticate with fresh or cached credentials.
async fn login(
    api: &mut NugsApi,
    email: &str,
    password: &str,
    force: bool,
) -> Result<(crate::models::StreamParams, String), crate::api::error::AuthError> {
    if force {
        api.login(email, password).await
    } else {
        api.login_cached(email, password).await
    }
}

/// Launch the interactive browser TUI.
async fn run_interactive_browser(force_login: bool) -> Result<()> {
    let mut config = load_config();

    // Get credentials for the nugs service
    let (email, password) =
        get_credentials(config.email_for(Service::Nugs)).map_err(|e| anyhow::anyhow!(e))?;

    let mut nugs_api = NugsApi::new();
    let (_stream_params, status) = login(&mut nugs_api, &email, &password, force_login)
        .await
        .context("Authentication failed")?;
    info!("Logged in ({status})");

    // Optionally authenticate LivePhish
    let livephish_api = try_login_livephish(&config, force_login).await;

    let mut router = ServiceRouter {
        nugs: nugs_api,
        livephish: livephish_api,
    };

    // Initialize catalog (load cached artist registry + show data from disk)
    let mut catalog = Catalog::new(crate::config::paths::cache_dir());
    catalog.load(router.has_livephish());

    // Run the browser loop
    run_browser(&mut catalog, &mut router, &mut config).await;

    Ok(())
}

/// Try to authenticate LivePhish. Returns `Some(api)` on success, `None` on
/// any failure (missing credentials, auth error, etc.). Never blocks startup.
async fn try_login_livephish(config: &crate::config::Config, force_login: bool) -> Option<NugsApi> {
    let (email, password) = match get_credentials_for_service(config, Service::LivePhish) {
        Ok(creds) => creds,
        Err(_) => return None,
    };

    let mut api = NugsApi::new_for_service(Service::LivePhish);
    match login(&mut api, &email, &password, force_login).await {
        Ok((_sp, status)) => {
            info!("LivePhish logged in ({status})");
            Some(api)
        }
        Err(e) => {
            tracing::warn!("LivePhish login failed (will use nugs.net for Phish): {e}");
            None
        }
    }
}

/// Configure credentials: prompt for email + password for each service, save to keyring + config.
fn run_config() -> Result<()> {
    use crate::config::credentials::{set_keyring_password, set_keyring_password_for};
    use crate::config::{save_config, ServiceSection};

    let mut config = load_config();

    // === nugs.net ===
    println!("=== nugs.net ===");

    let nugs_hint = if config.nugs.email.is_empty() {
        String::new()
    } else {
        format!(" [{}]", config.nugs.email)
    };
    print!("Email{nugs_hint}: ");
    std::io::Write::flush(&mut std::io::stdout())?;

    let mut nugs_email_input = String::new();
    std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut nugs_email_input)?;
    let nugs_email_trimmed = nugs_email_input.trim();

    let nugs_email = if nugs_email_trimmed.is_empty() && !config.nugs.email.is_empty() {
        config.nugs.email.clone()
    } else if nugs_email_trimmed.is_empty() {
        bail!("nugs.net email is required.");
    } else {
        nugs_email_trimmed.to_string()
    };

    let nugs_password =
        rpassword::prompt_password("Password: ").context("Failed to read password")?;
    if nugs_password.is_empty() {
        bail!("nugs.net password is required.");
    }

    // === LivePhish (optional) ===
    println!();
    println!("=== LivePhish (optional, press Enter to skip) ===");

    let lp_hint = if config.livephish.email.is_empty() {
        String::new()
    } else {
        format!(" [{}]", config.livephish.email)
    };
    print!("Email{lp_hint}: ");
    std::io::Write::flush(&mut std::io::stdout())?;

    let mut lp_email_input = String::new();
    std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut lp_email_input)?;
    let lp_email_trimmed = lp_email_input.trim();

    // Resolve LivePhish email (empty input keeps the existing value, if any)
    let lp_email = if lp_email_trimmed.is_empty() && !config.livephish.email.is_empty() {
        config.livephish.email.clone()
    } else {
        lp_email_trimmed.to_string()
    };

    // Only prompt for LivePhish password when an email was provided
    let lp_password = if lp_email.is_empty() {
        String::new()
    } else {
        rpassword::prompt_password("Password: ").context("Failed to read password")?
    };

    // Persist nugs credentials
    config.nugs = ServiceSection {
        email: nugs_email.clone(),
    };
    // Persist LivePhish email (password stored in keyring separately)
    config.livephish = ServiceSection {
        email: lp_email.clone(),
    };
    save_config(&config);
    println!("Email(s) saved to config.");

    if set_keyring_password(&nugs_email, &nugs_password) {
        println!("nugs.net password saved to system keyring.");
    } else {
        println!(
            "Warning: Could not save nugs.net password to keyring. You'll be prompted each time."
        );
    }

    if !lp_email.is_empty() && !lp_password.is_empty() {
        let lp_keyring = Service::LivePhish.config().keyring_service;
        if set_keyring_password_for(&lp_email, &lp_password, lp_keyring) {
            println!("LivePhish password saved to system keyring.");
        } else {
            println!("Warning: Could not save LivePhish password to keyring. You'll be prompted each time.");
        }
    } else if lp_email.is_empty() {
        println!("LivePhish skipped.");
    }

    Ok(())
}

/// Force-refresh the catalog cache.
async fn run_refresh() -> Result<()> {
    let config = load_config();

    let (email, password) =
        get_credentials(config.email_for(Service::Nugs)).map_err(|e| anyhow::anyhow!(e))?;

    let mut nugs_api = NugsApi::new();
    let (_stream_params, status) = nugs_api
        .login_cached(&email, &password)
        .await
        .context("Authentication failed")?;
    println!("Logged in ({status})");

    let livephish_api = try_login_livephish(&config, false).await;

    let mut router = ServiceRouter {
        nugs: nugs_api,
        livephish: livephish_api,
    };

    let mut catalog = Catalog::new(crate::config::paths::cache_dir());
    catalog.load(router.has_livephish());

    println!("Refreshing catalog...");
    catalog.refresh(&mut router).await;
    println!("Catalog refreshed.");

    Ok(())
}

/// Vertical slice: download a show by container ID.
async fn run_download(
    container_id: i64,
    format_name: &str,
    output_override: Option<&str>,
    force_login: bool,
) -> Result<()> {
    let cfg = load_config();

    // Resolve output directory
    let output_dir = expand_tilde(output_override.unwrap_or(&cfg.output_dir));

    // Get credentials for nugs service
    let (email, password) =
        get_credentials(cfg.email_for(Service::Nugs)).map_err(|e| anyhow::anyhow!(e))?;

    let mut api = NugsApi::new();
    let (_stream_params, status) = login(&mut api, &email, &password, force_login)
        .await
        .context("Authentication failed")?;
    println!("Logged in ({status})");

    // Fetch show detail
    println!("Fetching show {container_id}...");
    let show = api
        .get_show_detail(container_id)
        .await
        .context("Failed to fetch show detail")?;

    println!(
        "{} — {} ({} tracks)",
        show.artist_name,
        show.container_info,
        show.tracks.len()
    );

    if show.tracks.is_empty() {
        bail!("Show has no tracks");
    }

    // Resolve format code
    let format_code = FormatCode::from_name(format_name)
        .with_context(|| format!("Unknown format: {format_name}"))?;

    // Resolve stream URLs for all tracks with format fallback
    let (mut tracks_with_urls, mut stats) = resolve_tracks(&show, &mut api, format_code).await;

    // Retry with session refresh if all tracks failed and some had no URL
    if tracks_with_urls.is_empty() && stats.no_stream_url > 0 {
        let (retry_tracks, retry_stats) =
            retry_resolve_with_refresh(&show, &mut api, format_code, "").await;
        tracks_with_urls = retry_tracks;
        stats = retry_stats;
    }

    print_resolution_warnings(&stats, "");

    if tracks_with_urls.is_empty() {
        bail!("Could not resolve any stream URLs");
    }

    println!(
        "Resolved {}/{} tracks, downloading to {}",
        tracks_with_urls.len(),
        show.tracks.len(),
        output_dir.display()
    );

    // Download all tracks (CLI download is always nugs.net)
    let completed = download_show(
        &show,
        &tracks_with_urls,
        &output_dir,
        &cfg.postprocess_codec,
        Service::Nugs,
    )
    .await;

    if completed {
        println!("Done!");
    } else {
        println!("Download cancelled.");
    }

    Ok(())
}

/// Download all shows for an artist by name or numeric ID.
async fn run_download_all(
    artist_input: &str,
    format_name: &str,
    output_override: Option<&str>,
    force_login: bool,
) -> Result<()> {
    use crate::catalog::ArtistTarget;

    let cfg = load_config();
    let output_dir = expand_tilde(output_override.unwrap_or(&cfg.output_dir));
    let format_code = FormatCode::from_name(format_name)
        .with_context(|| format!("Unknown format: {format_name}"))?;

    // Authenticate
    let (email, password) =
        get_credentials(cfg.email_for(Service::Nugs)).map_err(|e| anyhow::anyhow!(e))?;

    let mut nugs_api = NugsApi::new();
    let (_stream_params, status) = login(&mut nugs_api, &email, &password, force_login)
        .await
        .context("Authentication failed")?;
    println!("Logged in ({status})");

    let livephish_api = try_login_livephish(&cfg, force_login).await;

    let mut router = ServiceRouter {
        nugs: nugs_api,
        livephish: livephish_api,
    };

    let mut catalog = Catalog::new(crate::config::paths::cache_dir());
    catalog.load(router.has_livephish());

    // Resolve artist target — numeric ID or name
    let target = match artist_input.parse::<i64>() {
        Ok(id) => ArtistTarget::Id(id),
        Err(_) => ArtistTarget::Name(artist_input.to_string()),
    };

    // Load artist catalog
    println!("Loading artist catalog...");
    let artist_id = catalog
        .load_artist(&mut router, target, true)
        .await
        .context("Could not find or load artist")?;

    let shows = catalog.get_shows_by_artist_id(artist_id);
    if shows.is_empty() {
        bail!("No shows found for this artist");
    }

    let artist_name = shows[0].artist_name.clone();
    let total = shows.len();
    println!("{artist_name}: {total} shows");

    // Collect shows into owned vec (avoid borrow issues during iteration)
    let shows_owned: Vec<crate::models::CatalogShow> = shows;

    let mut downloaded = 0usize;
    let mut skipped = 0usize;

    for (i, catalog_show) in shows_owned.iter().enumerate() {
        let d = catalog_show.display_date();
        let date = if d.is_empty() { "Unknown date" } else { d };

        println!(
            "\n\x1b[1;38;5;214m[{}/{}]\x1b[0m {date} \u{00b7} {}",
            i + 1,
            total,
            catalog_show.venue_name,
        );

        // Fetch full show detail
        let show = match router
            .api_for(catalog_show.service)
            .get_show_detail(catalog_show.container_id)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                println!("  \x1b[31mFailed to fetch show: {e}\x1b[0m");
                skipped += 1;
                continue;
            }
        };

        // Resolve tracks
        let api = router.api_for(catalog_show.service);
        let (mut tracks_with_urls, mut stats) = resolve_tracks(&show, api, format_code).await;

        if tracks_with_urls.is_empty() && stats.no_stream_url > 0 {
            let api = router.api_for(catalog_show.service);
            let (retry_tracks, retry_stats) =
                retry_resolve_with_refresh(&show, api, format_code, "  ").await;
            tracks_with_urls = retry_tracks;
            stats = retry_stats;
        }

        print_resolution_warnings(&stats, "  ");

        if tracks_with_urls.is_empty() {
            println!("  \x1b[38;5;214mNo downloadable tracks.\x1b[0m");
            skipped += 1;
            continue;
        }

        let completed = download_show(
            &show,
            &tracks_with_urls,
            &output_dir,
            &cfg.postprocess_codec,
            catalog_show.service,
        )
        .await;

        if completed {
            downloaded += 1;
        } else {
            // User cancelled
            break;
        }
    }

    println!("\n{downloaded}/{total} shows downloaded");
    if skipped > 0 {
        println!("{skipped} skipped");
    }

    Ok(())
}
