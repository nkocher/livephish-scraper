pub mod cache;
pub mod registry;
pub mod search;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::warn;

use crate::api::error::ApiError;
use crate::api::NugsApi;
use crate::models::CatalogShow;
use crate::service::router::ServiceRouter;
use crate::service::Service;

use cache::{
    artist_id_from_cache_file, cache_show_count, load_artist_cache, load_bman_cache,
    load_catalog_meta, load_livephish_cache, save_artist_cache, save_bman_cache,
    save_bman_id_map, save_catalog_meta, save_livephish_cache,
};
use registry::{
    find_artist_ids, load_artist_registry, normalize_artist_name, registry_groups,
    save_artist_registry, ARTIST_REGISTRY_SCHEMA_VERSION,
};
use search::{build_corpus_entry, search_artist_shows, search_shows};

/// Discovery staleness threshold (days).
const DISCOVERY_STALENESS_DAYS: u64 = 30;

/// Artist IDs served by Bman (Google Drive archive).
pub fn is_bman_artist(artist_id: i64) -> bool {
    artist_id == crate::bman::parser::BMAN_GD_ARTIST_ID
        || artist_id == crate::bman::parser::BMAN_JGB_ARTIST_ID
}

/// Accumulates parsed Bman shows and folder scan counters.
struct BmanScanResult {
    parsed: Vec<crate::bman::parser::ParsedShow>,
    total_folders: usize,
    failed_folders: usize,
}

impl BmanScanResult {
    fn new() -> Self {
        Self {
            parsed: Vec::new(),
            total_folders: 0,
            failed_folders: 0,
        }
    }

    /// Try to parse a single Drive item as a Bman show folder.
    fn parse_item(
        &mut self,
        item: &crate::bman::gdrive::DriveItem,
        artist: crate::bman::parser::BmanArtist,
        is_nll: bool,
    ) {
        // Skip known non-show folders silently
        if crate::bman::parser::should_skip_folder(&item.name) {
            return;
        }

        self.total_folders += 1;
        match crate::bman::parser::parse_show_folder(&item.name, &item.id, artist, is_nll) {
            Some(parsed) => self.parsed.push(parsed),
            None => {
                self.failed_folders += 1;
                warn!("Unparseable show folder: {}", item.name);
            }
        }
    }
}

// ── Placeholder / validation helpers ────────────────────────────────────

const PLACEHOLDER_TOKENS: &[&str] = &["none", "null", "unknown", "n/a", "na", ""];

/// True if value is missing or a placeholder string like "None", "unknown", "n/a".
pub fn is_placeholder(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    let trimmed = value.trim().to_lowercase();
    PLACEHOLDER_TOKENS.contains(&trimmed.as_str())
}

/// Filter predicate: True for show-like containers with date and venue data.
///
/// Non-show entries (albums, compilations) typically lack performance dates
/// and venue metadata. Placeholder values like "None", "unknown" are treated as missing.
///
/// Bman shows use negative synthetic IDs and may lack venue metadata (etree folders),
/// so the rules are relaxed: only a meaningful date is required.
pub fn is_valid_live_show(show: &CatalogShow) -> bool {
    if show.service == Service::Bman {
        // Bman shows have negative synthetic IDs — only require a date
        return !is_placeholder(&show.performance_date);
    }
    if show.container_id <= 0 {
        return false;
    }
    // Must have meaningful date
    if is_placeholder(&show.performance_date) {
        return false;
    }
    // Must have at least one non-placeholder venue field
    !is_placeholder(&show.venue_name)
        || !is_placeholder(&show.venue_city)
        || !is_placeholder(&show.venue_state)
}

// ── Catalog struct ──────────────────────────────────────────────────────

/// Target for load_artist — either by numeric ID or by name string.
pub enum ArtistTarget {
    Id(i64),
    #[allow(dead_code)] // Phase 4: artist sub-menu search
    Name(String),
}

/// Multi-artist catalog with per-artist caching, dedup, and search.
pub struct Catalog {
    cache_dir: PathBuf,

    /// All loaded shows (filtered to valid live shows).
    pub shows: Vec<CatalogShow>,

    /// Year index: year string -> indices into `shows`.
    by_year: HashMap<String, Vec<usize>>,

    /// Search corpus: (container_id, lowercase_corpus_text).
    search_corpus: Vec<(i64, String)>,
    /// container_id -> index into `shows` (for search result lookup).
    search_show_idx: HashMap<i64, usize>,

    /// Artist registry: artist_id -> display name.
    artist_registry: HashMap<i64, String>,
    /// Artists with non-empty loaded data.
    loaded_artists: HashSet<i64>,
    /// All fetched/loaded artists (may be empty).
    attempted_artists: HashSet<i64>,
    /// Normalized name -> preferred artist_id (for dedup).
    preferred_artist_ids: HashMap<String, i64>,

    /// setlist.fm API key for catalog-time venue enrichment.
    setlistfm_api_key: String,
}

pub(crate) fn build_container_info(date: &str, venue: &str, source_tag: &str) -> String {
    let parts: Vec<&str> = [date, venue].into_iter().filter(|s| !s.is_empty()).collect();
    let mut info = parts.join(" ");
    if !source_tag.is_empty() {
        if info.is_empty() {
            info = format!("({})", source_tag);
        } else {
            info.push(' ');
            info.push_str(&format!("({})", source_tag));
        }
    }
    info
}

impl Catalog {
    pub fn new(cache_dir: PathBuf) -> Self {
        Catalog {
            cache_dir,
            shows: Vec::new(),
            by_year: HashMap::new(),
            search_corpus: Vec::new(),
            search_show_idx: HashMap::new(),
            artist_registry: HashMap::new(),
            loaded_artists: HashSet::new(),
            attempted_artists: HashSet::new(),
            preferred_artist_ids: HashMap::new(),
            setlistfm_api_key: String::new(),
        }
    }

    /// Set the setlist.fm API key for catalog-time venue enrichment.
    pub fn set_setlistfm_api_key(&mut self, key: &str) {
        self.setlistfm_api_key = key.to_string();
    }

    // ── Load (sync — disk only) ─────────────────────────────────────

    /// Load artist registry from disk and any cached artist catalogs.
    ///
    /// If `has_livephish` is true and a LivePhish cache exists, Phish shows are
    /// loaded from `catalog_livephish.json` (tagged `Service::LivePhish`) instead
    /// of the regular per-artist cache.
    pub fn load(&mut self, has_livephish: bool) {
        self.artist_registry = load_artist_registry(&self.cache_dir);
        self.run_registry_migration_if_needed();

        // Load Bman cache only if enrichment has been applied.
        // If the cache exists but lacks the enrichment marker, skip loading
        // so fetch_bman_enriched() runs on next access and enriches via setlist.fm.
        let bman_enriched = self.cache_dir.join("bman_enriched.marker").exists();
        let bman_loaded = bman_enriched
            && load_bman_cache(&self.cache_dir)
                .filter(|s| !s.is_empty())
                .map(|s| {
                    for show in &s {
                        self.attempted_artists.insert(show.artist_id);
                        self.loaded_artists.insert(show.artist_id);
                    }
                    self.shows.extend(s);
                })
                .is_some();

        // Determine which Phish artist IDs should be skipped (served from LivePhish cache)
        let livephish_loaded = has_livephish
            && load_livephish_cache(&self.cache_dir)
                .filter(|s| !s.is_empty())
                .map(|s| self.shows.extend(s))
                .is_some();

        let mut registry_changed = false;
        let artist_ids: Vec<i64> = self.artist_registry.keys().copied().collect();

        for artist_id in artist_ids {
            // Skip nugs cache for Bman artists if Bman cache was loaded
            if bman_loaded && is_bman_artist(artist_id) {
                continue;
            }

            // Skip nugs Phish cache if LivePhish cache was loaded
            if livephish_loaded && self.is_phish_artist(artist_id) {
                self.attempted_artists.insert(artist_id);
                self.loaded_artists.insert(artist_id);
                continue;
            }

            let cached = load_artist_cache(&self.cache_dir, artist_id);
            if let Some(shows) = cached {
                self.attempted_artists.insert(artist_id);
                if !shows.is_empty() {
                    if self.reconcile_registry_name(&shows, false) {
                        registry_changed = true;
                    }
                    self.shows.extend(shows);
                    self.loaded_artists.insert(artist_id);
                } else {
                    self.loaded_artists.remove(&artist_id);
                }
            }
        }

        if registry_changed {
            save_artist_registry(&self.cache_dir, &self.artist_registry);
        }
        self.build_indexes();
    }

    // ── Fetch (async — API calls) ───────────────────────────────────

    /// Fetch all containers for one artist from API, cache results.
    ///
    /// If the artist is Phish and LivePhish is available, tries the LivePhish API first
    /// (returns richer catalog data). Falls back to nugs.net on failure.
    pub async fn fetch_artist(
        &mut self,
        router: &mut ServiceRouter,
        artist_id: i64,
    ) -> Result<Vec<CatalogShow>, ApiError> {
        // Route Bman artists (GD/JGB) to Google Drive — no fallback to nugs
        if is_bman_artist(artist_id) {
            if let Some(bman) = router.bman.as_mut() {
                // fetch_bman handles retain+extend+build_indexes internally
                let sfm_key = self.setlistfm_api_key.clone();
                let shows = self.fetch_bman_enriched(bman, &sfm_key).await?;

                let bman_ids = [
                    crate::bman::parser::BMAN_GD_ARTIST_ID,
                    crate::bman::parser::BMAN_JGB_ARTIST_ID,
                ];
                for aid in bman_ids {
                    self.attempted_artists.insert(aid);
                    if shows.iter().any(|s| s.artist_id == aid) {
                        self.loaded_artists.insert(aid);
                    }
                }

                // Return just the requested artist's shows
                let artist_shows: Vec<CatalogShow> =
                    shows.into_iter().filter(|s| s.artist_id == artist_id).collect();
                return Ok(artist_shows);
            }
            // No Bman API available — fall through (will try nugs for GD 461)
        }

        // Try LivePhish for Phish artists when available
        if self.is_phish_artist(artist_id) && router.has_livephish() {
            match self
                .fetch_artist_livephish(router.livephish.as_mut().unwrap())
                .await
            {
                Ok(shows) => {
                    // LivePhish has no artist_id concept; register under nugs artist_id
                    if let Some(first) = shows.first() {
                        self.set_artist_name(artist_id, &first.artist_name, true);
                    }

                    // Remove old shows for this artist and add new ones
                    self.shows.retain(|s| s.artist_id != artist_id);
                    self.shows.extend(shows.clone());

                    self.attempted_artists.insert(artist_id);
                    if !shows.is_empty() {
                        self.loaded_artists.insert(artist_id);
                        self.remember_preferred_artist(artist_id);
                    } else {
                        self.loaded_artists.remove(&artist_id);
                    }
                    self.build_indexes();
                    return Ok(shows);
                }
                Err(e) => {
                    warn!(
                        "LivePhish catalog fetch failed, falling back to nugs.net: {}",
                        e
                    );
                }
            }
        }

        let api = &mut router.nugs;
        let containers = api.get_artist_catalog(artist_id, 500).await?;
        let shows: Vec<CatalogShow> = containers
            .iter()
            .map(CatalogShow::from_json)
            .filter(is_valid_live_show)
            .collect();

        if let Some(first) = shows.first() {
            self.set_artist_name(artist_id, &first.artist_name, true);
        }

        save_artist_cache(&self.cache_dir, artist_id, &shows);

        // Remove old shows for this artist and add new ones
        self.shows.retain(|s| s.artist_id != artist_id);
        self.shows.extend(shows.clone());

        self.attempted_artists.insert(artist_id);
        if !shows.is_empty() {
            self.loaded_artists.insert(artist_id);
            self.remember_preferred_artist(artist_id);
        } else {
            self.loaded_artists.remove(&artist_id);
        }
        self.build_indexes();
        Ok(shows)
    }

    /// Fetch all containers from the LivePhish single-artist API.
    ///
    /// Tags all returned shows with `Service::LivePhish` and saves to a
    /// separate cache file (`catalog_livephish.json`).
    async fn fetch_artist_livephish(
        &mut self,
        api: &mut NugsApi,
    ) -> Result<Vec<CatalogShow>, ApiError> {
        let containers = api.get_all_catalog(500).await?;
        let mut shows: Vec<CatalogShow> = containers
            .iter()
            .map(CatalogShow::from_json)
            .filter(is_valid_live_show)
            .collect();

        // Tag all shows as LivePhish
        for show in &mut shows {
            show.service = Service::LivePhish;
        }

        save_livephish_cache(&self.cache_dir, &shows);
        Ok(shows)
    }

    /// Resolve/load an artist and return the ID that has show data.
    ///
    /// Name lookups are normalized so punctuation variants resolve together.
    /// If the first matching ID is empty, probes aliases until non-empty data is found.
    pub async fn load_artist(
        &mut self,
        router: &mut ServiceRouter,
        target: ArtistTarget,
        allow_alias_fallback: bool,
    ) -> Option<i64> {
        let candidate_ids = self.candidate_artist_ids(&target, allow_alias_fallback);
        if candidate_ids.is_empty() {
            return None;
        }

        let requested_normalized_name = match &target {
            ArtistTarget::Name(name) => normalize_artist_name(name),
            ArtistTarget::Id(_) => String::new(),
        };

        let num_candidates = candidate_ids.len();
        for (idx, artist_id) in candidate_ids.iter().enumerate() {
            let artist_id = *artist_id;

            // Check already-loaded shows
            let existing = self.get_shows_by_artist_id(artist_id);
            if !existing.is_empty() {
                if !requested_normalized_name.is_empty() {
                    let resolved = normalize_artist_name(&existing[0].artist_name);
                    if resolved != requested_normalized_name {
                        if !allow_alias_fallback || idx == num_candidates - 1 {
                            break;
                        }
                        continue;
                    }
                }
                self.remember_preferred_artist(artist_id);
                return Some(artist_id);
            }

            // Fetch if not attempted
            if !self.attempted_artists.contains(&artist_id) {
                match self.fetch_artist(router, artist_id).await {
                    Ok(shows) if !shows.is_empty() => {
                        if !requested_normalized_name.is_empty() {
                            let resolved = normalize_artist_name(&shows[0].artist_name);
                            if resolved != requested_normalized_name {
                                if !allow_alias_fallback || idx == num_candidates - 1 {
                                    break;
                                }
                                continue;
                            }
                        }
                        self.remember_preferred_artist(artist_id);
                        return Some(artist_id);
                    }
                    Ok(_) => {} // empty — try next
                    Err(_) => return None,
                }
            }

            if !allow_alias_fallback || idx == num_candidates - 1 {
                break;
            }
        }

        None
    }

    // ── Discovery (async — API calls) ───────────────────────────────

    /// Run artist discovery if never discovered or stale (>30 days).
    pub async fn discover_if_needed(&mut self, router: &mut ServiceRouter) {
        let meta = load_catalog_meta(&self.cache_dir);
        let last_discovery = meta
            .get("last_discovery_at")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);

        let needs_discovery = last_discovery == 0.0
            || (now - last_discovery) > (DISCOVERY_STALENESS_DAYS * 86400) as f64;

        if !needs_discovery {
            return;
        }

        if let Ok((_, discovered_any)) = self.run_discovery(&mut router.nugs).await {
            if discovered_any {
                let mut meta = load_catalog_meta(&self.cache_dir);
                meta.insert("last_discovery_at".to_string(), serde_json::json!(now));
                save_catalog_meta(&self.cache_dir, &meta);
            }
        }
    }

    /// Discover new artists and re-fetch all loaded artist catalogs.
    pub async fn refresh(&mut self, router: &mut ServiceRouter) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);

        // Run discovery (always uses nugs API)
        let _ = self.run_discovery(&mut router.nugs).await;
        let mut meta = load_catalog_meta(&self.cache_dir);
        meta.insert("last_discovery_at".to_string(), serde_json::json!(now));
        save_catalog_meta(&self.cache_dir, &meta);

        // Re-fetch all loaded artists
        let artist_ids: Vec<i64> = self.loaded_artists.iter().copied().collect();
        for artist_id in artist_ids {
            let _ = self.fetch_artist(router, artist_id).await;
        }
    }

    /// Run artist discovery and merge into registry.
    /// Returns (new_count, discovered_any).
    async fn run_discovery(&mut self, api: &mut NugsApi) -> Result<(usize, bool), ApiError> {
        let discovered = api.get_all_artists().await;
        if discovered.is_empty() {
            return Ok((0, false));
        }

        let new_count = discovered
            .keys()
            .filter(|aid| !self.artist_registry.contains_key(aid))
            .count();

        for (aid, aname) in &discovered {
            self.artist_registry
                .entry(*aid)
                .or_insert_with(|| aname.clone());
        }

        save_artist_registry(&self.cache_dir, &self.artist_registry);
        self.rebuild_preferred_artist_ids();

        Ok((new_count, true))
    }

    // ── Bman helpers ───────────────────────────────────────────────────

    /// Traverse Bman's Google Drive archive and build catalog entries.
    ///
    /// Walks root -> year/NLL/JGB -> show folders, parses metadata,
    /// deduplicates, caches, and converts to CatalogShow.
    /// Saves partial results on mid-traversal errors.
    /// When `setlistfm_api_key` is non-empty, enriches shows missing venue data.
    pub async fn fetch_bman_enriched(
        &mut self,
        bman: &mut crate::bman::BmanApi,
        setlistfm_api_key: &str,
    ) -> Result<Vec<CatalogShow>, ApiError> {
        use crate::bman::gdrive::FOLDER_MIME;
        use crate::bman::parser::{
            dedup_shows, is_jgb_folder, is_nll_folder, is_year_folder, BmanArtist,
            BMAN_GD_ARTIST_ID, BMAN_JGB_ARTIST_ID,
        };

        // Load persisted ID map if available
        if let Some(id_map) = cache::load_bman_id_map(&self.cache_dir) {
            bman.id_map = id_map;
        }

        let mut result = BmanScanResult::new();

        // List root folder
        let root_items = bman
            .list_folder_filtered(&bman.root_folder_id.clone(), FOLDER_MIME)
            .await
            .map_err(|e| ApiError::UnexpectedResponse(e.to_string()))?;

        for item in &root_items {
            if let Some(year) = is_year_folder(&item.name) {
                tracing::info!("Scanning year {}...", year);
                self.scan_bman_folder(bman, &item.id, BmanArtist::GratefulDead, false, &mut result)
                    .await;
            } else if is_nll_folder(&item.name) {
                tracing::info!("Scanning NLL folder...");
                self.scan_bman_subfolder_tree(bman, &item.id, BmanArtist::GratefulDead, true, &mut result)
                    .await;
            } else if is_jgb_folder(&item.name) {
                tracing::info!("Scanning JGB folder...");
                self.scan_bman_subfolder_tree(bman, &item.id, BmanArtist::JerryGarciaBand, false, &mut result)
                    .await;
            }
        }

        tracing::info!(
            "Parsed {} of {} folders ({} failed)",
            result.parsed.len(),
            result.total_folders,
            result.failed_folders
        );

        let mut deduped = dedup_shows(result.parsed);
        tracing::info!("{} shows after deduplication", deduped.len());

        // Enrich ALL shows via setlist.fm — it's the source of truth for
        // venue/city/state. Folder names are unreliable (taper names, shn.org IDs, etc.)
        let mut aborted = false;
        if !setlistfm_api_key.is_empty() {
            tracing::info!(
                "Enriching {} shows via setlist.fm...",
                deduped.len()
            );
            let client = reqwest::Client::new();
            let mut enriched = 0usize;
            let mut consecutive_errors = 0usize;
            let total = deduped.len();
            let rate_limit = std::time::Duration::from_millis(500);

            // Apply venue data from a setlist.fm result to a parsed show
            let apply_venue = |show: &mut crate::bman::parser::ParsedShow,
                               sv: crate::bman::setlistfm::SetlistVenue,
                               enriched: &mut usize| {
                show.venue = sv.venue_name;
                show.city = sv.city;
                show.state = sv.state;
                *enriched += 1;
            };

            for (i, show) in deduped.iter_mut().enumerate() {
                if (i + 1) % 100 == 0 {
                    println!("  setlist.fm: {}/{} shows processed ({} enriched)...", i + 1, total, enriched);
                }
                let artist_name = show.artist.name();
                let fetch = || {
                    crate::bman::setlistfm::fetch_setlist_venue(
                        &client,
                        setlistfm_api_key,
                        artist_name,
                        &show.date,
                    )
                };
                match fetch().await {
                    Ok(Some(sv)) => {
                        apply_venue(show, sv, &mut enriched);
                        consecutive_errors = 0;
                    }
                    Ok(None) => {
                        consecutive_errors = 0;
                    }
                    Err(e) => {
                        let e_str = e.to_string();
                        // Permanent auth errors — bad API key, abort immediately
                        if e_str.contains("HTTP 401") || e_str.contains("HTTP 403") {
                            tracing::warn!(
                                "setlist.fm auth error ({}), stopping enrichment — check API key",
                                e_str
                            );
                            aborted = true;
                            break;
                        }
                        // Transient errors (429 rate limit or 5xx server error) — retry once
                        let is_retryable = e_str.contains("429") || e_str.contains("HTTP 5");
                        if is_retryable {
                            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                            match fetch().await {
                                Ok(Some(sv)) => {
                                    apply_venue(show, sv, &mut enriched);
                                    consecutive_errors = 0;
                                    tokio::time::sleep(rate_limit).await;
                                    continue;
                                }
                                Ok(None) => {
                                    consecutive_errors = 0;
                                    tokio::time::sleep(rate_limit).await;
                                    continue;
                                }
                                Err(retry_e) => {
                                    tracing::warn!("setlist.fm retry failed for {}: {}", show.date, retry_e);
                                    consecutive_errors += 1;
                                }
                            }
                        } else {
                            tracing::warn!("setlist.fm error for {}: {}", show.date, e_str);
                            consecutive_errors += 1;
                        }
                        if consecutive_errors >= 10 {
                            tracing::warn!(
                                "Too many consecutive setlist.fm errors, stopping enrichment"
                            );
                            aborted = true;
                            break;
                        }
                    }
                }
                tokio::time::sleep(rate_limit).await;
            }
            tracing::info!(
                "setlist.fm enriched {}/{} shows",
                enriched,
                deduped.len()
            );
        }

        // Register artists
        self.register_artist(BMAN_GD_ARTIST_ID, "Grateful Dead");
        self.register_artist(BMAN_JGB_ARTIST_ID, "Jerry Garcia Band");

        // Convert to CatalogShow with synthetic IDs
        let shows: Vec<CatalogShow> = deduped
            .iter()
            .map(|parsed| {
                let container_id = bman.id_map.insert(&parsed.folder_id);
                CatalogShow {
                    container_id,
                    artist_id: parsed.artist.artist_id(),
                    artist_name: parsed.artist.name().to_string(),
                    container_info: build_container_info(
                        &parsed.date,
                        &parsed.venue,
                        &parsed.source_tag,
                    ),
                    venue_name: parsed.venue.clone(),
                    venue_city: parsed.city.clone(),
                    venue_state: parsed.state.clone(),
                    performance_date: parsed.date.clone(),
                    performance_date_year: parsed.date.get(..4).unwrap_or("").to_string(),
                    service: Service::Bman,
                    ..Default::default()
                }
            })
            .collect();

        // Cache results + ID map + enrichment marker
        save_bman_cache(&self.cache_dir, &shows);
        save_bman_id_map(&self.cache_dir, &bman.id_map);
        if !setlistfm_api_key.is_empty() && !aborted {
            let _ = std::fs::write(self.cache_dir.join("bman_enriched.marker"), "1");
        }

        // Build artwork index (load from cache or fetch from Drive)
        let artwork_index = match crate::bman::artwork::load_artwork_index(&self.cache_dir) {
            Some(idx) => {
                tracing::debug!("Loaded artwork index from cache ({} dates)", idx.len());
                idx
            }
            None => {
                tracing::info!("Building artwork index from Drive catalog...");
                match crate::bman::artwork::build_artwork_index(bman).await {
                    Ok(idx) => {
                        crate::bman::artwork::save_artwork_index(&self.cache_dir, &idx);
                        tracing::info!("Artwork index: {} dates indexed", idx.len());
                        idx
                    }
                    Err(e) => {
                        tracing::warn!("Failed to build artwork index: {e}");
                        crate::bman::artwork::ArtworkIndex::new()
                    }
                }
            }
        };
        bman.artwork_index = artwork_index;

        // Store in memory so callers don't need to manually extend
        self.shows.retain(|s| s.service != Service::Bman);
        self.shows.extend(shows.clone());
        self.build_indexes();

        Ok(shows)
    }

    /// List subfolders in a Bman folder and parse each as a show.
    /// Shortcuts are resolved to their target folder IDs.
    async fn scan_bman_folder(
        &self,
        bman: &crate::bman::BmanApi,
        folder_id: &str,
        artist: crate::bman::parser::BmanArtist,
        is_nll: bool,
        result: &mut BmanScanResult,
    ) {
        use crate::bman::gdrive::FOLDER_MIME;

        match bman.list_folder_filtered(folder_id, FOLDER_MIME).await {
            Ok(show_items) => {
                for show_item in &show_items {
                    let Some(eid) = show_item.effective_folder_id() else {
                        continue;
                    };
                    let mut resolved = show_item.clone();
                    resolved.id = eid.to_string();
                    result.parse_item(&resolved, artist, is_nll);
                }
            }
            Err(e) => warn!("Failed to list Bman folder {}: {}", folder_id, e),
        }
    }

    /// List a container folder (NLL or JGB), scanning year subfolders and flat shows.
    async fn scan_bman_subfolder_tree(
        &self,
        bman: &crate::bman::BmanApi,
        folder_id: &str,
        artist: crate::bman::parser::BmanArtist,
        is_nll: bool,
        result: &mut BmanScanResult,
    ) {
        use crate::bman::gdrive::FOLDER_MIME;
        use crate::bman::parser::is_year_folder;

        match bman.list_folder_filtered(folder_id, FOLDER_MIME).await {
            Ok(items) => {
                for item in &items {
                    // Resolve shortcuts to their target folder ID
                    let effective_id = match item.effective_folder_id() {
                        Some(id) => id.to_string(),
                        None => continue, // Skip non-folder shortcuts
                    };
                    if is_year_folder(&item.name).is_some() {
                        self.scan_bman_folder(bman, &effective_id, artist, is_nll, result)
                            .await;
                    } else {
                        let mut resolved = item.clone();
                        resolved.id = effective_id;
                        result.parse_item(&resolved, artist, is_nll);
                    }
                }
            }
            Err(e) => warn!("Failed to list Bman subfolder {}: {}", folder_id, e),
        }
    }

    // ── LivePhish helpers ─────────────────────────────────────────────

    /// Whether this artist ID maps to "Phish" in the registry.
    fn is_phish_artist(&self, artist_id: i64) -> bool {
        self.artist_registry
            .get(&artist_id)
            .map(|name| normalize_artist_name(name) == "phish")
            .unwrap_or(false)
    }

    // ── Registry helpers ────────────────────────────────────────────

    /// Add an artist to the registry (idempotent — won't overwrite existing).
    pub fn register_artist(&mut self, artist_id: i64, artist_name: &str) {
        if !self.artist_registry.contains_key(&artist_id) {
            self.set_artist_name(artist_id, artist_name, true);
        }
    }

    /// Get artist name from the registry.
    pub fn get_artist_name(&self, artist_id: i64) -> Option<&str> {
        // Check loaded shows first (most accurate)
        let from_shows = self
            .shows
            .iter()
            .find(|s| s.artist_id == artist_id)
            .map(|s| s.artist_name.as_str());
        if from_shows.is_some() {
            return from_shows;
        }
        // Fall back to registry
        self.artist_registry.get(&artist_id).map(|s| s.as_str())
    }

    /// Deduplicated artist choices for UI: (artist_id, display_name).
    pub fn get_all_artist_choices(&self) -> Vec<(i64, String)> {
        let groups = registry_groups(&self.artist_registry);
        let mut choices: Vec<(i64, String)> = Vec::new();

        for artist_ids in groups.values() {
            let preferred = self.choose_preferred_artist_id(artist_ids);
            if let Some(name) = self.get_artist_name(preferred) {
                choices.push((preferred, name.to_string()));
            }
        }

        choices.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
        choices
    }

    fn set_artist_name(&mut self, artist_id: i64, name: &str, persist: bool) -> bool {
        let current = self.artist_registry.get(&artist_id);
        if current.map(|s| s.as_str()) == Some(name) {
            return false;
        }
        self.artist_registry.insert(artist_id, name.to_string());
        self.rebuild_preferred_artist_ids();
        if persist {
            save_artist_registry(&self.cache_dir, &self.artist_registry);
        }
        true
    }

    fn reconcile_registry_name(&mut self, shows: &[CatalogShow], persist: bool) -> bool {
        match shows.first() {
            Some(show) => self.set_artist_name(show.artist_id, &show.artist_name, persist),
            None => false,
        }
    }

    fn run_registry_migration_if_needed(&mut self) {
        if self.catalog_schema_version() >= ARTIST_REGISTRY_SCHEMA_VERSION {
            return;
        }

        let mut changed = false;
        // Scan cache files for artist names
        if let Ok(entries) = fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some(artist_id) = artist_id_from_cache_file(&name) {
                    if let Some(cached) = load_artist_cache(&self.cache_dir, artist_id) {
                        if self.reconcile_registry_name(&cached, false) {
                            changed = true;
                        }
                    }
                }
            }
        }

        if changed {
            save_artist_registry(&self.cache_dir, &self.artist_registry);
        }
        self.set_catalog_schema_version(ARTIST_REGISTRY_SCHEMA_VERSION);
    }

    fn catalog_schema_version(&self) -> i64 {
        let meta = load_catalog_meta(&self.cache_dir);
        meta.get("artist_registry_schema_version")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
    }

    fn set_catalog_schema_version(&self, version: i64) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let mut meta = load_catalog_meta(&self.cache_dir);
        meta.insert(
            "artist_registry_schema_version".to_string(),
            serde_json::json!(version),
        );
        meta.insert("last_reconcile_at".to_string(), serde_json::json!(now));
        save_catalog_meta(&self.cache_dir, &meta);
    }

    fn remember_preferred_artist(&mut self, artist_id: i64) {
        if let Some(name) = self.artist_registry.get(&artist_id) {
            let normalized = normalize_artist_name(name);
            self.preferred_artist_ids.insert(normalized, artist_id);
        }
    }

    fn rebuild_preferred_artist_ids(&mut self) {
        self.preferred_artist_ids.clear();
        let groups = registry_groups(&self.artist_registry);
        for (normalized, artist_ids) in &groups {
            let preferred = self.choose_preferred_artist_id(artist_ids);
            self.preferred_artist_ids
                .insert(normalized.clone(), preferred);
        }
    }

    fn choose_preferred_artist_id(&self, artist_ids: &[i64]) -> i64 {
        if artist_ids.len() == 1 {
            return artist_ids[0];
        }
        // Count loaded shows per artist
        let mut counts: HashMap<i64, usize> = HashMap::new();
        for show in &self.shows {
            *counts.entry(show.artist_id).or_default() += 1;
        }

        *artist_ids
            .iter()
            .max_by_key(|&&aid| self.artist_rank_key(aid, &counts))
            .unwrap_or(&artist_ids[0])
    }

    fn artist_rank_key(
        &self,
        artist_id: i64,
        counts: &HashMap<i64, usize>,
    ) -> (u8, usize, u8, i32, i64) {
        let loaded_count = *counts.get(&artist_id).unwrap_or(&0);
        let cc = cache_show_count(&self.cache_dir, artist_id);
        // Cache tiers: has shows (2) > untried/stale (1) > known empty (0)
        let cache_tier = if cc > 0 {
            2
        } else if cc < 0 {
            1
        } else {
            0
        };
        (
            if loaded_count > 0 { 1 } else { 0 },
            loaded_count,
            cache_tier,
            cc.max(0),
            -artist_id, // tie-break: lower ID preferred
        )
    }

    fn candidate_artist_ids(&self, target: &ArtistTarget, allow_alias_fallback: bool) -> Vec<i64> {
        match target {
            ArtistTarget::Id(requested_id) => {
                let requested_id = *requested_id;
                if !self.artist_registry.contains_key(&requested_id) {
                    return vec![requested_id];
                }
                if !allow_alias_fallback {
                    return vec![requested_id];
                }
                let name = &self.artist_registry[&requested_id];
                let mut ids = find_artist_ids(&self.artist_registry, name);
                if ids.is_empty() {
                    ids = vec![requested_id];
                }
                // Put requested_id first
                if ids.contains(&requested_id) {
                    ids.retain(|&id| id != requested_id);
                    ids.insert(0, requested_id);
                }
                ids
            }
            ArtistTarget::Name(name) => {
                let mut ids = find_artist_ids(&self.artist_registry, name);
                if ids.is_empty() {
                    return Vec::new();
                }
                // Sort by rank, preferred first
                let counts: HashMap<i64, usize> = {
                    let mut c = HashMap::new();
                    for show in &self.shows {
                        *c.entry(show.artist_id).or_default() += 1;
                    }
                    c
                };
                ids.sort_by(|a, b| {
                    self.artist_rank_key(*b, &counts)
                        .cmp(&self.artist_rank_key(*a, &counts))
                });
                // Put preferred ID first if known
                let normalized = normalize_artist_name(name);
                if let Some(&preferred) = self.preferred_artist_ids.get(&normalized) {
                    if ids.contains(&preferred) {
                        ids.retain(|&id| id != preferred);
                        ids.insert(0, preferred);
                    }
                }
                ids
            }
        }
    }

    // ── Query methods (sync) ────────────────────────────────────────

    /// Get available years sorted descending.
    #[allow(dead_code)] // Used in catalog tests
    pub fn get_years(&self) -> Vec<String> {
        let mut years: Vec<String> = self.by_year.keys().cloned().collect();
        years.sort_by(|a, b| b.cmp(a));
        years
    }

    /// Get years with show counts, sorted descending.
    pub fn year_show_counts(&self) -> Vec<(String, usize)> {
        let mut counts: Vec<(String, usize)> = self
            .by_year
            .iter()
            .map(|(year, indices)| (year.clone(), indices.len()))
            .collect();
        counts.sort_by(|a, b| b.0.cmp(&a.0));
        counts
    }

    /// Get shows for a year sorted by date descending.
    pub fn get_shows_by_year(&self, year: &str) -> Vec<CatalogShow> {
        let mut shows: Vec<CatalogShow> = self
            .by_year
            .get(year)
            .map(|indices| indices.iter().map(|&i| self.shows[i].clone()).collect())
            .unwrap_or_default();
        shows.sort_by(|a, b| b.performance_date.cmp(&a.performance_date));
        shows
    }

    /// Get shows for an artist sorted by date descending.
    pub fn get_shows_by_artist_id(&self, artist_id: i64) -> Vec<CatalogShow> {
        let mut shows: Vec<CatalogShow> = self
            .shows
            .iter()
            .filter(|s| s.artist_id == artist_id)
            .cloned()
            .collect();
        shows.sort_by(|a, b| b.performance_date.cmp(&a.performance_date));
        shows
    }

    /// Whether artist discovery has been performed (registry is non-empty).
    pub fn has_discovered(&self) -> bool {
        !self.artist_registry.is_empty()
    }

    /// Whether artist has known non-empty loaded or cached data.
    pub fn artist_has_data(&self, artist_id: i64) -> bool {
        if self.shows.iter().any(|s| s.artist_id == artist_id) {
            return true;
        }
        cache_show_count(&self.cache_dir, artist_id) > 0
    }

    /// Fuzzy search across all shows.
    pub fn search(&self, query: &str, limit: usize) -> Vec<CatalogShow> {
        search_shows(
            query,
            &self.search_corpus,
            &self.search_show_idx,
            &self.shows,
            limit,
        )
    }

    /// Fuzzy search within a single artist's shows.
    pub fn search_artist(&self, query: &str, artist_id: i64, limit: usize) -> Vec<CatalogShow> {
        search_artist_shows(
            query,
            artist_id,
            &self.search_corpus,
            &self.search_show_idx,
            &self.shows,
            limit,
        )
    }

    // ── Index building (sync) ───────────────────────────────────────

    pub(crate) fn build_indexes(&mut self) {
        // Re-filter shows (in case any invalid slipped in)
        self.shows.retain(is_valid_live_show);

        self.by_year.clear();
        self.search_corpus.clear();
        self.search_show_idx.clear();

        for (idx, show) in self.shows.iter().enumerate() {
            let year = if show.performance_date_year.is_empty() {
                "Unknown"
            } else {
                &show.performance_date_year
            };
            self.by_year.entry(year.to_string()).or_default().push(idx);

            // Search corpus
            let corpus = build_corpus_entry(show);
            self.search_corpus.push((show.container_id, corpus));
            self.search_show_idx.insert(show.container_id, idx);
        }

        self.rebuild_preferred_artist_ids();
    }
}

#[cfg(test)]
mod tests;
