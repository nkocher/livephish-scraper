use std::path::Path;

use lofty::config::WriteOptions;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::probe::Probe;

use crate::scanner;

const MAX_COVER_SIZE: u64 = 10 * 1024 * 1024; // 10 MB

/// Re-embed cover artwork into all audio files in a show directory.
/// Returns count of files updated.
pub fn reembed_artwork(show_dir: &Path) -> anyhow::Result<u32> {
    let cover_path = match scanner::find_cover_file(show_dir) {
        Some(p) => p,
        None => anyhow::bail!("No cover file found in {}", show_dir.display()),
    };

    let metadata = std::fs::metadata(&cover_path)?;
    if metadata.len() > MAX_COVER_SIZE {
        anyhow::bail!(
            "Cover file too large ({:.1} MB, max 10 MB): {}",
            metadata.len() as f64 / 1024.0 / 1024.0,
            cover_path.display()
        );
    }

    let data = std::fs::read(&cover_path)?;
    let mime = detect_image_mime(&data)?;

    let mut count = 0u32;
    embed_in_dir(show_dir, &data, mime.clone(), &mut count)?;

    // Also check disc subfolders
    if let Ok(entries) = std::fs::read_dir(show_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                embed_in_dir(&path, &data, mime.clone(), &mut count)?;
            }
        }
    }

    Ok(count)
}

fn embed_in_dir(dir: &Path, data: &[u8], mime: MimeType, count: &mut u32) -> anyhow::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if !(name.ends_with(".flac") || name.ends_with(".m4a")) {
            continue;
        }

        if let Err(e) = embed_cover(&path, data, mime.clone()) {
            eprintln!("  Warning: failed to embed artwork in {}: {}", path.display(), e);
            continue;
        }
        *count += 1;
    }

    Ok(())
}

fn embed_cover(path: &Path, data: &[u8], mime: MimeType) -> anyhow::Result<()> {
    let mut tagged_file = Probe::open(path)?.read()?;

    let tag = match tagged_file.primary_tag_mut() {
        Some(t) => t,
        None => {
            let tag_type = tagged_file.primary_tag_type();
            tagged_file.insert_tag(lofty::tag::Tag::new(tag_type));
            tagged_file.primary_tag_mut().unwrap()
        }
    };

    // Remove existing cover art
    tag.remove_picture_type(PictureType::CoverFront);

    let picture = Picture::new_unchecked(PictureType::CoverFront, Some(mime), None, data.to_vec());
    tag.push_picture(picture);
    tag.save_to_path(path, WriteOptions::default())?;
    Ok(())
}

/// Detect JPEG vs PNG from magic bytes.
fn detect_image_mime(data: &[u8]) -> anyhow::Result<MimeType> {
    if data.len() < 4 {
        anyhow::bail!("Image file too small to detect format");
    }
    if data[0] == 0xFF && data[1] == 0xD8 {
        Ok(MimeType::Jpeg)
    } else if data[0] == 0x89 && data[1] == 0x50 && data[2] == 0x4E && data[3] == 0x47 {
        Ok(MimeType::Png)
    } else {
        anyhow::bail!("Unsupported image format (not JPEG or PNG)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_jpeg() {
        let data = [0xFF, 0xD8, 0xFF, 0xE0, 0x00];
        assert!(matches!(detect_image_mime(&data).unwrap(), MimeType::Jpeg));
    }

    #[test]
    fn test_detect_png() {
        let data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A];
        assert!(matches!(detect_image_mime(&data).unwrap(), MimeType::Png));
    }

    #[test]
    fn test_detect_unknown() {
        let data = [0x00, 0x00, 0x00, 0x00];
        assert!(detect_image_mime(&data).is_err());
    }

    #[test]
    fn test_detect_too_small() {
        let data = [0xFF, 0xD8];
        // Only 2 bytes, rejected by the length check
        assert!(detect_image_mime(&data).is_err());
    }

    #[test]
    fn test_reembed_no_cover() {
        let tmp = tempfile::tempdir().unwrap();
        let result = reembed_artwork(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_reembed_oversized_cover() {
        let tmp = tempfile::tempdir().unwrap();
        let cover = tmp.path().join("cover.jpg");
        // Write >10MB of data
        let data = vec![0xFF; 11 * 1024 * 1024];
        std::fs::write(&cover, &data).unwrap();
        let result = reembed_artwork(tmp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too large"));
    }
}
