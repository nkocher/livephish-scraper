use std::path::Path;

use lofty::config::WriteOptions;
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, ItemValue, TagItem};

/// Patch specific tags on an audio file, preserving all others.
pub fn patch_tags(
    path: &Path,
    title: Option<&str>,
    album_sort: Option<&str>,
    disc_total: Option<u32>,
) -> anyhow::Result<()> {
    let mut tagged_file = Probe::open(path)?.read()?;

    let tag = match tagged_file.primary_tag_mut() {
        Some(t) => t,
        None => {
            let tag_type = tagged_file.primary_tag_type();
            tagged_file.insert_tag(lofty::tag::Tag::new(tag_type));
            tagged_file.primary_tag_mut().unwrap()
        }
    };

    if let Some(title) = title {
        tag.set_title(title.to_string());
    }

    if let Some(sort) = album_sort {
        tag.remove_key(&ItemKey::AlbumTitleSortOrder);
        tag.push(TagItem::new(
            ItemKey::AlbumTitleSortOrder,
            ItemValue::Text(sort.to_string()),
        ));
    }

    if let Some(total) = disc_total {
        tag.remove_key(&ItemKey::DiscTotal);
        tag.push(TagItem::new(
            ItemKey::DiscTotal,
            ItemValue::Text(total.to_string()),
        ));
    }

    tag.save_to_path(path, WriteOptions::default())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_patch_tags_nonexistent_file() {
        let result = patch_tags(Path::new("/nonexistent.flac"), Some("Test"), None, None);
        assert!(result.is_err());
    }
}
