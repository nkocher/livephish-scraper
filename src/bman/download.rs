/// Bman download helpers: build Show and TracksWithUrls from Google Drive.
///
/// These produce the same output types as the nugs/LivePhish path so the
/// existing `download_show()` pipeline works unchanged.
use crate::bman::parser::{parse_disc_subfolder, parse_track_filename};
use crate::bman::setlistfm::{SfmCache, SfmStatus};
use crate::bman::BmanApi;
use crate::download::TracksWithUrls;
use crate::models::{CatalogShow, FormatCode, Quality, Show, Track};

/// Parse FLAC items into tracks, optionally overriding disc_num for multi-disc shows.
///
/// Unmatched FLACs (those that don't match any track filename pattern) are assigned
/// sequential track numbers after the highest parsed track, sorted alphabetically.
pub(crate) fn collect_tracks_from_items(
    items: &[crate::bman::gdrive::DriveItem],
    disc_override: Option<i64>,
    bman: &mut BmanApi,
    tracks: &mut Vec<Track>,
) {
    let mut unmatched_flacs: Vec<&crate::bman::gdrive::DriveItem> = Vec::new();

    for item in items {
        if !item.is_flac() {
            continue;
        }
        if let Some(parsed) = parse_track_filename(&item.name, &item.id) {
            let track_id = bman.id_map.insert(&item.id);
            let disc_num = disc_override.unwrap_or(parsed.disc_num);
            tracks.push(Track {
                track_id,
                song_id: 0,
                song_title: parsed.title,
                track_num: parsed.track_num,
                disc_num,
                set_num: disc_num.max(1),
                duration_seconds: 0,
                duration_display: String::new(),
            });
        } else {
            unmatched_flacs.push(item);
        }
    }

    // Fallback: assign sequential track numbers to unmatched FLACs
    if !unmatched_flacs.is_empty() {
        unmatched_flacs.sort_by(|a, b| a.name.cmp(&b.name));
        let max_track = tracks.iter().map(|t| t.track_num).max().unwrap_or(0);
        let disc = disc_override.unwrap_or(1);
        for (i, item) in unmatched_flacs.iter().enumerate() {
            let track_id = bman.id_map.insert(&item.id);
            let track_num = max_track + 1 + i as i64;
            let title = item
                .name
                .rsplit_once('.')
                .map_or(&*item.name, |(stem, _)| stem)
                .to_string();
            tracks.push(Track {
                track_id,
                song_id: 0,
                song_title: title,
                track_num,
                disc_num: disc,
                set_num: disc.max(1),
                duration_seconds: 0,
                duration_display: String::new(),
            });
        }
        tracing::debug!(
            "Assigned sequential track numbers to {} unmatched FLACs",
            unmatched_flacs.len()
        );
    }
}

/// Fetch show detail from Google Drive folder listing.
///
/// Resolves the synthetic container_id back to a Drive folder via id_map,
/// lists FLAC files, parses track filenames, and builds a full `Show`.
/// Applies cached setlist.fm titles to empty tracks by position so titles
/// flow into `resolve_bman_tracks` → `tracks_with_urls` → filenames + tags.
pub async fn fetch_bman_show_detail(
    bman: &mut BmanApi,
    catalog_show: &CatalogShow,
    sfm_cache: &SfmCache,
) -> Result<Show, String> {
    let container_id = catalog_show.container_id;
    let folder_id = bman
        .id_map
        .get_drive_id(container_id)
        .ok_or_else(|| format!("No Drive folder mapped for container_id {container_id}"))?
        .to_string();

    // List all items in the show folder
    let items = bman
        .list_folder(&folder_id)
        .await
        .map_err(|e| format!("Failed to list show folder: {e}"))?;

    // Check for disc subfolders
    let disc_subfolders: Vec<_> = items
        .iter()
        .filter(|i| i.is_folder())
        .filter_map(|i| parse_disc_subfolder(&i.name).map(|d| (d, i.id.clone())))
        .collect();

    let mut tracks: Vec<Track> = Vec::new();

    if disc_subfolders.is_empty() {
        // Single-disc show: parse FLAC files directly
        collect_tracks_from_items(&items, None, bman, &mut tracks);
    } else {
        // Multi-disc: list FLAC files inside each disc subfolder
        let mut sorted_discs = disc_subfolders;
        sorted_discs.sort_by_key(|(disc_num, _)| *disc_num);

        for (disc_num, subfolder_id) in &sorted_discs {
            let disc_items = bman
                .list_folder(subfolder_id)
                .await
                .map_err(|e| format!("Failed to list disc folder: {e}"))?;

            collect_tracks_from_items(&disc_items, Some(*disc_num), bman, &mut tracks);
        }
    }

    // Sort by disc then track number
    tracks.sort_by_key(|t| (t.disc_num, t.track_num));

    // Apply cached setlist.fm titles to empty tracks by position.
    // This covers 917/928 shows so titles are available before the clone
    // in resolve_bman_tracks, fixing filenames like "01. .m4a".
    if let Some(SfmStatus::Found { songs, .. }) =
        sfm_cache.lookup(&catalog_show.artist_name, &catalog_show.performance_date)
    {
        for (i, track) in tracks.iter_mut().enumerate() {
            if track.song_title.is_empty() {
                if let Some(title) = songs.get(i) {
                    if !title.is_empty() {
                        track.song_title = title.clone();
                    }
                }
            }
        }
    }

    Ok(Show {
        container_id,
        artist_name: catalog_show.artist_name.clone(),
        container_info: catalog_show.container_info.clone(),
        venue_name: catalog_show.venue_name.clone(),
        venue_city: catalog_show.venue_city.clone(),
        venue_state: catalog_show.venue_state.clone(),
        performance_date: catalog_show.performance_date.clone(),
        performance_date_formatted: catalog_show.performance_date_formatted.clone(),
        performance_date_year: catalog_show.performance_date_year.clone(),
        artist_id: catalog_show.artist_id,
        total_duration_seconds: 0,
        total_duration_display: String::new(),
        tracks,
        image_url: catalog_show.image_url.clone(),
    })
}

/// Build TracksWithUrls for Bman show — direct Google Drive download URLs.
///
/// Quality is always FLAC. No stream parameter resolution needed.
/// When OAuth is configured, URLs are bare (no API key) and a Bearer token
/// is returned for use in the Authorization header. Falls back to API-key
/// URLs when OAuth is not available.
pub async fn resolve_bman_tracks(show: &Show, bman: &BmanApi) -> (TracksWithUrls, Option<String>) {
    let quality = Quality::from_format_code(FormatCode::Flac);

    // Get bearer token once per show (valid ~1h, a show takes minutes).
    // With OAuth: bare URLs (auth via Authorization header).
    // Without OAuth: API-key URLs (no header needed).
    let bearer = bman
        .oauth_access_token()
        .await
        .map(|t| format!("Bearer {t}"));

    let tracks: TracksWithUrls = show
        .tracks
        .iter()
        .filter_map(|track| {
            let drive_id = bman.id_map.get_drive_id(track.track_id)?;
            let url = if bearer.is_some() {
                format!("{}/files/{}?alt=media", bman.drive_base_url, drive_id)
            } else {
                bman.download_url(drive_id)
            };
            Some((track.clone(), url, quality.clone()))
        })
        .collect();

    (tracks, bearer)
}

/// Sync enriched song titles from show.tracks back to tracks_with_urls.
///
/// After `bman_enrich_metadata` enriches `show.tracks` (via Vorbis comments,
/// info files, setlist.fm, or fallback), the separate `tracks_with_urls` clone
/// still has stale titles. This copies the final titles back so `download_show`
/// uses correct titles for filenames and audio tags.
pub fn sync_enriched_titles(show: &Show, tracks_with_urls: &mut TracksWithUrls) {
    for (track, _, _) in tracks_with_urls.iter_mut() {
        if let Some(enriched) = show.tracks.iter().find(|t| t.track_id == track.track_id) {
            if track.song_title != enriched.song_title {
                track.song_title = enriched.song_title.clone();
            }
        }
    }
}

/// Effective flac_convert for Bman: defaults to "aac" unless user explicitly set something else.
pub fn bman_flac_convert(config_flac_convert: &str) -> &str {
    if config_flac_convert.is_empty() || config_flac_convert == "none" {
        "aac"
    } else {
        config_flac_convert
    }
}

/// Enrich track titles via the metadata pipeline (setlist.fm, info files, etc.)
///
/// Run this BEFORE `download_show()` so the enriched titles are used for both
/// filenames and audio tags. Steps that need downloaded files (Vorbis comments)
/// are no-ops at this point and that's acceptable — setlist.fm provides the
/// most valuable title data for GD/JGB shows.
pub async fn bman_enrich_metadata(
    show: &mut crate::models::Show,
    output_dir: &std::path::Path,
    sfm_keys: &crate::bman::setlistfm::SetlistFmKeys,
    sfm_cache: &mut crate::bman::setlistfm::SfmCache,
) {
    let show_dir = output_dir.join(show.folder_name());
    // Create the show dir early so info-file step can scan it (usually empty at this point)
    let _ = std::fs::create_dir_all(&show_dir);

    if let Err(e) = crate::bman::metadata::resolve_metadata(&show_dir, show, sfm_keys, sfm_cache).await {
        tracing::warn!("Metadata enrichment: {e}");
    }
}

/// Download artwork and save cover art for a Bman show, then embed in audio files.
///
/// Run AFTER `download_show()` completes so the audio files exist for embedding.
/// Uses 3-source priority: show-folder images → artwork catalog → generated art.
pub async fn bman_save_cover_art(
    show: &crate::models::Show,
    output_dir: &std::path::Path,
    bman: &super::BmanApi,
) {
    let show_dir = output_dir.join(show.folder_name());

    // Skip if cover already exists
    if show_dir.join("cover.jpg").exists() || show_dir.join("cover.png").exists() {
        return;
    }

    // Try to resolve the show's Drive folder ID for artwork download
    let folder_id = bman.id_map.get_drive_id(show.container_id);
    if let Some(fid) = folder_id {
        super::artwork::download_show_artwork(bman, fid, &show_dir).await;
    }

    // Select best cover from all sources
    let cover = super::artwork::select_best_cover(
        &show_dir,
        &show.performance_date,
        &bman.artwork_index,
        bman,
        &show.artist_name,
        &show.venue_name,
        &show.venue_city,
        &show.venue_state,
    )
    .await;

    let Some((bytes, is_real_art)) = cover else {
        return;
    };

    // Save cover file: jpg for real art, png for generated
    let ext = if is_real_art { "cover.jpg" } else { "cover.png" };
    let cover_path = show_dir.join(ext);
    if let Err(e) = std::fs::write(&cover_path, &bytes) {
        tracing::warn!("Cover art save: {e}");
        return;
    }

    // Embed in all audio files
    if let Ok(entries) = std::fs::read_dir(&show_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_audio = path.is_file()
                && path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|ext| matches!(ext, "m4a" | "flac"));
            if is_audio {
                if let Err(e) = crate::tagger::embed_cover_art(&path, &bytes) {
                    tracing::debug!("Cover embed {}: {e}", path.display());
                }
            }
        }
    }
}
