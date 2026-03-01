mod api;
mod bman;
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
use crate::browser::resolve::{print_resolution_warnings, resolve_tracks};
use crate::catalog::Catalog;
use crate::config::credentials::{get_credentials, get_credentials_for_service};
use crate::config::{expand_tilde, load_config};
use crate::download::{download_show, download_show_with_retry};
use crate::models::show::DisplayLocation;
use crate::models::FormatCode;
use crate::service::router::ServiceRouter;
use crate::service::Service;

const VALID_FLAC_CONVERT: &[&str] = &["none", "alac", "aac"];

/// Format an API key as a masked hint string for config prompts.
/// Returns ` [abcd...wxyz]` for keys >8 chars, ` [****]` for shorter keys,
/// or empty string if the key is empty.
fn mask_api_key(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    let masked = if key.len() > 8 {
        format!("{}...{}", &key[..4], &key[key.len() - 4..])
    } else {
        "****".to_string()
    };
    format!(" [{masked}]")
}

/// Validate --flac-convert CLI override. Returns the value or a default.
fn resolve_flac_convert<'a>(
    cli_override: Option<&'a str>,
    config_default: &'a str,
) -> Result<&'a str> {
    match cli_override {
        Some(v) if VALID_FLAC_CONVERT.contains(&v) => Ok(v),
        Some(v) => bail!("Invalid --flac-convert value: {v} (must be none, alac, or aac)"),
        None => Ok(config_default),
    }
}

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
    Refresh {
        /// Clear setlist.fm response cache before enrichment.
        /// Values: (no arg) = clear all, "notfound" = re-query 404s only,
        /// or a date like "1977-05-08" to clear entries matching that date.
        #[arg(long, value_name = "FILTER")]
        clear_sfm_cache: Option<Option<String>>,
    },
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

        /// FLAC conversion: none, alac, aac (overrides config)
        #[arg(long)]
        flac_convert: Option<String>,
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

        /// FLAC conversion: none, alac, aac (overrides config)
        #[arg(long)]
        flac_convert: Option<String>,
    },
    /// Download all Bman shows for a year (Grateful Dead / JGB)
    DownloadYear {
        /// 4-digit year (e.g. 1977)
        year: String,

        /// Output directory
        #[arg(short, long)]
        output: Option<String>,

        /// FLAC conversion: none, alac, aac (overrides config; default aac for Bman)
        #[arg(long)]
        flac_convert: Option<String>,
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
        Some(Commands::Refresh { clear_sfm_cache }) => {
            run_refresh(clear_sfm_cache).await?;
        }
        Some(Commands::Download {
            container_id,
            format,
            output,
            flac_convert,
        }) => {
            run_download(container_id, &format, output.as_deref(), flac_convert.as_deref(), cli.force_login).await?;
        }
        Some(Commands::DownloadAll {
            artist,
            format,
            output,
            flac_convert,
        }) => {
            run_download_all(&artist, &format, output.as_deref(), flac_convert.as_deref(), cli.force_login).await?;
        }
        Some(Commands::DownloadYear {
            year,
            output,
            flac_convert,
        }) => {
            run_download_year(&year, output.as_deref(), flac_convert.as_deref()).await?;
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
        bman: try_init_bman(&config),
    };

    // Initialize catalog (load cached artist registry + show data from disk)
    let mut catalog = Catalog::new(crate::config::paths::cache_dir());
    catalog.set_setlistfm_api_key(&config.bman.setlistfm_api_key);
    catalog.load(router.has_livephish());

    // Run the browser loop
    run_browser(&mut catalog, &mut router, &mut config).await;

    Ok(())
}

/// Initialize BmanApi if GOOGLE_API_KEY is available. Returns None if no key.
fn try_init_bman(config: &crate::config::Config) -> Option<bman::BmanApi> {
    if config.bman.google_api_key.is_empty() {
        return None;
    }
    let mut api = bman::BmanApi::new(config.bman.google_api_key.clone());
    let cache_dir = crate::config::paths::cache_dir();
    // Load cached ID map if available
    if let Some(id_map) = crate::catalog::cache::load_bman_id_map(&cache_dir) {
        api.id_map = id_map;
    }
    // Load cached artwork index if available
    if let Some(idx) = bman::artwork::load_artwork_index(&cache_dir) {
        api.artwork_index = idx;
    }

    // OAuth: try config fields first, then auto-import from rclone
    let oauth_creds = if config.bman.has_oauth_config() {
        Some((
            config.bman.google_client_id.clone(),
            config.bman.google_client_secret.clone(),
            config.bman.google_refresh_token.clone(),
        ))
    } else {
        bman::rclone::load_rclone_credentials().map(|c| {
            info!("Loaded Google OAuth from rclone config");
            (c.client_id, c.client_secret, c.refresh_token)
        })
    };

    if let Some((id, secret, refresh)) = oauth_creds {
        api.set_oauth(id, secret, refresh);
        info!("Bman enabled (Google Drive archive, OAuth)");
    } else {
        info!("Bman enabled (Google Drive archive, API key only)");
    }
    Some(api)
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

    // === Bman / Google Drive (optional) ===
    println!();
    println!("=== Bman / Google Drive (optional, press Enter to skip) ===");

    let gapi_hint = mask_api_key(&config.bman.google_api_key);
    print!("Google API Key{gapi_hint}: ");
    std::io::Write::flush(&mut std::io::stdout())?;

    let mut gapi_input = String::new();
    std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut gapi_input)?;
    let gapi_trimmed = gapi_input.trim();

    if !gapi_trimmed.is_empty() {
        config.bman.google_api_key = gapi_trimmed.to_string();
    }

    let sfm_key_count = bman::setlistfm::SetlistFmKeys::from_comma_separated(&config.bman.setlistfm_api_key).len();
    let sfm_hint = if sfm_key_count > 1 {
        format!(" [{sfm_key_count} keys]")
    } else {
        mask_api_key(&config.bman.setlistfm_api_key)
    };
    print!("Setlist.fm API Key(s){sfm_hint}: ");
    std::io::Write::flush(&mut std::io::stdout())?;

    let mut sfm_input = String::new();
    std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut sfm_input)?;
    let sfm_trimmed = sfm_input.trim();

    if !sfm_trimmed.is_empty() {
        config.bman.setlistfm_api_key = sfm_trimmed.to_string();
    }

    save_config(&config);

    if !config.bman.google_api_key.is_empty() {
        println!("Bman API keys saved to config.");
    } else {
        println!("Bman skipped.");
    }

    Ok(())
}

/// Force-refresh the catalog cache.
async fn run_refresh(clear_sfm_cache: Option<Option<String>>) -> Result<()> {
    let mut config = load_config();
    config.normalize();

    // Apply --clear-sfm-cache before enrichment
    if let Some(filter) = &clear_sfm_cache {
        let cache_dir = crate::config::paths::cache_dir();
        let mut sfm_cache = bman::setlistfm::SfmCache::load(&cache_dir);
        let before = sfm_cache.len();
        match filter.as_deref() {
            None | Some("") => {
                // Clear all
                sfm_cache.clear_matching(|_| true);
                println!("Cleared all {} setlist.fm cache entries.", before);
            }
            Some("notfound") => {
                sfm_cache.clear_not_found();
                let after = sfm_cache.len();
                println!("Cleared {} NotFound entries from setlist.fm cache.", before - after);
            }
            Some(date_filter) => {
                // Cache keys use DD-MM-YYYY format; convert ISO dates for matching
                let filter = bman::setlistfm::convert_date(date_filter)
                    .unwrap_or_else(|_| date_filter.to_string());
                sfm_cache.clear_matching(|key| key.contains(&filter));
                let after = sfm_cache.len();
                println!(
                    "Cleared {} setlist.fm cache entries matching '{}'.",
                    before - after,
                    date_filter
                );
            }
        }
        sfm_cache.save();
    }

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
        bman: try_init_bman(&config),
    };

    let mut catalog = Catalog::new(crate::config::paths::cache_dir());
    catalog.set_setlistfm_api_key(&config.bman.setlistfm_api_key);
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
    flac_convert_override: Option<&str>,
    force_login: bool,
) -> Result<()> {
    let mut cfg = load_config();
    cfg.normalize();

    // Resolve output directory
    let output_dir = expand_tilde(output_override.unwrap_or(&cfg.output_dir));

    // Bman shows have negative container IDs
    if container_id < 0 {
        return run_download_bman(container_id, &output_dir, flac_convert_override, &cfg).await;
    }

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
    let (tracks_with_urls, stats) = resolve_tracks(&show, &mut api, format_code).await;

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
    let flac_convert = resolve_flac_convert(flac_convert_override, &cfg.flac_convert)?;
    let outcome = download_show(
        &show,
        &tracks_with_urls,
        &output_dir,
        &cfg.postprocess_codec,
        flac_convert,
        Service::Nugs,
        format_code,
        None,
    )
    .await;

    if outcome.completed {
        println!("Done!");
    } else {
        println!("Download cancelled.");
    }

    Ok(())
}

/// Download a single Bman show by negative container ID.
async fn run_download_bman(
    container_id: i64,
    output_dir: &std::path::Path,
    flac_convert_override: Option<&str>,
    cfg: &crate::config::Config,
) -> Result<()> {
    let mut bman = try_init_bman(cfg)
        .context("GOOGLE_API_KEY not set — run: export GOOGLE_API_KEY=<your key>")?;

    // Build a minimal CatalogShow from the ID map
    let _folder_id = bman
        .id_map
        .get_drive_id(container_id)
        .context("Unknown Bman container ID — try refreshing the catalog")?;

    // Load catalog to find show metadata
    let catalog_shows = crate::catalog::cache::load_bman_cache(&crate::config::paths::cache_dir())
        .unwrap_or_default();
    let catalog_show = catalog_shows
        .iter()
        .find(|s| s.container_id == container_id)
        .cloned()
        .unwrap_or_else(|| crate::models::CatalogShow {
            container_id,
            artist_name: "Grateful Dead".to_string(),
            container_info: format!("Bman Show {container_id}"),
            service: Service::Bman,
            ..Default::default()
        });

    println!("Fetching Bman show {container_id}...");
    let cache_dir = crate::config::paths::cache_dir();
    let mut sfm_cache = bman::setlistfm::SfmCache::load(&cache_dir);
    let mut show = bman::download::fetch_bman_show_detail(&mut bman, &catalog_show, &sfm_cache)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch Bman show: {e}"))?;

    println!(
        "{} — {} ({} tracks)",
        show.artist_name,
        show.display_location(),
        show.tracks.len()
    );

    if show.tracks.is_empty() {
        bail!("Show has no tracks");
    }

    let (mut tracks_with_urls, bearer) = bman::download::resolve_bman_tracks(&show, &bman).await;

    if tracks_with_urls.is_empty() {
        bail!("Could not resolve any download URLs");
    }

    // Enrich metadata before download (setlist.fm titles used for tagging)
    let sfm_keys = bman::setlistfm::SetlistFmKeys::from_comma_separated(&cfg.bman.setlistfm_api_key);
    bman::download::bman_enrich_metadata(&mut show, output_dir, &sfm_keys, &mut sfm_cache)
        .await;
    sfm_cache.save();
    bman::download::sync_enriched_titles(&show, &mut tracks_with_urls);

    let bman_default = bman::download::bman_flac_convert(&cfg.flac_convert);
    let flac_convert = resolve_flac_convert(flac_convert_override, bman_default)?;
    let outcome = download_show(
        &show,
        &tracks_with_urls,
        output_dir,
        &cfg.postprocess_codec,
        flac_convert,
        Service::Bman,
        FormatCode::Flac,
        bearer.as_deref(),
    )
    .await;

    if outcome.completed {
        bman::download::bman_save_cover_art(&show, output_dir, &bman).await;
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
    flac_convert_override: Option<&str>,
    force_login: bool,
) -> Result<()> {
    use crate::catalog::ArtistTarget;

    let mut cfg = load_config();
    cfg.normalize();
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
        bman: try_init_bman(&cfg),
    };

    let mut catalog = Catalog::new(crate::config::paths::cache_dir());
    catalog.set_setlistfm_api_key(&cfg.bman.setlistfm_api_key);
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

    let mut downloaded = 0usize;
    let mut skipped = 0usize;
    let sfm_keys = bman::setlistfm::SetlistFmKeys::from_comma_separated(&cfg.bman.setlistfm_api_key);
    let cache_dir = crate::config::paths::cache_dir();
    let mut sfm_cache = bman::setlistfm::SfmCache::load(&cache_dir);

    for (i, catalog_show) in shows.iter().enumerate() {
        let d = catalog_show.display_date();
        let date = if d.is_empty() { "Unknown date" } else { d };

        println!(
            "\n\x1b[1;38;5;214m[{}/{}]\x1b[0m {date} \u{00b7} {}",
            i + 1,
            total,
            catalog_show.venue_name,
        );

        // Fetch show detail + resolve tracks (branch on service)
        let (mut show, mut tracks_with_urls, flac_convert, bearer) = if catalog_show.service == Service::Bman {
            let bman = match router.bman_api() {
                Some(b) => b,
                None => {
                    println!("  \x1b[31mBman API not available\x1b[0m");
                    skipped += 1;
                    continue;
                }
            };
            let show = match bman::download::fetch_bman_show_detail(bman, catalog_show, &sfm_cache).await {
                Ok(s) => s,
                Err(e) => {
                    println!("  \x1b[31mFailed to fetch Bman show: {e}\x1b[0m");
                    skipped += 1;
                    continue;
                }
            };
            let (twu, bearer) = bman::download::resolve_bman_tracks(&show, bman).await;
            let bman_default = bman::download::bman_flac_convert(&cfg.flac_convert);
            let fc = resolve_flac_convert(flac_convert_override, bman_default)?.to_string();
            (show, twu, fc, bearer)
        } else {
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
            let api = router.api_for(catalog_show.service);
            let (twu, stats) = resolve_tracks(&show, api, format_code).await;
            print_resolution_warnings(&stats, "  ");
            let fc = resolve_flac_convert(flac_convert_override, &cfg.flac_convert)?.to_string();
            (show, twu, fc, None)
        };

        if tracks_with_urls.is_empty() {
            println!("  \x1b[38;5;214mNo downloadable tracks.\x1b[0m");
            skipped += 1;
            continue;
        }

        // Bman: enrich metadata before download
        if catalog_show.service == Service::Bman {
            bman::download::bman_enrich_metadata(
                &mut show,
                &output_dir,
                &sfm_keys,
                &mut sfm_cache,
            )
            .await;
            bman::download::sync_enriched_titles(&show, &mut tracks_with_urls);
        }

        let outcome = download_show_with_retry(
            &show,
            &tracks_with_urls,
            &output_dir,
            &cfg.postprocess_codec,
            &flac_convert,
            catalog_show.service,
            format_code,
            std::time::Duration::from_secs(30),
            bearer.as_deref(),
        )
        .await;

        if catalog_show.service == Service::Bman && outcome.completed {
            if let Some(bman) = router.bman_api() {
                bman::download::bman_save_cover_art(&show, &output_dir, bman).await;
            }
        }

        if outcome.completed {
            downloaded += 1;
        } else {
            // User cancelled
            break;
        }

        // Inter-show cooldown (skip after last show)
        if i + 1 < shows.len() {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    sfm_cache.save();

    println!("\n{downloaded}/{total} shows downloaded");
    if skipped > 0 {
        println!("{skipped} skipped");
    }

    Ok(())
}

/// Download a batch of Bman shows with progress, retry, enrichment, and cover art.
///
/// Returns `(downloaded_count, skipped_count)`. Stops early if the user cancels.
async fn download_bman_show_batch(
    shows: &[crate::models::CatalogShow],
    bman: &mut bman::BmanApi,
    cfg: &crate::config::Config,
    output_dir: &std::path::Path,
    flac_convert_override: Option<&str>,
) -> Result<(usize, usize)> {
    let total = shows.len();
    let mut downloaded = 0usize;
    let mut skipped = 0usize;
    let sfm_keys = bman::setlistfm::SetlistFmKeys::from_comma_separated(&cfg.bman.setlistfm_api_key);
    let cache_dir = crate::config::paths::cache_dir();
    let mut sfm_cache = bman::setlistfm::SfmCache::load(&cache_dir);

    for (i, catalog_show) in shows.iter().enumerate() {
        let d = catalog_show.display_date();
        let date = if d.is_empty() { "Unknown date" } else { d };

        println!(
            "\n\x1b[1;38;5;214m[{}/{}]\x1b[0m {date} \u{00b7} {}",
            i + 1,
            total,
            catalog_show.venue_name,
        );

        let mut show = match bman::download::fetch_bman_show_detail(bman, catalog_show, &sfm_cache).await {
            Ok(s) => s,
            Err(e) => {
                println!("  \x1b[31mFailed to fetch show: {e}\x1b[0m");
                skipped += 1;
                continue;
            }
        };

        let (mut tracks_with_urls, bearer) = bman::download::resolve_bman_tracks(&show, bman).await;

        if tracks_with_urls.is_empty() {
            println!("  \x1b[38;5;214mNo downloadable tracks.\x1b[0m");
            skipped += 1;
            continue;
        }

        bman::download::bman_enrich_metadata(&mut show, output_dir, &sfm_keys, &mut sfm_cache)
            .await;
        bman::download::sync_enriched_titles(&show, &mut tracks_with_urls);

        let bman_default = bman::download::bman_flac_convert(&cfg.flac_convert);
        let flac_convert = resolve_flac_convert(flac_convert_override, bman_default)?;

        let outcome = download_show_with_retry(
            &show,
            &tracks_with_urls,
            output_dir,
            &cfg.postprocess_codec,
            flac_convert,
            Service::Bman,
            FormatCode::Flac,
            std::time::Duration::from_secs(30),
            bearer.as_deref(),
        )
        .await;

        if outcome.completed {
            bman::download::bman_save_cover_art(&show, output_dir, bman).await;
            downloaded += 1;
        } else {
            break;
        }

        if i + 1 < total {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    sfm_cache.save();
    Ok((downloaded, skipped))
}

/// Download all Bman shows for a given year.
async fn run_download_year(
    year: &str,
    output_override: Option<&str>,
    flac_convert_override: Option<&str>,
) -> Result<()> {
    if year.len() != 4 || year.parse::<u16>().is_err() {
        bail!("Year must be a 4-digit number (e.g. 1977)");
    }

    let mut cfg = load_config();
    cfg.normalize();
    let output_dir = expand_tilde(output_override.unwrap_or(&cfg.output_dir));

    let mut bman = try_init_bman(&cfg)
        .context("GOOGLE_API_KEY not set — run: export GOOGLE_API_KEY=<your key>")?;

    // Load or fetch Bman catalog
    let mut catalog = Catalog::new(crate::config::paths::cache_dir());
    catalog.load(false);

    // Re-fetch if no cache or not yet enriched via setlist.fm
    let cache_dir = crate::config::paths::cache_dir();
    let has_enriched_cache = cache_dir.join("bman_enriched.marker").exists()
        && crate::catalog::cache::load_bman_cache(&cache_dir).is_some_and(|s| !s.is_empty());
    if !has_enriched_cache {
        println!("Fetching Bman catalog (first run may take a few minutes)...");
        catalog.fetch_bman_enriched(&mut bman, &cfg.bman.setlistfm_api_key)
            .await.context("Failed to fetch Bman catalog")?;
    }

    let shows: Vec<crate::models::CatalogShow> = catalog
        .get_shows_by_year(year)
        .into_iter()
        .filter(|s| s.service == Service::Bman)
        .collect();

    if shows.is_empty() {
        bail!("No Bman shows found for year {year}");
    }

    println!("{year}: {} shows", shows.len());

    let (downloaded, skipped) =
        download_bman_show_batch(&shows, &mut bman, &cfg, &output_dir, flac_convert_override).await?;

    println!("\n{downloaded}/{} shows downloaded", shows.len());
    if skipped > 0 {
        println!("{skipped} skipped");
    }

    Ok(())
}
