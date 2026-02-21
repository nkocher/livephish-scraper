use std::path::Path;

use lofty::config::WriteOptions;
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, ItemValue, Tag, TagItem};

use crate::models::{Show, Track};

/// Tag an audio file with show and track metadata.
///
/// FLAC files get Vorbis comments (title, artist, album, date, tracknumber,
/// discnumber). M4A/MP4 files get MP4 atoms (©nam, ©ART, ©alb, ©day, trkn, disk).
/// Unsupported formats are silently skipped.
///
/// Note: Python version also writes a custom VENUE Vorbis comment for FLAC.
/// lofty's unified Tag API drops Unknown ItemKeys during save, so venue is
/// omitted. This can be added later with format-specific VorbisComments API.
pub fn tag_track(path: &Path, show: &Show, track: &Track) -> anyhow::Result<()> {
    let mut tagged_file = Probe::open(path)?.read()?;

    let tag = match tagged_file.primary_tag_mut() {
        Some(t) => t,
        None => {
            let tag_type = tagged_file.primary_tag_type();
            tagged_file.insert_tag(Tag::new(tag_type));
            tagged_file.primary_tag_mut().unwrap()
        }
    };

    // Standard fields (mapped automatically by lofty for each format)
    tag.set_title(track.song_title.clone());
    tag.set_artist(show.artist_name.clone());
    tag.set_album(show.container_info.clone());

    // Date → Vorbis DATE / MP4 ©day
    push_text(tag, ItemKey::RecordingDate, show.performance_date.clone());

    // Track number/total → Vorbis TRACKNUMBER / MP4 trkn tuple
    push_text(tag, ItemKey::TrackNumber, track.track_num.to_string());
    push_text(tag, ItemKey::TrackTotal, show.tracks.len().to_string());

    // Disc number/total → Vorbis DISCNUMBER / MP4 disk tuple
    push_text(tag, ItemKey::DiscNumber, track.disc_num.to_string());
    push_text(tag, ItemKey::DiscTotal, "1".to_string());

    tag.save_to_path(path, WriteOptions::default())?;
    Ok(())
}

fn push_text(tag: &mut Tag, key: ItemKey, value: String) {
    tag.push(TagItem::new(key, ItemValue::Text(value)));
}

#[cfg(test)]
mod tests;
