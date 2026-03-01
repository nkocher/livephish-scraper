use once_cell::sync::Lazy;
use regex::Regex;

/// Parsed result from a show folder name.
#[derive(Debug, Clone)]
pub struct FolderInfo {
    pub date: String,   // "YYYY-MM-DD"
    pub artist: String, // "Grateful Dead" or "Jerry Garcia Band"
}

// ---- Compiled regexes (subset of parent bman/parser.rs) ---------------------

static NICE_DATE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d{4}-\d{2}-\d{2})\s+(?:-\s+)?(.+)$").unwrap());

static BARE_DATE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d{4})[.-](\d{2})[.-](\d{2})\s*$").unwrap());

static US_DATE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d{1,2})-(\d{1,2})-(\d{2,4})\s+(?:-\s+)?(.+)$").unwrap());

static ETREE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^(?:gd|jgb|jg)(\d{2,4})[.\-](\d{2})[.\-](\d{2})").unwrap());

static DOT_DATE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d{4})\.(\d{2})\.(\d{2})\s+(.+)$").unwrap());

static NICE_ETREE_HYBRID_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d{4}-\d{2}-\d{2})[.][\w.]+$").unwrap());

static BARE_US_DATE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d{1,2})-(\d{1,2})-(\d{2,4})\s*$").unwrap());

static ARTIST_DASH_PREFIX_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(?:grateful\s+dead|jerry\s+garcia\s+band|jgb|gd)\s+-\s+(.+)$").unwrap()
});

static VOLUME_PREFIX_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(?:[\w'']+\s+)*vol\.?\s*\d+\s*[-–]\s*(.+)$").unwrap()
});

static ARTIST_PREFIX_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(?:grateful\s+dead|jerry\s+garcia\s+band|jgb|gd)\s+(.+)$").unwrap()
});

static ETREE_ARTIST_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^(gd|jgb|jg)").unwrap());

static JGB_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(jerry\s*garcia|jgb)").unwrap());

static DRIVE_ZIP_SUFFIX_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"-\d{8}T\d{6}Z-\d+$").unwrap());

// ---- Public API -------------------------------------------------------------

/// Parse a show folder name into date + artist.
/// Returns None if no date pattern matches.
pub fn parse_folder_name(name: &str) -> Option<FolderInfo> {
    let name = DRIVE_ZIP_SUFFIX_RE.replace(name, "").to_string();
    let name = name.trim();

    // Strip artist/volume prefixes and retry
    let candidates = [
        name.to_string(),
        strip_artist_dash_prefix(name),
        strip_volume_prefix(name),
        strip_artist_prefix(name),
    ];

    for candidate in &candidates {
        let candidate = candidate.trim();
        if candidate.is_empty() {
            continue;
        }
        if let Some(info) = try_parse(candidate, name) {
            return Some(info);
        }
    }

    None
}

fn try_parse(candidate: &str, original: &str) -> Option<FolderInfo> {
    // Nice date: "1977-05-08 Barton Hall"
    if let Some(caps) = NICE_DATE_RE.captures(candidate) {
        let date = caps[1].to_string();
        let artist = resolve_artist(original);
        return Some(FolderInfo { date, artist });
    }

    // Nice-etree hybrid: "1977-05-08.sbd.miller.flac16"
    if let Some(caps) = NICE_ETREE_HYBRID_RE.captures(candidate) {
        let date = caps[1].to_string();
        let artist = resolve_artist(original);
        return Some(FolderInfo { date, artist });
    }

    // Dot date: "1973.06.10 RFK Stadium"
    if let Some(caps) = DOT_DATE_RE.captures(candidate) {
        let date = format!("{}-{}-{}", &caps[1], &caps[2], &caps[3]);
        let artist = resolve_artist(original);
        return Some(FolderInfo { date, artist });
    }

    // Etree: "gd77-04-23.aud.shnf"
    if let Some(caps) = ETREE_RE.captures(candidate) {
        let year = expand_year(&caps[1]);
        let date = format!("{}-{}-{}", year, &caps[2], &caps[3]);
        let artist = resolve_artist_etree(candidate);
        return Some(FolderInfo { date, artist });
    }

    // US date with venue: "6-10-73 RFK Stadium"
    if let Some(caps) = US_DATE_RE.captures(candidate) {
        let year = expand_year(&caps[3]);
        let date = format!("{}-{:0>2}-{:0>2}", year, &caps[1], &caps[2]);
        let artist = resolve_artist(original);
        return Some(FolderInfo { date, artist });
    }

    // Bare US date: "2-26-73"
    if let Some(caps) = BARE_US_DATE_RE.captures(candidate) {
        let year = expand_year(&caps[3]);
        let date = format!("{}-{:0>2}-{:0>2}", year, &caps[1], &caps[2]);
        let artist = resolve_artist(original);
        return Some(FolderInfo { date, artist });
    }

    // Bare date: "1973-06-10" or "1973.06.10"
    if let Some(caps) = BARE_DATE_RE.captures(candidate) {
        let date = format!("{}-{}-{}", &caps[1], &caps[2], &caps[3]);
        let artist = resolve_artist(original);
        return Some(FolderInfo { date, artist });
    }

    None
}

fn resolve_artist(name: &str) -> String {
    if JGB_RE.is_match(name) {
        "Jerry Garcia Band".to_string()
    } else {
        "Grateful Dead".to_string()
    }
}

fn resolve_artist_etree(name: &str) -> String {
    if let Some(caps) = ETREE_ARTIST_RE.captures(name) {
        match caps[1].to_lowercase().as_str() {
            "jgb" | "jg" => "Jerry Garcia Band".to_string(),
            _ => "Grateful Dead".to_string(),
        }
    } else {
        "Grateful Dead".to_string()
    }
}

fn expand_year(year: &str) -> String {
    if year.len() == 4 {
        return year.to_string();
    }
    let y: u32 = year.parse().unwrap_or(0);
    if y >= 60 {
        format!("19{:0>2}", y)
    } else {
        format!("20{:0>2}", y)
    }
}

fn strip_artist_dash_prefix(name: &str) -> String {
    ARTIST_DASH_PREFIX_RE
        .captures(name)
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

fn strip_volume_prefix(name: &str) -> String {
    VOLUME_PREFIX_RE
        .captures(name)
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

fn strip_artist_prefix(name: &str) -> String {
    ARTIST_PREFIX_RE
        .captures(name)
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nice_date() {
        let info = parse_folder_name("1977-05-08 Barton Hall").unwrap();
        assert_eq!(info.date, "1977-05-08");
        assert_eq!(info.artist, "Grateful Dead");
    }

    #[test]
    fn test_etree() {
        let info = parse_folder_name("gd77-04-23.aud.shnf.flac16").unwrap();
        assert_eq!(info.date, "1977-04-23");
        assert_eq!(info.artist, "Grateful Dead");
    }

    #[test]
    fn test_jgb_etree() {
        let info = parse_folder_name("jgb80-03-01.sbd.flac").unwrap();
        assert_eq!(info.date, "1980-03-01");
        assert_eq!(info.artist, "Jerry Garcia Band");
    }

    #[test]
    fn test_dot_date() {
        let info = parse_folder_name("1973.06.10 RFK Stadium").unwrap();
        assert_eq!(info.date, "1973-06-10");
    }

    #[test]
    fn test_us_date() {
        let info = parse_folder_name("6-10-73 RFK Stadium").unwrap();
        assert_eq!(info.date, "1973-06-10");
    }

    #[test]
    fn test_bare_date() {
        let info = parse_folder_name("1973-06-10").unwrap();
        assert_eq!(info.date, "1973-06-10");
    }

    #[test]
    fn test_bare_us_date() {
        let info = parse_folder_name("2-26-73").unwrap();
        assert_eq!(info.date, "1973-02-26");
    }

    #[test]
    fn test_nice_etree_hybrid() {
        let info = parse_folder_name("1972-10-18.miller.sbd.flac16").unwrap();
        assert_eq!(info.date, "1972-10-18");
    }

    #[test]
    fn test_artist_dash_prefix() {
        let info = parse_folder_name("Grateful Dead - 1978-07-05 Red Rocks").unwrap();
        assert_eq!(info.date, "1978-07-05");
        assert_eq!(info.artist, "Grateful Dead");
    }

    #[test]
    fn test_jgb_artist_prefix() {
        let info = parse_folder_name("Jerry Garcia Band - 1980-03-01 Capitol Theatre").unwrap();
        assert_eq!(info.date, "1980-03-01");
        assert_eq!(info.artist, "Jerry Garcia Band");
    }

    #[test]
    fn test_volume_prefix() {
        let info = parse_folder_name("Vol. 03 - 1977-05-08 Barton Hall").unwrap();
        assert_eq!(info.date, "1977-05-08");
    }

    #[test]
    fn test_drive_zip_suffix_stripped() {
        let info = parse_folder_name("1977-05-08 Barton Hall-20250509T025938Z-001").unwrap();
        assert_eq!(info.date, "1977-05-08");
    }

    #[test]
    fn test_unparseable_returns_none() {
        assert!(parse_folder_name("random-folder").is_none());
        assert!(parse_folder_name("artwork").is_none());
    }
}
