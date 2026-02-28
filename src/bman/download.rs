/// Bman download helpers: build Show and TracksWithUrls from Google Drive.
///
/// These produce the same output types as the nugs/LivePhish path so the
/// existing `download_show()` pipeline works unchanged.
use crate::bman::parser::{parse_disc_subfolder, parse_track_filename};
use crate::bman::BmanApi;
use crate::download::TracksWithUrls;
use crate::models::{CatalogShow, FormatCode, Quality, Show, Track};

/// Parse FLAC items into tracks, optionally overriding disc_num for multi-disc shows.
pub(crate) fn collect_tracks_from_items(
    items: &[crate::bman::gdrive::DriveItem],
    disc_override: Option<i64>,
    bman: &mut BmanApi,
    tracks: &mut Vec<Track>,
) {
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
        }
    }
}

/// Fetch show detail from Google Drive folder listing.
///
/// Resolves the synthetic container_id back to a Drive folder via id_map,
/// lists FLAC files, parses track filenames, and builds a full `Show`.
pub async fn fetch_bman_show_detail(
    bman: &mut BmanApi,
    catalog_show: &CatalogShow,
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
pub fn resolve_bman_tracks(show: &Show, bman: &BmanApi) -> TracksWithUrls {
    let quality = Quality::from_format_code(FormatCode::Flac);

    show.tracks
        .iter()
        .filter_map(|track| {
            let drive_id = bman.id_map.get_drive_id(track.track_id)?;
            let url = bman.download_url(drive_id);
            Some((track.clone(), url, quality.clone()))
        })
        .collect()
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
    setlistfm_api_key: &str,
) {
    let show_dir = output_dir.join(show.folder_name());
    // Create the show dir early so info-file step can scan it (usually empty at this point)
    let _ = std::fs::create_dir_all(&show_dir);

    if let Err(e) =
        crate::bman::metadata::resolve_metadata(&show_dir, show, setlistfm_api_key).await
    {
        tracing::warn!("Metadata enrichment: {e}");
    }
}

/// Generate and save cover art for a Bman show, then embed it in audio files.
///
/// Run AFTER `download_show()` completes so the audio files exist for embedding.
#[cfg(feature = "bman")]
pub fn bman_save_cover_art(show: &crate::models::Show, output_dir: &std::path::Path) {
    let show_dir = output_dir.join(show.folder_name());
    let png = super::cover::generate_cover(
        &show.artist_name,
        &show.performance_date,
        &show.venue_name,
        &show.venue_city,
        &show.venue_state,
    );
    if png.is_empty() {
        return;
    }
    if let Err(e) = super::cover::save_cover(&show_dir, &png) {
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
                    .is_some_and(|ext| ext == "m4a" || ext == "flac");
            if is_audio {
                if let Err(e) = crate::tagger::embed_cover_art(&path, &png) {
                    tracing::debug!("Cover embed {}: {e}", path.display());
                }
            }
        }
    }
}

/// No-op when bman feature is disabled.
#[cfg(not(feature = "bman"))]
pub fn bman_save_cover_art(_show: &crate::models::Show, _output_dir: &std::path::Path) {}
