/// Bman artwork: catalog index, show-folder download, and smart cover selection.
///
/// Three sources of cover art, checked in priority order:
/// 1. Show-folder images (downloaded to `artwork/` subdir)
/// 2. Artwork catalog folder (997 venue-specific JPEGs keyed by date)
/// 3. Generated procedural art (fallback)
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::bman::BmanApi;

/// Artwork catalog folder ID in Bman's Google Drive.
pub const BMAN_ARTWORK_FOLDER_ID: &str = "161K2Fj9sXIY7vgaP0wuPpRUBcmSyFyfc";

/// Maximum images to download per show folder.
const MAX_IMAGES_PER_SHOW: usize = 20;

/// Maximum image file size (50 MB).
const MAX_IMAGE_SIZE: u64 = 50 * 1024 * 1024;

/// Cache filename for the artwork index.
const ARTWORK_INDEX_CACHE: &str = "bman_artwork_index.json";

/// Cache TTL for the artwork index (30 days, matching bman catalog).
const ARTWORK_INDEX_TTL_DAYS: u64 = 30;

/// Parses dates from artwork catalog filenames: `gd77-05-08-Barton-Hall.jpg`
static ARTWORK_DATE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^(?:gd|jgb|jg)(\d{2})-(\d{2})-(\d{2})").unwrap());

// ---- Types ------------------------------------------------------------------

/// A single artwork file from the catalog folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtworkEntry {
    pub file_id: String,
    pub name: String,
    pub size: u64,
}

/// Date-keyed index of artwork catalog: `"1977-05-08"` → `Vec<ArtworkEntry>`.
pub type ArtworkIndex = HashMap<String, Vec<ArtworkEntry>>;

// ---- Artwork index building -------------------------------------------------

/// Build the artwork index by listing the artwork catalog folder.
pub async fn build_artwork_index(
    bman: &BmanApi,
) -> Result<ArtworkIndex, crate::bman::BmanError> {
    let items = bman.list_folder(BMAN_ARTWORK_FOLDER_ID).await?;
    let mut index: ArtworkIndex = HashMap::new();

    for item in &items {
        // Only index image files
        if !is_image_file(&item.name) {
            continue;
        }
        if let Some(date) = parse_artwork_date(&item.name) {
            index.entry(date).or_default().push(ArtworkEntry {
                file_id: item.id.clone(),
                name: item.name.clone(),
                size: item.size_bytes(),
            });
        }
    }

    debug!(
        "Built artwork index: {} dates, {} total images",
        index.len(),
        index.values().map(|v| v.len()).sum::<usize>()
    );
    Ok(index)
}

/// Parse date from an artwork catalog filename.
/// `gd77-05-08-Barton-Hall.jpg` → `"1977-05-08"`
fn parse_artwork_date(name: &str) -> Option<String> {
    let cap = ARTWORK_DATE_RE.captures(name)?;
    let yy: u32 = cap[1].parse().ok()?;
    let year = if yy >= 50 { 1900 + yy } else { 2000 + yy };
    Some(format!("{}-{}-{}", year, &cap[2], &cap[3]))
}

fn is_image_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".png")
        || lower.ends_with(".webp")
}

// ---- Artwork index caching --------------------------------------------------

pub fn load_artwork_index(cache_dir: &Path) -> Option<ArtworkIndex> {
    let path = cache_dir.join(ARTWORK_INDEX_CACHE);
    let metadata = std::fs::metadata(&path).ok()?;

    // Check TTL
    let age = metadata.modified().ok()?.elapsed().ok()?;
    if age.as_secs() > ARTWORK_INDEX_TTL_DAYS * 86400 {
        return None;
    }

    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn save_artwork_index(cache_dir: &Path, index: &ArtworkIndex) {
    let path = cache_dir.join(ARTWORK_INDEX_CACHE);
    if let Ok(json) = serde_json::to_string(index) {
        let _ = std::fs::write(&path, json);
    }
}

// ---- Show-folder artwork download -------------------------------------------

/// Download image files from a show's Google Drive folder to `{show_dir}/artwork/`.
///
/// Also checks for `covers/` or `artwork/` subfolders within the show folder.
/// Returns paths of downloaded images. Failures are logged but don't propagate.
pub async fn download_show_artwork(
    bman: &BmanApi,
    show_folder_id: &str,
    show_dir: &Path,
) -> Vec<PathBuf> {
    let artwork_dir = show_dir.join("artwork");
    let mut downloaded: Vec<PathBuf> = Vec::new();

    // List show folder contents
    let items = match bman.list_folder(show_folder_id).await {
        Ok(items) => items,
        Err(e) => {
            debug!("Could not list show folder for artwork: {e}");
            return downloaded;
        }
    };

    // Collect image items from the show folder itself
    let mut image_items: Vec<(String, String, u64)> = Vec::new(); // (file_id, name, size)
    let mut cover_subfolder_ids: Vec<String> = Vec::new();

    for item in &items {
        if is_image_file(&item.name) {
            image_items.push((item.id.clone(), item.name.clone(), item.size_bytes()));
        }
        // Check for covers/artwork subfolders
        if item.is_folder() {
            let lower = item.name.to_ascii_lowercase();
            if matches!(lower.as_str(), "covers" | "cover" | "artwork" | "art") {
                cover_subfolder_ids.push(item.id.clone());
            }
        }
    }

    // Also list cover subfolders
    for subfolder_id in &cover_subfolder_ids {
        if let Ok(sub_items) = bman.list_folder(subfolder_id).await {
            for item in &sub_items {
                if is_image_file(&item.name) {
                    image_items.push((item.id.clone(), item.name.clone(), item.size_bytes()));
                }
            }
        }
    }

    if image_items.is_empty() {
        return downloaded;
    }

    // Prioritize: front > cover > date-match > largest
    image_items.sort_by(|a, b| {
        let score = |name: &str| -> i32 {
            let lower = name.to_ascii_lowercase();
            if lower.contains("front") {
                return 3;
            }
            if lower.contains("cover") {
                return 2;
            }
            1
        };
        score(&b.1).cmp(&score(&a.1)).then(b.2.cmp(&a.2))
    });
    image_items.truncate(MAX_IMAGES_PER_SHOW);

    // Create artwork directory
    if let Err(e) = std::fs::create_dir_all(&artwork_dir) {
        warn!("Could not create artwork dir: {e}");
        return downloaded;
    }

    // Download each image
    for (file_id, name, size) in &image_items {
        if *size > MAX_IMAGE_SIZE {
            debug!("Skipping oversized image: {} ({} bytes)", name, size);
            continue;
        }
        let safe_name = crate::models::sanitize::sanitize_filename(name, 200);
        let dest = artwork_dir.join(&safe_name);
        if dest.exists() {
            downloaded.push(dest);
            continue;
        }

        let url = bman.download_url(file_id);
        match bman.client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.bytes().await {
                    Ok(bytes) => {
                        if let Err(e) = std::fs::write(&dest, &bytes) {
                            debug!("Failed to write artwork {}: {e}", safe_name);
                        } else {
                            downloaded.push(dest);
                        }
                    }
                    Err(e) => debug!("Failed to download artwork {}: {e}", safe_name),
                }
            }
            Ok(resp) => debug!("Artwork download {} returned {}", name, resp.status()),
            Err(e) => debug!("Artwork download {} failed: {e}", name),
        }
    }

    if !downloaded.is_empty() {
        debug!("Downloaded {} artwork files to {}", downloaded.len(), artwork_dir.display());
    }
    downloaded
}

// ---- Smart cover selection --------------------------------------------------

/// Select the best cover image from available sources.
///
/// Returns `(image_bytes, is_real_artwork)` — the bool determines save format
/// (jpg for real art, png for generated).
///
/// Priority:
/// 1. Show-folder images with "front" in the name
/// 2. Show-folder images with "cover" in the name
/// 3. Show-folder images matching the show date
/// 4. Largest show-folder image
/// 5. Artwork catalog match by date
/// 6. Generated procedural art
#[allow(clippy::too_many_arguments)]
pub async fn select_best_cover(
    show_dir: &Path,
    show_date: &str,
    artwork_index: &ArtworkIndex,
    bman: &BmanApi,
    artist_name: &str,
    venue_name: &str,
    venue_city: &str,
    venue_state: &str,
) -> Option<(Vec<u8>, bool)> {
    let artwork_dir = show_dir.join("artwork");

    // Source 1: Show-folder images (already downloaded to artwork/)
    if artwork_dir.is_dir() {
        if let Some(bytes) = select_from_local_artwork(&artwork_dir, show_date) {
            return Some((bytes, true));
        }
    }

    // Source 2: Artwork catalog (matched by date)
    if let Some(bytes) = try_download_catalog_art(artwork_index, show_date, bman, &artwork_dir).await {
        return Some((bytes, true));
    }

    // Source 3: Generated procedural art
    #[cfg(feature = "bman")]
    {
        let png = crate::bman::cover::generate_cover(
            artist_name,
            show_date,
            venue_name,
            venue_city,
            venue_state,
        );
        if !png.is_empty() {
            return Some((png, false));
        }
    }

    // Suppress unused variable warnings when bman feature is off
    #[cfg(not(feature = "bman"))]
    {
        let _ = (artist_name, venue_name, venue_city, venue_state);
    }

    None
}

/// Try to download artwork from the catalog index for a given date.
async fn try_download_catalog_art(
    index: &ArtworkIndex,
    show_date: &str,
    bman: &BmanApi,
    artwork_dir: &Path,
) -> Option<Vec<u8>> {
    let entry = index.get(show_date)?.first()?;
    debug!("Using catalog artwork for {}: {}", show_date, entry.name);

    let url = bman.download_url(&entry.file_id);
    let resp = bman.client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;

    // Save to artwork/ for persistence
    let _ = std::fs::create_dir_all(artwork_dir);
    let safe = crate::models::sanitize::sanitize_filename(&entry.name, 200);
    let _ = std::fs::write(artwork_dir.join(&safe), &bytes);

    Some(bytes.to_vec())
}

/// Select the best image from a local artwork directory.
fn select_from_local_artwork(artwork_dir: &Path, show_date: &str) -> Option<Vec<u8>> {
    let Ok(entries) = std::fs::read_dir(artwork_dir) else {
        return None;
    };

    let mut images: Vec<(PathBuf, u64)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.is_file() && is_image_file(&path.to_string_lossy()) {
                let size = e.metadata().ok()?.len();
                Some((path, size))
            } else {
                None
            }
        })
        .collect();

    if images.is_empty() {
        return None;
    }

    // Helper: find the first image whose filename satisfies a predicate.
    let find_by_name = |pred: &dyn Fn(&str) -> bool| -> Option<&PathBuf> {
        images.iter().find_map(|(p, _)| {
            let name = p.file_name()?.to_str()?;
            pred(name).then_some(p)
        })
    };

    // Priority 1: "front" in name
    if let Some(path) = find_by_name(&|n| n.to_ascii_lowercase().contains("front")) {
        return std::fs::read(path).ok();
    }
    // Priority 2: "cover" in name
    if let Some(path) = find_by_name(&|n| n.to_ascii_lowercase().contains("cover")) {
        return std::fs::read(path).ok();
    }
    // Priority 3: show date in filename
    if !show_date.is_empty() {
        if let Some(path) = find_by_name(&|n| n.contains(show_date)) {
            return std::fs::read(path).ok();
        }
    }

    // Priority 4: largest image
    images.sort_by(|a, b| b.1.cmp(&a.1));
    images.first().and_then(|(path, _)| std::fs::read(path).ok())
}

// ---- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_artwork_date_gd() {
        assert_eq!(
            parse_artwork_date("gd77-05-08-Barton-Hall.jpg"),
            Some("1977-05-08".to_string())
        );
    }

    #[test]
    fn test_parse_artwork_date_jgb() {
        assert_eq!(
            parse_artwork_date("jgb80-03-08-Kean-College.jpg"),
            Some("1980-03-08".to_string())
        );
    }

    #[test]
    fn test_parse_artwork_date_case_insensitive() {
        assert_eq!(
            parse_artwork_date("GD73-06-10-RFK-Stadium.jpg"),
            Some("1973-06-10".to_string())
        );
    }

    #[test]
    fn test_parse_artwork_date_invalid() {
        assert_eq!(parse_artwork_date("random-file.jpg"), None);
        assert_eq!(parse_artwork_date("readme.txt"), None);
    }

    #[test]
    fn test_is_image_file() {
        assert!(is_image_file("cover.jpg"));
        assert!(is_image_file("front.JPEG"));
        assert!(is_image_file("art.png"));
        assert!(is_image_file("photo.webp"));
        assert!(!is_image_file("track.flac"));
        assert!(!is_image_file("info.txt"));
    }

    #[test]
    fn test_select_from_local_artwork_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(select_from_local_artwork(dir.path(), "1977-05-08").is_none());
    }

    #[test]
    fn test_select_from_local_artwork_front_priority() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("back.jpg"), b"back-data").unwrap();
        std::fs::write(dir.path().join("front.jpg"), b"front-data").unwrap();
        let result = select_from_local_artwork(dir.path(), "1977-05-08").unwrap();
        assert_eq!(result, b"front-data");
    }

    #[test]
    fn test_select_from_local_artwork_cover_priority() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("liner.jpg"), b"liner-data").unwrap();
        std::fs::write(dir.path().join("cover.jpg"), b"cover-data").unwrap();
        let result = select_from_local_artwork(dir.path(), "1977-05-08").unwrap();
        assert_eq!(result, b"cover-data");
    }

    #[test]
    fn test_select_from_local_artwork_date_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("random.jpg"), b"aa").unwrap();
        std::fs::write(dir.path().join("1977-05-08.jpg"), b"date-match").unwrap();
        let result = select_from_local_artwork(dir.path(), "1977-05-08").unwrap();
        assert_eq!(result, b"date-match");
    }

    #[test]
    fn test_select_from_local_artwork_largest_fallback() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("small.jpg"), b"sm").unwrap();
        std::fs::write(dir.path().join("large.jpg"), b"large-image-data").unwrap();
        let result = select_from_local_artwork(dir.path(), "").unwrap();
        assert_eq!(result, b"large-image-data");
    }
}
