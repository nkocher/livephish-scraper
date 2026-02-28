use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use crate::models::CatalogShow;
use crate::service::Service;

use super::is_valid_live_show;

pub const CACHE_TTL_DAYS: u64 = 7;
pub const BMAN_CACHE_TTL_DAYS: u64 = 30;

/// Read and parse a cache file, returning None if missing, expired, or corrupt.
fn read_cache_file(path: &Path) -> Option<Vec<serde_json::Value>> {
    read_cache_file_with_ttl(path, CACHE_TTL_DAYS)
}

/// Read and parse a cache file with custom TTL.
fn read_cache_file_with_ttl(path: &Path, ttl_days: u64) -> Option<Vec<serde_json::Value>> {
    let metadata = fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let age = SystemTime::now().duration_since(modified).ok()?;
    if age.as_secs() > ttl_days * 86400 {
        return None;
    }
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Load cached shows for an artist. Returns None if cache missing/expired.
/// Auto-cleans invalid rows and rewrites cache if needed.
pub fn load_artist_cache(cache_dir: &Path, artist_id: i64) -> Option<Vec<CatalogShow>> {
    let cache_file = cache_dir.join(format!("catalog_{artist_id}.json"));
    let data = read_cache_file(&cache_file)?;
    let all_shows: Vec<CatalogShow> = data.iter().map(CatalogShow::from_json).collect();
    let valid_shows: Vec<CatalogShow> = all_shows.into_iter().filter(is_valid_live_show).collect();

    if valid_shows.len() < data.len() {
        save_artist_cache(cache_dir, artist_id, &valid_shows);
    }

    Some(valid_shows)
}

/// Serialize shows to JSON array for caching.
fn serialize_shows(shows: &[CatalogShow]) -> Vec<serde_json::Value> {
    shows
        .iter()
        .map(|s| {
            serde_json::json!({
                "containerID": s.container_id,
                "artistName": s.artist_name,
                "containerInfo": s.container_info,
                "venueName": s.venue_name,
                "venueCity": s.venue_city,
                "venueState": s.venue_state,
                "performanceDate": s.performance_date,
                "performanceDateFormatted": s.performance_date_formatted,
                "performanceDateYear": s.performance_date_year,
                "artistID": s.artist_id,
                "img": {"url": &s.image_url},
                "songList": s.song_list,
            })
        })
        .collect()
}

/// Write a JSON array to a cache file, creating parent directories if needed.
fn write_cache_file(cache_dir: &Path, filename: &str, shows: &[CatalogShow]) {
    let _ = fs::create_dir_all(cache_dir);
    let data = serialize_shows(shows);
    if let Ok(content) = serde_json::to_string_pretty(&data) {
        let _ = fs::write(cache_dir.join(filename), content);
    }
}

/// Save shows to per-artist cache file (JSON).
pub fn save_artist_cache(cache_dir: &Path, artist_id: i64, shows: &[CatalogShow]) {
    write_cache_file(cache_dir, &format!("catalog_{artist_id}.json"), shows);
}

/// Return cached show count, or -1 if missing/stale/corrupt.
pub fn cache_show_count(cache_dir: &Path, artist_id: i64) -> i32 {
    match load_artist_cache(cache_dir, artist_id) {
        Some(shows) => shows.len() as i32,
        None => -1,
    }
}

/// Extract artist_id from cache filename like "catalog_196.json".
pub fn artist_id_from_cache_file(filename: &str) -> Option<i64> {
    let stem = filename.strip_suffix(".json")?;
    let id_str = stem.strip_prefix("catalog_")?;
    id_str.parse().ok()
}

/// Load cached LivePhish shows. Returns None if cache missing/expired.
/// Auto-cleans invalid rows and rewrites cache if needed.
/// All returned shows are tagged with `Service::LivePhish`.
pub fn load_livephish_cache(cache_dir: &Path) -> Option<Vec<CatalogShow>> {
    let cache_file = cache_dir.join("catalog_livephish.json");
    let data = read_cache_file(&cache_file)?;
    let all_shows: Vec<CatalogShow> = data
        .iter()
        .map(|v| {
            let mut show = CatalogShow::from_json(v);
            show.service = Service::LivePhish;
            show
        })
        .collect();
    let valid_shows: Vec<CatalogShow> = all_shows.into_iter().filter(is_valid_live_show).collect();

    if valid_shows.len() < data.len() {
        save_livephish_cache(cache_dir, &valid_shows);
    }

    Some(valid_shows)
}

/// Save LivePhish shows to cache file.
pub fn save_livephish_cache(cache_dir: &Path, shows: &[CatalogShow]) {
    write_cache_file(cache_dir, "catalog_livephish.json", shows);
}

/// Load catalog metadata from catalog_meta.json.
pub fn load_catalog_meta(cache_dir: &Path) -> HashMap<String, serde_json::Value> {
    let meta_file = cache_dir.join("catalog_meta.json");
    if !meta_file.exists() {
        return HashMap::new();
    }
    fs::read_to_string(&meta_file)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Save catalog metadata to catalog_meta.json.
pub fn save_catalog_meta(
    cache_dir: &Path,
    meta: &HashMap<String, serde_json::Value>,
) {
    let _ = fs::create_dir_all(cache_dir);
    let meta_file = cache_dir.join("catalog_meta.json");
    if let Ok(content) = serde_json::to_string_pretty(meta) {
        let _ = fs::write(meta_file, content);
    }
}

// ── Bman cache ────────────────────────────────────────────────────────

/// Atomic write: write to `.tmp` then rename, preventing partial reads.
fn atomic_write(cache_dir: &Path, filename: &str, content: &str) {
    let _ = fs::create_dir_all(cache_dir);
    let tmp_file = cache_dir.join(format!("{filename}.tmp"));
    let final_file = cache_dir.join(filename);
    if fs::write(&tmp_file, content).is_ok() {
        let _ = fs::rename(tmp_file, final_file);
    }
}

/// Load cached Bman shows. Returns None if cache missing/expired.
/// All returned shows are tagged with `Service::Bman`.
/// If deserialization fails, deletes the corrupt cache and returns None.
pub fn load_bman_cache(cache_dir: &Path) -> Option<Vec<CatalogShow>> {
    let cache_file = cache_dir.join("catalog_bman.json");
    if let Some(data) = read_cache_file_with_ttl(&cache_file, BMAN_CACHE_TTL_DAYS) {
        let shows: Vec<CatalogShow> = data
            .iter()
            .map(|v| {
                let mut show = CatalogShow::from_json(v);
                show.service = Service::Bman;
                show
            })
            .collect();
        return Some(shows);
    }

    // Check if file exists but failed to parse (corrupt)
    if cache_file.exists() {
        if let Ok(content) = fs::read_to_string(&cache_file) {
            if serde_json::from_str::<Vec<serde_json::Value>>(&content).is_err() {
                tracing::warn!("Corrupt Bman cache — deleting and re-traversing");
                let _ = fs::remove_file(&cache_file);
            }
        }
    }
    None
}

/// Save Bman shows to cache file with atomic write (.tmp → rename).
pub fn save_bman_cache(cache_dir: &Path, shows: &[CatalogShow]) {
    let data = serialize_shows(shows);
    if let Ok(content) = serde_json::to_string_pretty(&data) {
        atomic_write(cache_dir, "catalog_bman.json", &content);
    }
}

/// Load Bman ID map from cache.
pub fn load_bman_id_map(cache_dir: &Path) -> Option<crate::bman::id_map::BmanIdMap> {
    let path = cache_dir.join("bman_id_map.json");
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Save Bman ID map to cache with atomic write.
pub fn save_bman_id_map(cache_dir: &Path, id_map: &crate::bman::id_map::BmanIdMap) {
    if let Ok(content) = serde_json::to_string_pretty(id_map) {
        atomic_write(cache_dir, "bman_id_map.json", &content);
    }
}
