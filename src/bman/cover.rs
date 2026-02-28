use std::path::{Path, PathBuf};

#[cfg(feature = "bman")]
use ab_glyph::{FontVec, PxScale};
#[cfg(feature = "bman")]
use image::{Rgba, RgbaImage};
#[cfg(feature = "bman")]
use imageproc::drawing::{
    draw_filled_circle_mut, draw_filled_rect_mut, draw_hollow_circle_mut, draw_polygon_mut,
    draw_text_mut, text_size,
};
#[cfg(feature = "bman")]
use imageproc::point::Point;
#[cfg(feature = "bman")]
use imageproc::rect::Rect;

#[cfg(feature = "bman")]
struct CoverPalette {
    bg_dark: Rgba<u8>,
    bg_mid: Rgba<u8>,
    accent: Rgba<u8>,
    text_primary: Rgba<u8>,
    text_secondary: Rgba<u8>,
    frame: Rgba<u8>,
}

#[cfg(feature = "bman")]
fn hsl_to_rgba(h: f32, s: f32, l: f32, a: u8) -> Rgba<u8> {
    if s == 0.0 {
        let v = (l * 255.0).clamp(0.0, 255.0) as u8;
        return Rgba([v, v, v, a]);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let h = ((h % 360.0) + 360.0) % 360.0 / 360.0;

    let hue_to_rgb = |mut t: f32| -> f32 {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            return p + (q - p) * 6.0 * t;
        }
        if t < 1.0 / 2.0 {
            return q;
        }
        if t < 2.0 / 3.0 {
            return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
        }
        p
    };

    let r = (hue_to_rgb(h + 1.0 / 3.0) * 255.0).clamp(0.0, 255.0) as u8;
    let g = (hue_to_rgb(h) * 255.0).clamp(0.0, 255.0) as u8;
    let b = (hue_to_rgb(h - 1.0 / 3.0) * 255.0).clamp(0.0, 255.0) as u8;
    Rgba([r, g, b, a])
}

/// Returns (base_hue, complement_hue, saturation, lightness) for a given era.
#[cfg(feature = "bman")]
fn era_palette(year: u16) -> (f32, f32, f32, f32) {
    match year {
        ..=1966 => (40.0, 220.0, 0.65, 0.35),   // Gold/amber
        1967..=1969 => (310.0, 130.0, 0.70, 0.30), // Magenta
        1970..=1974 => (15.0, 195.0, 0.60, 0.30),  // Red
        1975..=1979 => (200.0, 20.0, 0.55, 0.28),  // Blue
        1980..=1984 => (270.0, 90.0, 0.50, 0.30),  // Purple
        1985..=1989 => (225.0, 45.0, 0.65, 0.32),  // Electric blue
        1990..=1995 => (150.0, 330.0, 0.45, 0.28), // Forest
        _ => (60.0, 240.0, 0.40, 0.30),            // Warm olive
    }
}

/// Deterministic hash of date bytes → (hue_offset 0-59, lightness_variance -0.05 to 0.05).
#[cfg(feature = "bman")]
fn show_accent(date: &str) -> (f32, f32) {
    let mut h: u32 = 5381;
    for b in date.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u32);
    }
    let hue_offset = (h % 60) as f32;
    // Rotate+XOR mixes low bytes into high bytes so even 1-day date differences produce
    // distinct lightness values (plain >> 8 leaves low-byte changes invisible).
    let h2 = h.rotate_right(8) ^ h;
    let light_var = (h2 % 100) as f32 / 1000.0 - 0.05;
    (hue_offset, light_var)
}

#[cfg(feature = "bman")]
fn build_palette(year: u16, date: &str, is_jgb: bool) -> CoverPalette {
    let (mut base_hue, comp_hue, mut sat, light) = era_palette(year);
    let (hue_off, light_var) = show_accent(date);

    if is_jgb {
        base_hue = (base_hue + 180.0) % 360.0;
        sat = (sat - 0.20).max(0.15);
    }

    CoverPalette {
        bg_dark: hsl_to_rgba(
            base_hue + hue_off,
            sat * 0.4,
            light * 0.5 + light_var,
            255,
        ),
        bg_mid: hsl_to_rgba(
            base_hue + hue_off,
            sat * 0.5,
            light * 0.7 + light_var,
            255,
        ),
        accent: hsl_to_rgba(comp_hue, sat, light + 0.25 + light_var, 255),
        text_primary: Rgba([240, 240, 235, 255]),
        text_secondary: Rgba([200, 200, 195, 220]),
        frame: hsl_to_rgba(comp_hue, sat * 0.6, light + 0.35, 255),
    }
}

/// Load (heavy, bold) font pair. Returns None if no bold font is found.
#[cfg(feature = "bman")]
fn load_fonts() -> Option<(FontVec, FontVec)> {
    let (heavy_paths, bold_paths): (&[&str], &[&str]) = if cfg!(target_os = "macos") {
        (
            &[
                "/System/Library/Fonts/Supplemental/Arial Black.ttf",
                "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
                "/System/Library/Fonts/Supplemental/Arial.ttf",
            ],
            &[
                "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
                "/System/Library/Fonts/Supplemental/Arial.ttf",
                "/System/Library/Fonts/Helvetica.ttc",
            ],
        )
    } else {
        (
            &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            ],
            &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
            ],
        )
    };

    let load = |paths: &[&str]| -> Option<FontVec> {
        for path in paths {
            if let Ok(data) = std::fs::read(path) {
                if let Ok(font) = FontVec::try_from_vec(data) {
                    return Some(font);
                }
            }
        }
        None
    };

    let bold = load(bold_paths)?;
    // If heavy font unavailable, re-read bold bytes as heavy (can't clone FontVec)
    let heavy = load(heavy_paths).or_else(|| load(bold_paths))?;

    Some((heavy, bold))
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
    let (w, _h) = text_size(PxScale::from(scale), font, text);
    let x = ((600.0 - w as f32) / 2.0).max(10.0) as i32;
    draw_text_mut(img, color, x, y, PxScale::from(scale), font, text);
}

#[cfg(feature = "bman")]
fn draw_centered_text_clamped(
    img: &mut RgbaImage,
    font: &FontVec,
    text: &str,
    scale: f32,
    y: i32,
    color: Rgba<u8>,
    max_width: u32,
) {
    let px = PxScale::from(scale);
    let (w, _) = text_size(px, font, text);
    if w <= max_width {
        draw_centered_text(img, font, text, scale, y, color);
        return;
    }
    // Progressively truncate and append "..." until it fits
    let mut s = text.to_string();
    while !s.is_empty() {
        s.pop();
        let candidate = format!("{}...", s);
        let (cw, _) = text_size(px, font, &candidate);
        if cw <= max_width {
            draw_centered_text(img, font, &candidate, scale, y, color);
            return;
        }
    }
    // Even "..." may not fit — draw it as a last resort
    draw_centered_text(img, font, "...", scale, y, color);
}

#[cfg(feature = "bman")]
fn draw_sunburst(img: &mut RgbaImage, palette: &CoverPalette) {
    use std::f32::consts::PI;
    let cx = 300.0f32;
    let cy = 300.0f32;
    let r = 424.0f32;

    for i in (1u32..18).step_by(2) {
        let angle = (i as f32) * 20.0 * PI / 180.0;
        let next_angle = (i as f32 + 1.0) * 20.0 * PI / 180.0;
        let points = vec![
            Point::new(cx as i32, cy as i32),
            Point::new(
                (cx + r * angle.cos()) as i32,
                (cy + r * angle.sin()) as i32,
            ),
            Point::new(
                (cx + r * next_angle.cos()) as i32,
                (cy + r * next_angle.sin()) as i32,
            ),
        ];
        draw_polygon_mut(img, &points, palette.bg_mid);
    }
}

#[cfg(feature = "bman")]
fn draw_rings(img: &mut RgbaImage, palette: &CoverPalette) {
    for r in [60i32, 120, 180] {
        for dr in [-1i32, 0, 1] {
            draw_hollow_circle_mut(img, (300, 300), r + dr, palette.accent);
        }
    }
}

#[cfg(feature = "bman")]
fn draw_lightning_bolt(img: &mut RgbaImage, color: Rgba<u8>) {
    let points = vec![
        Point::new(285, 220),
        Point::new(310, 220),
        Point::new(305, 270),
        Point::new(330, 270),
        Point::new(295, 340),
        Point::new(300, 310),
        Point::new(275, 310),
        Point::new(310, 240),
        Point::new(305, 260),
        Point::new(280, 260),
        Point::new(300, 220),
    ];
    draw_polygon_mut(img, &points, color);
}

#[cfg(feature = "bman")]
fn draw_jgb_motif(img: &mut RgbaImage, color: Rgba<u8>) {
    draw_filled_circle_mut(img, (300, 300), 50, color);
    for dr in [-1i32, 0, 1] {
        draw_hollow_circle_mut(img, (300, 300), 55 + dr, color);
    }
}

#[cfg(feature = "bman")]
fn draw_text_bands(img: &mut RgbaImage) {
    let darken = |c: u8| (c as f32 * 0.35) as u8;
    for (y_start, y_end) in [(30u32, 110), (255, 345), (460, 565)] {
        for y in y_start..y_end.min(600) {
            for x in 0..600u32 {
                let pixel = img.get_pixel(x, y);
                img.put_pixel(
                    x,
                    y,
                    Rgba([darken(pixel[0]), darken(pixel[1]), darken(pixel[2]), pixel[3]]),
                );
            }
        }
    }
}

#[cfg(feature = "bman")]
fn draw_rect_frame(img: &mut RgbaImage, inset: i32, width: u32, color: Rgba<u8>) {
    let inner = 600 - 2 * inset as u32;
    let w = width as i32;
    draw_filled_rect_mut(img, Rect::at(inset, inset).of_size(inner, width), color);
    draw_filled_rect_mut(img, Rect::at(inset, 600 - inset - w).of_size(inner, width), color);
    draw_filled_rect_mut(img, Rect::at(inset, inset).of_size(width, inner), color);
    draw_filled_rect_mut(img, Rect::at(600 - inset - w, inset).of_size(width, inner), color);
}

#[cfg(feature = "bman")]
fn draw_frame(img: &mut RgbaImage, palette: &CoverPalette) {
    let color = palette.frame;
    draw_rect_frame(img, 12, 3, color);
    draw_rect_frame(img, 20, 1, color);
}

/// Generate cover art PNG bytes for a Bman show.
#[cfg(feature = "bman")]
pub fn generate_cover(artist: &str, date: &str, venue: &str, city: &str, state: &str) -> Vec<u8> {
    let year: u16 = date
        .get(..4)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1975);
    let artist_lower = artist.to_lowercase();
    let is_jgb = artist_lower.contains("jerry") || artist_lower.contains("jgb");

    let palette = build_palette(year, date, is_jgb);

    let mut img = RgbaImage::from_pixel(600, 600, palette.bg_dark);

    // Layer 1: Sunburst rays
    draw_sunburst(&mut img, &palette);
    // Layer 2: Concentric rings
    draw_rings(&mut img, &palette);
    // Layer 3: Central motif
    if is_jgb {
        draw_jgb_motif(&mut img, palette.accent);
    } else {
        draw_lightning_bolt(&mut img, palette.accent);
    }
    // Layer 4: Text bands (semi-transparent darken)
    draw_text_bands(&mut img);
    // Layer 5: Frame
    draw_frame(&mut img, &palette);

    // Layer 6: Text
    if let Some((heavy, bold)) = load_fonts() {
        draw_centered_text_clamped(&mut img, &heavy, date, 52.0, 45, palette.text_primary, 540);
        draw_centered_text_clamped(
            &mut img,
            &bold,
            artist,
            36.0,
            270,
            palette.text_primary,
            540,
        );
        draw_centered_text_clamped(
            &mut img,
            &bold,
            venue,
            26.0,
            475,
            palette.text_primary,
            540,
        );
        let location = if state.is_empty() {
            city.to_string()
        } else {
            format!("{}, {}", city, state)
        };
        draw_centered_text_clamped(
            &mut img,
            &bold,
            &location,
            22.0,
            520,
            palette.text_secondary,
            540,
        );
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

    #[cfg(feature = "bman")]
    #[test]
    fn test_palette_determinism() {
        let p1 = build_palette(1977, "1977-05-08", false);
        let p2 = build_palette(1977, "1977-05-08", false);
        assert_eq!(p1.bg_dark, p2.bg_dark);
        assert_eq!(p1.accent, p2.accent);
    }

    #[cfg(feature = "bman")]
    #[test]
    fn test_palette_year_variation() {
        let p1 = build_palette(1969, "1969-02-28", false);
        let p2 = build_palette(1977, "1977-05-08", false);
        assert_ne!(p1.bg_dark, p2.bg_dark);
    }

    #[cfg(feature = "bman")]
    #[test]
    fn test_palette_show_variation() {
        let p1 = build_palette(1977, "1977-05-08", false);
        let p2 = build_palette(1977, "1977-05-09", false);
        assert_ne!(p1.bg_dark, p2.bg_dark);
    }

    #[cfg(feature = "bman")]
    #[test]
    fn test_gd_vs_jgb_palette_differs() {
        let gd = build_palette(1977, "1977-05-08", false);
        let jgb = build_palette(1977, "1977-05-08", true);
        assert_ne!(gd.bg_dark, jgb.bg_dark);
    }

    #[cfg(feature = "bman")]
    #[test]
    fn test_long_venue_no_panic() {
        let long_venue = "A".repeat(250);
        let png = generate_cover("Grateful Dead", "1977-05-08", &long_venue, "New York", "NY");
        assert!(png.len() > 8);
    }

    #[cfg(feature = "bman")]
    #[test]
    fn test_hsl_to_rgba_basic() {
        // Pure red: H=0, S=1.0, L=0.5
        let red = hsl_to_rgba(0.0, 1.0, 0.5, 255);
        assert_eq!(red[0], 255);
        assert_eq!(red[1], 0);
        assert_eq!(red[2], 0);
        // Black: any H, S=0, L=0
        let black = hsl_to_rgba(0.0, 0.0, 0.0, 255);
        assert_eq!(black, Rgba([0, 0, 0, 255]));
        // White: any H, S=0, L=1
        let white = hsl_to_rgba(0.0, 0.0, 1.0, 255);
        assert_eq!(white, Rgba([255, 255, 255, 255]));
    }

    #[cfg(feature = "bman")]
    #[test]
    fn test_empty_date_no_panic() {
        let png = generate_cover("Grateful Dead", "", "Venue", "City", "ST");
        assert!(png.len() > 8);
    }

    #[cfg(feature = "bman")]
    #[test]
    #[ignore] // Run manually: cargo test gen_sample_covers -- --ignored
    fn gen_sample_covers() {
        let covers: Vec<(&str, &str, &str, &str, &str, &str)> = vec![
            ("Grateful Dead", "1977-05-08", "Barton Hall, Cornell University", "Ithaca", "NY", "/tmp/cover_1977_gd.png"),
            ("Grateful Dead", "1969-02-28", "Fillmore West", "San Francisco", "CA", "/tmp/cover_1969_gd.png"),
            ("Grateful Dead", "1989-07-04", "Buffalo Memorial Auditorium", "Buffalo", "NY", "/tmp/cover_1989_gd.png"),
            ("Jerry Garcia Band", "1977-05-08", "Barton Hall, Cornell University", "Ithaca", "NY", "/tmp/cover_1977_jgb.png"),
        ];
        for (artist, date, venue, city, state, path) in &covers {
            let png = generate_cover(artist, date, venue, city, state);
            std::fs::write(path, &png).unwrap();
            eprintln!("Wrote {} ({} bytes)", path, png.len());
        }
    }
}
