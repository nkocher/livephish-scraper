use std::path::{Path, PathBuf};

#[cfg(feature = "bman")]
use ab_glyph::{FontVec, PxScale};
#[cfg(feature = "bman")]
use image::{Rgba, RgbaImage};
#[cfg(feature = "bman")]
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut};
#[cfg(feature = "bman")]
use imageproc::rect::Rect;

#[cfg(feature = "bman")]
fn load_system_font() -> Option<FontVec> {
    let candidates = if cfg!(target_os = "macos") {
        vec![
            "/System/Library/Fonts/HelveticaNeue.ttc",
            "/System/Library/Fonts/Helvetica.ttc",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        ]
    } else {
        vec![
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        ]
    };
    for path in candidates {
        if let Ok(data) = std::fs::read(path) {
            if let Ok(font) = FontVec::try_from_vec(data) {
                return Some(font);
            }
        }
    }
    None
}

#[cfg(feature = "bman")]
fn draw_centered_text(
    img: &mut RgbaImage,
    font: &FontVec,
    text: &str,
    scale: f32,
    y: i32,
    color: Rgba<u8>,
) {
    let width = img.width() as f32;
    let text_width = text.len() as f32 * scale * 0.5;
    let x = ((width - text_width) / 2.0).max(0.0) as i32;
    draw_text_mut(img, color, x, y, PxScale::from(scale), font, text);
}

/// Generate cover art PNG bytes for a Bman show.
#[cfg(feature = "bman")]
pub fn generate_cover(artist: &str, date: &str, venue: &str, city: &str, state: &str) -> Vec<u8> {
    let size = 600u32;
    let mut img = RgbaImage::from_pixel(size, size, Rgba([30u8, 30, 30, 255]));

    // 2px white border inset by 20px from each edge
    let border_color = Rgba([255u8, 255, 255, 255]);
    let inset = 20i32;
    let border = 2u32;
    let inner_size = size - 2 * inset as u32;

    // Top edge
    draw_filled_rect_mut(
        &mut img,
        Rect::at(inset, inset).of_size(inner_size, border),
        border_color,
    );
    // Bottom edge
    draw_filled_rect_mut(
        &mut img,
        Rect::at(inset, size as i32 - inset - border as i32).of_size(inner_size, border),
        border_color,
    );
    // Left edge
    draw_filled_rect_mut(
        &mut img,
        Rect::at(inset, inset).of_size(border, inner_size),
        border_color,
    );
    // Right edge
    draw_filled_rect_mut(
        &mut img,
        Rect::at(size as i32 - inset - border as i32, inset).of_size(border, inner_size),
        border_color,
    );

    let text_color = Rgba([240u8, 240, 240, 255]);

    if let Some(font) = load_system_font() {
        // Artist name: ~40px, top third
        draw_centered_text(&mut img, &font, artist, 40.0, 150, text_color);
        // Date: ~30px, middle
        draw_centered_text(&mut img, &font, date, 30.0, 280, text_color);
        // Venue: ~22px, lower third
        draw_centered_text(&mut img, &font, venue, 22.0, 370, text_color);
        // City, ST: ~22px
        let location = if state.is_empty() {
            city.to_string()
        } else {
            format!("{}, {}", city, state)
        };
        draw_centered_text(&mut img, &font, &location, 22.0, 410, text_color);
    }

    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    img.write_to(&mut cursor, image::ImageFormat::Png)
        .unwrap_or_default();
    buf
}

/// No-op when bman feature is disabled.
#[cfg(not(feature = "bman"))]
pub fn generate_cover(
    _artist: &str,
    _date: &str,
    _venue: &str,
    _city: &str,
    _state: &str,
) -> Vec<u8> {
    Vec::new()
}

/// Save cover.png to the given directory. Returns path to saved file.
pub fn save_cover(dir: &Path, png_data: &[u8]) -> std::io::Result<PathBuf> {
    let path = dir.join("cover.png");
    std::fs::write(&path, png_data)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[cfg(feature = "bman")]
    #[test]
    fn test_generate_cover_png_magic_bytes() {
        let png = generate_cover("Bman", "2024-06-15", "Madison Square Garden", "New York", "NY");
        assert!(png.len() > 8, "PNG should not be empty");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "Should have PNG magic bytes");
    }

    #[cfg(feature = "bman")]
    #[test]
    fn test_generate_cover_dimensions() {
        let png = generate_cover("Bman", "2024-06-15", "The Venue", "Austin", "TX");
        let img = image::load_from_memory(&png).expect("Should decode as valid PNG");
        assert_eq!(img.width(), 600);
        assert_eq!(img.height(), 600);
    }

    #[cfg(not(feature = "bman"))]
    #[test]
    fn test_generate_cover_no_feature_returns_empty() {
        let result = generate_cover("Bman", "2024-06-15", "Venue", "City", "ST");
        assert!(result.is_empty());
    }

    #[test]
    fn test_save_cover_writes_file() {
        let dir = tempdir().unwrap();
        let png_data = b"\x89PNG\r\n\x1a\nfakedata";
        let path = save_cover(dir.path(), png_data).unwrap();
        assert!(path.exists());
        assert_eq!(path.file_name().unwrap(), "cover.png");
        assert_eq!(std::fs::read(&path).unwrap(), png_data);
    }
}
