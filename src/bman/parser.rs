use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;

/// Grateful Dead artist ID (matches nugs.net).
pub const BMAN_GD_ARTIST_ID: i64 = 461;

/// Jerry Garcia Band artist ID (negative, can't collide with nugs).
pub const BMAN_JGB_ARTIST_ID: i64 = -1;

/// Which artist a Bman show belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BmanArtist {
    GratefulDead,
    JerryGarciaBand,
}

impl BmanArtist {
    pub fn artist_id(self) -> i64 {
        match self {
            BmanArtist::GratefulDead => BMAN_GD_ARTIST_ID,
            BmanArtist::JerryGarciaBand => BMAN_JGB_ARTIST_ID,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            BmanArtist::GratefulDead => "Grateful Dead",
            BmanArtist::JerryGarciaBand => "Jerry Garcia Band",
        }
    }
}

/// Recording source quality type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceType {
    Unknown = 0,
    Aud = 1,
    Mtx = 2,
    Sbd = 3,
}

/// A parsed Bman show folder.
#[derive(Debug, Clone)]
pub struct ParsedShow {
    pub date: String,       // "YYYY-MM-DD"
    pub venue: String,
    pub city: String,
    pub state: String,
    pub source_type: SourceType,
    pub source_tag: String, // raw text in parens
    pub is_nll: bool,
    pub artist: BmanArtist,
    pub folder_id: String,  // Google Drive folder ID
}

/// A parsed Bman track file.
#[derive(Debug, Clone)]
pub struct ParsedTrack {
    pub track_num: i64,
    pub disc_num: i64,
    pub title: String,
    #[allow(dead_code)] // stored for debugging and future use
    pub file_id: String,
}

// ---- Compiled regexes -------------------------------------------------------

static NICE_DATE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d{4}-\d{2}-\d{2})\s+(?:-\s+)?(.+)$").unwrap());

static SOURCE_TAG_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[\[(]([^)\]]+)[\])]\s*$").unwrap());

static BARE_DATE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d{4})[.-](\d{2})[.-](\d{2})\s*$").unwrap());

static US_DATE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d{1,2})-(\d{1,2})-(\d{2,4})\s+(?:-\s+)?(.+)$").unwrap());

static ETREE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^(?:gd|jgb|jg)(\d{2,4})[.\-](\d{2})[.\-](\d{2})").unwrap());

static ETREE_ARTIST_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^(gd|jgb|jg)").unwrap());

static SOURCE_TYPE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(sbeok|sbd|betty\s*board|audience|aud|fob|matrix|mtx)\b").unwrap());

// Track number, separator (dot/dash/paren with optional spaces, OR just spaces), title, .flac.
// Matches: "01. Title.flac", "01-Title.flac", "01 Title.flac", "01 - Title.flac"
static NICE_TRACK_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^(\d+)(?:\s*[-.)]\s*|\s+)(.+)\.flac$").unwrap());

// Matches d#t## or s#t## anywhere in the filename, with optional embedded title after.
// Non-letter boundary before [sd] prevents matching words ending in s/d (e.g., "birds1t02").
// Boundary is [^a-z] NOT [^a-z0-9] — digits must be allowed before s/d
// because real filenames have "gd77-04-23d1t01.flac" where '3' precedes 'd'.
static ETREE_TRACK_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(?:^|[^a-z])[sd](\d+)t(\d+)(?:\s+(.+))?\.flac$").unwrap());

// Longer prefixes (disc/disk) before the single-char 'd'.
static DISC_SUBFOLDER_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^(?:disc|disk|cd|set|d)\s*(\d+)$").unwrap());

static YEAR_RANGE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b(196[5-9]|19[7-9]\d|200\d|201\d|202[0-5])\b").unwrap());

static JGB_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(jerry\s*garcia|jgb)").unwrap());

static DOT_DATE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d{4})\.(\d{2})\.(\d{2})\s+(.+)$").unwrap());

/// Matches parenthesized etree-style metadata like "(sbd.droncit.127478)".
static PAREN_ETREE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\([\w.]+\)\s*").unwrap());

/// Matches standalone source/format/quality/editorial words anywhere in location text.
/// NOTE: bare `Matrix` is NOT stripped (it's a real venue). Only taper-qualified
/// forms like "Dusborn Matrix" or "Photoleon Matrix" are stripped.
static LOCATION_JUNK_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r"(?i)\b(",
        // Source/format/quality
        r"sbd|aud|audience|mtx|sbeok|betty\s*board|flac\d*|mp3|wav|1st\s+gen|c\.\s*miller",
        // Editorial
        r"|incredible|amazing|excellent|holy\s*shit|rip",
        // Remaster notes
        r"|remaster(?:ed)?|dan\s+wainwright\s+remaster",
        // Set notes
        r"|set\s*\d+|1st\s+set|2nd\s+set|early|late",
        // Named taper patterns (matrix/board qualified by taper name)
        r"|(?:dusborn|photoleon|betty)\s+(?:matrix|board)",
        r")\b",
    ))
    .unwrap()
});

/// Nice date followed by etree-style dot metadata: `YYYY-MM-DD.stuff.stuff`
static NICE_ETREE_HYBRID_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d{4}-\d{2}-\d{2})[.][\w.]+$").unwrap());

/// Bare US date without venue: `M-DD-YY` or `MM-DD-YYYY` with nothing after.
static BARE_US_DATE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(\d{1,2})-(\d{1,2})-(\d{2,4})\s*$").unwrap());

/// Artist prefix with dash separator: `Grateful Dead - YYYY-MM-DD ...`
static ARTIST_DASH_PREFIX_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(?:grateful\s+dead|jerry\s+garcia\s+band|jgb|gd)\s+-\s+(.+)$").unwrap()
});

/// Volume/series prefix: `Vol. 03 - ...` or `Hunter's Trix Vol. 048 - ...`
static VOLUME_PREFIX_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(?:[\w'']+\s+)*vol\.?\s*\d+\s*[-–]\s*(.+)$").unwrap()
});

static ARTIST_PREFIX_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(?:grateful\s+dead|jerry\s+garcia\s+band|jgb|gd)\s+(.+)$").unwrap()
});

static SKIP_FOLDER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(VIDEO_TS|artwork|extras|bonus|info|text|liner\s*notes|covers?|set\s*\d+|disc\s*\d+|disk\s*\d+|cd\s*\d+|d\d+|bman.?s?\s*picks.*)$").unwrap()
});

/// Google Drive zip-download timestamp suffix: `-20250509T025938Z-001`
static DRIVE_ZIP_SUFFIX_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"-\d{8}T\d{6}Z-\d+$").unwrap());

// ---- Helpers ----------------------------------------------------------------

/// Override artist to JGB if the folder name mentions Jerry Garcia / JGB.
fn resolve_artist(name: &str, default: BmanArtist) -> BmanArtist {
    if JGB_RE.is_match(name) {
        BmanArtist::JerryGarciaBand
    } else {
        default
    }
}

/// Detect source type from arbitrary text (folder name, source tag, etc.)
fn detect_source_type(text: &str) -> SourceType {
    let Some(cap) = SOURCE_TYPE_RE.captures(text) else {
        return SourceType::Unknown;
    };
    let matched = cap[1].to_lowercase();
    if matched.starts_with("sbeok") || matched.starts_with("sbd") || matched.contains("betty") {
        SourceType::Sbd
    } else if matched.starts_with("aud") || matched == "fob" {
        SourceType::Aud
    } else {
        // mtx / matrix
        SourceType::Mtx
    }
}

/// Split "venue, city, ST" into (venue, city, state).
fn parse_venue_city_state(location: &str) -> (String, String, String) {
    let parts: Vec<&str> = location
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    match parts.len() {
        0 => (String::new(), String::new(), String::new()),
        1 => (parts[0].to_string(), String::new(), String::new()),
        2 => (parts[0].to_string(), String::new(), parts[1].to_string()),
        _ => {
            let state = parts[parts.len() - 1].to_string();
            let city = parts[parts.len() - 2].to_string();
            let venue = parts[..parts.len() - 2].join(", ");
            (venue, city, state)
        }
    }
}

/// Expand a 2- or 4-digit year string to 4 digits.
/// 2-digit: >=50 → 1900s, <50 → 2000s.
fn expand_year(year: &str) -> String {
    if year.len() <= 2 {
        let y: u32 = year.parse().unwrap_or(0);
        if y >= 50 {
            format!("19{:02}", y)
        } else {
            format!("20{:02}", y)
        }
    } else {
        year.to_string()
    }
}

/// Strip source/format/quality junk from location text.
///
/// Multi-pass cleanup:
/// 1. Strip parenthesized etree metadata like `(sbd.droncit.127478)`
/// 2. Strip standalone source/format/quality words (SBD, FLAC, INCREDIBLE, etc.)
/// 3. Clean up resulting whitespace, dashes, commas
/// 4. Return empty string if nothing meaningful remains
fn clean_location(text: &str) -> String {
    // Step 1: Strip parenthesized etree metadata at start or end
    let text = PAREN_ETREE_RE.replace_all(text, " ");
    let text = text.trim();

    // Step 2: Strip standalone junk words
    let text = LOCATION_JUNK_RE.replace_all(text, " ");

    // Step 2b: Strip orphaned square brackets (e.g., leftover from "[SBD FLAC]")
    let text = text.replace(['[', ']'], " ");

    // Step 3: Clean up whitespace and separators
    let text = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    // Strip leading/trailing dashes, commas, dots, whitespace
    let text = text.trim_matches(|c: char| c == '-' || c == '–' || c == ',' || c == '.' || c.is_whitespace());

    // Step 4: If only whitespace/punctuation remains, return empty
    if text.chars().all(|c| !c.is_alphanumeric()) {
        String::new()
    } else {
        text.to_string()
    }
}

// ---- Pattern parsers --------------------------------------------------------

fn try_parse_nice(
    name: &str,
    folder_id: &str,
    artist: BmanArtist,
    is_nll: bool,
) -> Option<ParsedShow> {
    let cap = NICE_DATE_RE.captures(name)?;
    let date = cap[1].to_string();
    let rest = cap[2].trim().to_string();

    let (location, source_tag) = if let Some(sc) = SOURCE_TAG_RE.captures(&rest) {
        let tag = sc[1].trim().to_string();
        let location = rest[..sc.get(0).unwrap().start()].trim().to_string();
        (location, tag)
    } else {
        (rest, String::new())
    };

    // Detect source type from location text BEFORE cleanup
    let location_source_type = detect_source_type(&location);
    // Strip all source/format/quality junk from location text
    let location = clean_location(&location);

    let (venue, city, state) = parse_venue_city_state(&location);
    let source_type = detect_source_type(&source_tag)
        .max(detect_source_type(name))
        .max(location_source_type);
    let show_is_nll = is_nll || source_tag.to_lowercase().contains("nll");

    Some(ParsedShow {
        date,
        venue,
        city,
        state,
        source_type,
        source_tag,
        is_nll: show_is_nll,
        artist: resolve_artist(name, artist),
        folder_id: folder_id.to_string(),
    })
}

fn try_parse_mm_dd_yyyy(
    name: &str,
    folder_id: &str,
    artist: BmanArtist,
    is_nll: bool,
) -> Option<ParsedShow> {
    let cap = US_DATE_RE.captures(name)?;
    let month: u32 = cap[1].parse().ok()?;
    let day: u32 = cap[2].parse().ok()?;
    let year = expand_year(&cap[3]);
    let rest = cap[4].trim();
    let date = format!("{}-{:02}-{:02}", year, month, day);

    // Split on " - ": first part is venue, remaining is "city, ST"
    let parts: Vec<&str> = rest.splitn(3, " - ").collect();
    let venue = parts.first().unwrap_or(&"").trim().to_string();
    let location = parts.get(1..).map(|p| p.join(" - ")).unwrap_or_default();

    let (city, state) = if let Some(pos) = location.rfind(',') {
        let city = location[..pos].trim().to_string();
        let state = location[pos + 1..].trim().to_string();
        (city, state)
    } else {
        (location, String::new())
    };

    let source_type = detect_source_type(name);

    Some(ParsedShow {
        date,
        venue,
        city,
        state,
        source_type,
        source_tag: String::new(),
        is_nll,
        artist: resolve_artist(name, artist),
        folder_id: folder_id.to_string(),
    })
}

fn try_parse_bare_date(
    name: &str,
    folder_id: &str,
    artist: BmanArtist,
    is_nll: bool,
) -> Option<ParsedShow> {
    let cap = BARE_DATE_RE.captures(name)?;
    let date = format!("{}-{}-{}", &cap[1], &cap[2], &cap[3]);

    Some(ParsedShow {
        date,
        venue: String::new(),
        city: String::new(),
        state: String::new(),
        source_type: SourceType::Unknown,
        source_tag: String::new(),
        is_nll,
        artist: resolve_artist(name, artist),
        folder_id: folder_id.to_string(),
    })
}

fn try_parse_etree(name: &str, folder_id: &str, is_nll: bool) -> Option<ParsedShow> {
    let cap = ETREE_RE.captures(name)?;
    let artist_prefix = ETREE_ARTIST_RE.captures(name)?[1].to_lowercase();
    let artist = if artist_prefix == "jgb" || artist_prefix == "jg" {
        BmanArtist::JerryGarciaBand
    } else {
        BmanArtist::GratefulDead
    };

    let year = expand_year(&cap[1]);
    let month = &cap[2];
    let day = &cap[3];
    let date = format!("{year}-{month}-{day}");
    let source_type = detect_source_type(name);

    Some(ParsedShow {
        date,
        venue: String::new(),
        city: String::new(),
        state: String::new(),
        source_type,
        source_tag: String::new(),
        is_nll,
        artist,
        folder_id: folder_id.to_string(),
    })
}

/// 2a: Parse dot-delimited date format: `YYYY.MM.DD venue, city, ST`
fn try_parse_dot_date(
    name: &str,
    folder_id: &str,
    artist: BmanArtist,
    is_nll: bool,
) -> Option<ParsedShow> {
    let cap = DOT_DATE_RE.captures(name)?;
    let date = format!("{}-{}-{}", &cap[1], &cap[2], &cap[3]);
    let rest = cap[4].trim().to_string();

    let (location, source_tag) = if let Some(sc) = SOURCE_TAG_RE.captures(&rest) {
        let tag = sc[1].trim().to_string();
        let location = rest[..sc.get(0).unwrap().start()].trim().to_string();
        (location, tag)
    } else {
        (rest, String::new())
    };

    // Detect source type from location text BEFORE cleanup
    let location_source_type = detect_source_type(&location);
    let location = clean_location(&location);

    let (venue, city, state) = parse_venue_city_state(&location);
    let source_type = detect_source_type(&source_tag)
        .max(detect_source_type(name))
        .max(location_source_type);
    let show_is_nll = is_nll || source_tag.to_lowercase().contains("nll");

    Some(ParsedShow {
        date,
        venue,
        city,
        state,
        source_type,
        source_tag,
        is_nll: show_is_nll,
        artist: resolve_artist(name, artist),
        folder_id: folder_id.to_string(),
    })
}

/// Try all core date parsers on `remainder` text (no prefix/volume stripping).
/// Used by artist-prefixed and volume-prefixed parsers to avoid repeating the chain.
fn try_parse_remainder(
    remainder: &str,
    folder_id: &str,
    artist: BmanArtist,
    is_nll: bool,
) -> Option<ParsedShow> {
    try_parse_nice(remainder, folder_id, artist, is_nll)
        .or_else(|| try_parse_dot_date(remainder, folder_id, artist, is_nll))
        .or_else(|| try_parse_mm_dd_yyyy(remainder, folder_id, artist, is_nll))
        .or_else(|| try_parse_bare_date(remainder, folder_id, artist, is_nll))
        .or_else(|| try_parse_etree(remainder, folder_id, is_nll))
}

/// Determine artist from a known artist prefix word (first word of the prefix).
fn artist_from_prefix(first_word: &str) -> BmanArtist {
    let lower = first_word.to_lowercase();
    if lower == "jerry" || lower == "jgb" {
        BmanArtist::JerryGarciaBand
    } else {
        BmanArtist::GratefulDead
    }
}

/// 2f: Parse artist-prefixed folders by stripping known artist name and re-parsing.
fn try_parse_artist_prefixed(
    name: &str,
    folder_id: &str,
    _artist: BmanArtist,
    is_nll: bool,
) -> Option<ParsedShow> {
    let cap = ARTIST_PREFIX_RE.captures(name)?;
    let first_word = cap.get(0).unwrap().as_str()
        .split_whitespace()
        .next()
        .unwrap_or("");
    let remainder = cap[1].to_string();
    let artist_override = artist_from_prefix(first_word);

    try_parse_remainder(&remainder, folder_id, artist_override, is_nll)
        .map(|mut show| {
            show.artist = artist_override;
            show
        })
}

/// Nice date followed by etree-style dot-separated metadata:
/// `1972-10-18.miller`, `1981-05-11.mtx.seamons.ht73.107252.flac16`
fn try_parse_nice_etree_hybrid(
    name: &str,
    folder_id: &str,
    artist: BmanArtist,
    is_nll: bool,
) -> Option<ParsedShow> {
    let cap = NICE_ETREE_HYBRID_RE.captures(name)?;
    let date = cap[1].to_string();
    let source_type = detect_source_type(name);

    Some(ParsedShow {
        date,
        venue: String::new(),
        city: String::new(),
        state: String::new(),
        source_type,
        source_tag: String::new(),
        is_nll,
        artist: resolve_artist(name, artist),
        folder_id: folder_id.to_string(),
    })
}

/// Bare US date without venue: `2-26-77`, `6-25-83`
fn try_parse_bare_us_date(
    name: &str,
    folder_id: &str,
    artist: BmanArtist,
    is_nll: bool,
) -> Option<ParsedShow> {
    let cap = BARE_US_DATE_RE.captures(name)?;
    let month: u32 = cap[1].parse().ok()?;
    let day: u32 = cap[2].parse().ok()?;
    let year = expand_year(&cap[3]);
    let date = format!("{}-{:02}-{:02}", year, month, day);

    Some(ParsedShow {
        date,
        venue: String::new(),
        city: String::new(),
        state: String::new(),
        source_type: SourceType::Unknown,
        source_tag: String::new(),
        is_nll,
        artist: resolve_artist(name, artist),
        folder_id: folder_id.to_string(),
    })
}

/// Artist-prefixed with dash separator: `Grateful Dead - 1978-07-05 Civic Auditorium...`
fn try_parse_artist_dash_prefixed(
    name: &str,
    folder_id: &str,
    _artist: BmanArtist,
    is_nll: bool,
) -> Option<ParsedShow> {
    let cap = ARTIST_DASH_PREFIX_RE.captures(name)?;
    let first_word = cap.get(0).unwrap().as_str()
        .split_whitespace()
        .next()
        .unwrap_or("");
    let remainder = cap[1].to_string();
    let artist_override = artist_from_prefix(first_word);

    try_parse_remainder(&remainder, folder_id, artist_override, is_nll)
        .map(|mut show| {
            show.artist = artist_override;
            show
        })
}

/// Volume/series prefix: `Vol. 03 - 1971-10-26 University of Rochester, NY`
fn try_parse_volume_prefixed(
    name: &str,
    folder_id: &str,
    artist: BmanArtist,
    is_nll: bool,
) -> Option<ParsedShow> {
    let cap = VOLUME_PREFIX_RE.captures(name)?;
    let remainder = cap[1].to_string();
    try_parse_remainder(&remainder, folder_id, artist, is_nll)
}

/// Check if a folder name is a known non-show folder that should be skipped.
pub fn should_skip_folder(name: &str) -> bool {
    SKIP_FOLDER_RE.is_match(name)
}

// ---- Public API -------------------------------------------------------------

/// Parse a Bman show folder name into a `ParsedShow`.
///
/// Patterns tried in priority order:
/// 1. Nice: `YYYY-MM-DD [- ]venue, city, ST (source)`
/// 2. Nice+etree hybrid: `YYYY-MM-DD.etree.metadata`
/// 3. Dot date: `YYYY.MM.DD venue, city, ST`
/// 4. US date: `M-DD-YY` / `MM-DD-YYYY - venue - city, ST`
/// 5. Bare US date: `M-DD-YY` (no venue)
/// 6. Bare date: `YYYY-MM-DD` / `YYYY.MM.DD`
/// 7. Etree: `(?i)(?:gd|jgb|jg)YY[-.]MM[-.]DD`
/// 8. Volume-prefixed: `Vol. 03 - ...` (strips prefix, re-parses)
/// 9. Artist-dash-prefixed: `Grateful Dead - YYYY-MM-DD ...`
/// 10. Artist-prefixed: `Grateful Dead YYYY-MM-DD ...`
///
/// Returns `None` if no pattern matches.
pub fn parse_show_folder(
    name: &str,
    folder_id: &str,
    artist: BmanArtist,
    is_nll: bool,
) -> Option<ParsedShow> {
    // Strip Google Drive zip-extract timestamp suffix before any pattern matching
    let name = DRIVE_ZIP_SUFFIX_RE.replace(name, "");
    let name = name.as_ref();

    try_parse_nice(name, folder_id, artist, is_nll)
        .or_else(|| try_parse_nice_etree_hybrid(name, folder_id, artist, is_nll))
        .or_else(|| try_parse_dot_date(name, folder_id, artist, is_nll))
        .or_else(|| try_parse_mm_dd_yyyy(name, folder_id, artist, is_nll))
        .or_else(|| try_parse_bare_us_date(name, folder_id, artist, is_nll))
        .or_else(|| try_parse_bare_date(name, folder_id, artist, is_nll))
        .or_else(|| try_parse_etree(name, folder_id, is_nll))
        .or_else(|| try_parse_volume_prefixed(name, folder_id, artist, is_nll))
        .or_else(|| try_parse_artist_dash_prefixed(name, folder_id, artist, is_nll))
        .or_else(|| try_parse_artist_prefixed(name, folder_id, artist, is_nll))
}

/// Parse a track filename into a `ParsedTrack`.
///
/// Patterns tried in order:
/// 1. Etree: `[sd](\d+)t(\d+)` anywhere in filename (handles prefixed names like `gd77-04-23d1t01.flac`)
/// 2. Nice: `(\d+) title.flac`
///
/// Returns `None` if no pattern matches.
pub fn parse_track_filename(name: &str, file_id: &str) -> Option<ParsedTrack> {
    // Etree pattern (d1t01 / s1t01) — try before nice so "d1t01.flac" isn't ambiguous.
    // Matches anywhere in filename to handle prefixed names like "gd77-04-23d1t01.flac".
    if let Some(cap) = ETREE_TRACK_RE.captures(name) {
        let disc_num: i64 = cap[1].parse().unwrap_or(1);
        let track_num: i64 = cap[2].parse().unwrap_or(0);
        let title = cap
            .get(3)
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default();
        return Some(ParsedTrack {
            track_num,
            disc_num,
            title,
            file_id: file_id.to_string(),
        });
    }

    // Nice pattern: NN [separator] title.flac
    if let Some(cap) = NICE_TRACK_RE.captures(name) {
        let raw_num: i64 = cap[1].parse().unwrap_or(0);
        let title = cap[2].trim().to_string();
        // Decode 3-digit track numbers: 101 → disc 1, track 1; 209 → disc 2, track 9
        // Guards: first digit 1-4 (realistic disc count), remainder non-zero (track 0 invalid)
        let (disc_num, track_num) = if raw_num >= 100 && raw_num / 100 <= 4 && raw_num % 100 != 0
        {
            (raw_num / 100, raw_num % 100)
        } else {
            (1, raw_num)
        };
        return Some(ParsedTrack {
            track_num,
            disc_num,
            title,
            file_id: file_id.to_string(),
        });
    }

    None
}

/// Detect if a folder name is a disc subfolder. Returns the disc number.
pub fn parse_disc_subfolder(name: &str) -> Option<i64> {
    DISC_SUBFOLDER_RE
        .captures(name)
        .and_then(|cap| cap[1].parse().ok())
}

/// Detect if a folder name looks like a year navigation folder (1965–2025).
/// Returns the year string, or `None` if it looks like a show or etree folder.
pub fn is_year_folder(name: &str) -> Option<String> {
    // Exclude YYYY-MM-DD date patterns
    let bytes = name.as_bytes();
    if bytes.len() >= 10 && bytes[4] == b'-' && bytes[7] == b'-' {
        return None;
    }
    // Exclude YYYY.MM.DD date patterns
    if bytes.len() >= 10 && bytes[4] == b'.' && bytes[7] == b'.' {
        return None;
    }
    // Exclude MM-DD-YYYY patterns
    if bytes.len() >= 10 && bytes[2] == b'-' && bytes[5] == b'-' {
        return None;
    }
    // Exclude M-DD-YY and similar short US date patterns
    if US_DATE_RE.is_match(name) {
        return None;
    }
    // Exclude etree folder names (gd... / jgb...)
    if ETREE_RE.is_match(name) {
        return None;
    }

    YEAR_RANGE_RE.captures(name).map(|cap| cap[1].to_string())
}

/// Detect if a folder is a New Lossless Library folder.
pub fn is_nll_folder(name: &str) -> bool {
    name.contains("New Lossless") || name.contains('⚡')
}

/// Detect if a folder name refers to the Jerry Garcia Band.
pub fn is_jgb_folder(name: &str) -> bool {
    JGB_RE.is_match(name)
}

/// Deduplicate shows by `(artist, date)`:
/// - Groups all shows for the same artist+date together
/// - NLL overrides all non-NLL
/// - Then highest `source_type` wins
/// - Among equals, prefer entries with venue data
/// - Copy venue/city/state from a venue-having entry to the winner if winner lacks it
pub fn dedup_shows(shows: Vec<ParsedShow>) -> Vec<ParsedShow> {
    let mut groups: HashMap<(String, String), Vec<ParsedShow>> = HashMap::new();
    for show in shows {
        let key = (show.artist.name().to_string(), show.date.clone());
        groups.entry(key).or_default().push(show);
    }

    let mut result = Vec::new();
    for (_key, mut group) in groups {
        // Capture best venue data BEFORE any filtering
        let best_venue = group
            .iter()
            .filter(|s| !s.venue.is_empty())
            .max_by_key(|s| s.city.len() + s.state.len())
            .cloned();

        // Drop AUD recordings first — before NLL/source-type selection.
        // This prevents an NLL AUD from shadowing a non-NLL SBD.
        group.retain(|s| s.source_type != SourceType::Aud);
        if group.is_empty() {
            continue;
        }

        if group.len() == 1 {
            let mut winner = group.into_iter().next().unwrap();
            if winner.venue.is_empty() {
                if let Some(bv) = best_venue {
                    winner.venue = bv.venue;
                    winner.city = bv.city;
                    winner.state = bv.state;
                }
            }
            result.push(winner);
            continue;
        }

        // NLL overrides all non-NLL
        let has_nll = group.iter().any(|s| s.is_nll);
        if has_nll {
            group.retain(|s| s.is_nll);
        }

        // Keep highest source_type
        let max_type = group.iter().map(|s| s.source_type).max().unwrap();
        group.retain(|s| s.source_type == max_type);

        // Pick the winner: prefer one with venue data among remaining
        let mut winner = {
            let venue_idx = group.iter().position(|s| !s.venue.is_empty());
            if let Some(idx) = venue_idx {
                group.swap_remove(idx)
            } else {
                group.into_iter().next().unwrap()
            }
        };

        // Copy venue data from best source if winner lacks it
        if winner.venue.is_empty() {
            if let Some(bv) = best_venue {
                winner.venue = bv.venue;
                winner.city = bv.city;
                winner.state = bv.state;
            }
        }

        result.push(winner);
    }

    result.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then(a.artist.name().cmp(b.artist.name()))
    });

    result
}

// ---- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_show_folder: Nice format ---

    #[test]
    fn test_nice_full_betty_board() {
        let show = parse_show_folder(
            "1977-05-08 Cornell University, Ithaca, NY (Betty Board)",
            "folder123",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.date, "1977-05-08");
        assert_eq!(show.venue, "Cornell University");
        assert_eq!(show.city, "Ithaca");
        assert_eq!(show.state, "NY");
        assert_eq!(show.source_tag, "Betty Board");
        assert_eq!(show.source_type, SourceType::Sbd);
        assert!(!show.is_nll);
        assert_eq!(show.folder_id, "folder123");
        assert_eq!(show.artist, BmanArtist::GratefulDead);
    }

    #[test]
    fn test_nice_sbd_tag() {
        let show = parse_show_folder(
            "1977-05-08 Cornell University, Ithaca, NY (SBD)",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.source_type, SourceType::Sbd);
        assert_eq!(show.source_tag, "SBD");
    }

    #[test]
    fn test_nice_aud_tag() {
        let show = parse_show_folder(
            "1977-05-08 Cornell University, Ithaca, NY (Aud)",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.source_type, SourceType::Aud);
    }

    #[test]
    fn test_nice_mtx_tag() {
        let show = parse_show_folder(
            "1977-05-08 Madison Square Garden, New York, NY (Matrix)",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.source_type, SourceType::Mtx);
    }

    #[test]
    fn test_nice_no_source_tag() {
        let show = parse_show_folder(
            "1977-05-08 Cornell University, Ithaca, NY",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.venue, "Cornell University");
        assert_eq!(show.city, "Ithaca");
        assert_eq!(show.state, "NY");
        assert!(show.source_tag.is_empty());
        assert_eq!(show.source_type, SourceType::Unknown);
    }

    #[test]
    fn test_nice_multi_part_venue() {
        // venue contains a comma -> last two comma-parts are city, state
        let show = parse_show_folder(
            "1977-05-08 Barton Hall, Cornell University, Ithaca, NY (SBD)",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.venue, "Barton Hall, Cornell University");
        assert_eq!(show.city, "Ithaca");
        assert_eq!(show.state, "NY");
    }

    #[test]
    fn test_nice_source_tag_only() {
        // No venue/city/state, just a source tag
        let show = parse_show_folder(
            "1977-05-08 (SBD)",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.date, "1977-05-08");
        assert_eq!(show.source_tag, "SBD");
        assert!(show.venue.is_empty());
    }

    #[test]
    fn test_nice_is_nll_from_param() {
        let show = parse_show_folder(
            "1977-05-08 Cornell University, Ithaca, NY (SBD)",
            "fid",
            BmanArtist::GratefulDead,
            true,
        )
        .unwrap();
        assert!(show.is_nll);
    }

    #[test]
    fn test_nice_is_nll_from_source_tag() {
        let show = parse_show_folder(
            "1977-05-08 Cornell University, Ithaca, NY (NLL SBD)",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert!(show.is_nll);
    }

    // --- parse_show_folder: MM-DD-YYYY format ---

    #[test]
    fn test_mm_dd_yyyy_basic() {
        let show = parse_show_folder(
            "05-08-1977 - Cornell University - Ithaca, NY",
            "folder456",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.date, "1977-05-08");
        assert_eq!(show.venue, "Cornell University");
        assert_eq!(show.city, "Ithaca");
        assert_eq!(show.state, "NY");
        assert_eq!(show.folder_id, "folder456");
    }

    #[test]
    fn test_mm_dd_yyyy_no_city_state() {
        let show = parse_show_folder(
            "07-18-1973 - Keystone Berkeley",
            "fid",
            BmanArtist::JerryGarciaBand,
            false,
        )
        .unwrap();
        assert_eq!(show.date, "1973-07-18");
        assert_eq!(show.venue, "Keystone Berkeley");
        assert!(show.city.is_empty());
    }

    // --- parse_show_folder: Bare date ---

    #[test]
    fn test_bare_date() {
        let show =
            parse_show_folder("1977-05-08", "fid", BmanArtist::GratefulDead, false).unwrap();
        assert_eq!(show.date, "1977-05-08");
        assert!(show.venue.is_empty());
        assert!(show.city.is_empty());
        assert!(show.state.is_empty());
        assert_eq!(show.source_type, SourceType::Unknown);
    }

    #[test]
    fn test_bare_date_with_trailing_space() {
        let show =
            parse_show_folder("1977-05-08 ", "fid", BmanArtist::GratefulDead, false).unwrap();
        assert_eq!(show.date, "1977-05-08");
    }

    // --- parse_show_folder: Etree format ---

    #[test]
    fn test_etree_gd_dash_separator() {
        let show = parse_show_folder(
            "gd77-05-08.sbd.miller.flac16",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.date, "1977-05-08");
        assert_eq!(show.artist, BmanArtist::GratefulDead);
        assert_eq!(show.source_type, SourceType::Sbd);
    }

    #[test]
    fn test_etree_jgb_two_digit_year() {
        let show = parse_show_folder(
            "jgb73-07-18.aud.flac16",
            "fid",
            BmanArtist::GratefulDead, // caller says GD, but etree prefix overrides
            false,
        )
        .unwrap();
        assert_eq!(show.date, "1973-07-18");
        assert_eq!(show.artist, BmanArtist::JerryGarciaBand);
        assert_eq!(show.source_type, SourceType::Aud);
    }

    #[test]
    fn test_etree_dot_separator() {
        let show = parse_show_folder(
            "gd77.05.08.sbd.miller.flac16",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.date, "1977-05-08");
    }

    #[test]
    fn test_etree_four_digit_year() {
        let show = parse_show_folder(
            "gd1977-05-08.sbd.flac16",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.date, "1977-05-08");
    }

    #[test]
    fn test_etree_low_two_digit_year_maps_to_2000s() {
        let show =
            parse_show_folder("gd03-05-08.sbd.flac", "fid", BmanArtist::GratefulDead, false)
                .unwrap();
        assert_eq!(show.date, "2003-05-08");
    }

    #[test]
    fn test_etree_mtx_source() {
        let show = parse_show_folder(
            "gd77-05-08.mtx.something.flac16",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.source_type, SourceType::Mtx);
    }

    #[test]
    fn test_unrecognized_folder_returns_none() {
        assert!(parse_show_folder(
            "Some Random Folder Name",
            "fid",
            BmanArtist::GratefulDead,
            false
        )
        .is_none());
        assert!(parse_show_folder("README", "fid", BmanArtist::GratefulDead, false).is_none());
    }

    // --- parse_track_filename ---

    #[test]
    fn test_nice_track_space_separator() {
        let track = parse_track_filename("02 Scarlet Begonias.flac", "fid").unwrap();
        assert_eq!(track.track_num, 2);
        assert_eq!(track.disc_num, 1);
        assert_eq!(track.title, "Scarlet Begonias");
        assert_eq!(track.file_id, "fid");
    }

    #[test]
    fn test_nice_track_dot_separator() {
        let track = parse_track_filename("02. Scarlet Begonias.flac", "fid").unwrap();
        assert_eq!(track.track_num, 2);
        assert_eq!(track.title, "Scarlet Begonias");
    }

    #[test]
    fn test_nice_track_dash_separator() {
        let track = parse_track_filename("02 - Scarlet Begonias.flac", "fid").unwrap();
        assert_eq!(track.track_num, 2);
        assert_eq!(track.title, "Scarlet Begonias");
    }

    #[test]
    fn test_nice_track_case_insensitive_extension() {
        let track = parse_track_filename("01 Fire On The Mountain.FLAC", "fid").unwrap();
        assert_eq!(track.track_num, 1);
        assert_eq!(track.title, "Fire On The Mountain");
    }

    #[test]
    fn test_etree_track_disc1() {
        let track = parse_track_filename("d1t01.flac", "fid").unwrap();
        assert_eq!(track.disc_num, 1);
        assert_eq!(track.track_num, 1);
        assert!(track.title.is_empty());
    }

    #[test]
    fn test_etree_track_disc2() {
        let track = parse_track_filename("d2t05.flac", "fid").unwrap();
        assert_eq!(track.disc_num, 2);
        assert_eq!(track.track_num, 5);
    }

    #[test]
    fn test_etree_track_multi_digit() {
        let track = parse_track_filename("d1t12.flac", "fid").unwrap();
        assert_eq!(track.disc_num, 1);
        assert_eq!(track.track_num, 12);
    }

    #[test]
    fn test_unrecognized_track_returns_none() {
        assert!(parse_track_filename("readme.txt", "fid").is_none());
        assert!(parse_track_filename("noext", "fid").is_none());
        assert!(parse_track_filename("cover.jpg", "fid").is_none());
    }

    // --- prefixed etree track filenames ---

    #[test]
    fn test_prefixed_etree_gd_disc_track() {
        let track = parse_track_filename("gd77-04-23d1t01.flac", "fid").unwrap();
        assert_eq!(track.disc_num, 1);
        assert_eq!(track.track_num, 1);
        assert!(track.title.is_empty());
    }

    #[test]
    fn test_prefixed_etree_set_track() {
        let track = parse_track_filename("gd77-05-08s1t02.flac", "fid").unwrap();
        assert_eq!(track.disc_num, 1);
        assert_eq!(track.track_num, 2);
        assert!(track.title.is_empty());
    }

    #[test]
    fn test_prefixed_etree_with_title() {
        let track = parse_track_filename("gd85-03-10 s1t02 Stranger.flac", "fid").unwrap();
        assert_eq!(track.disc_num, 1);
        assert_eq!(track.track_num, 2);
        assert_eq!(track.title, "Stranger");
    }

    #[test]
    fn test_prefixed_jgb_with_title() {
        let track = parse_track_filename("JGB 1980-03-08d1t01 Sugaree.flac", "fid").unwrap();
        assert_eq!(track.disc_num, 1);
        assert_eq!(track.track_num, 1);
        assert_eq!(track.title, "Sugaree");
    }

    #[test]
    fn test_prefixed_jgb_no_space() {
        let track = parse_track_filename("jgb1976-01-10s1t01.flac", "fid").unwrap();
        assert_eq!(track.disc_num, 1);
        assert_eq!(track.track_num, 1);
    }

    #[test]
    fn test_etree_track_still_works_bare() {
        // Bare d1t01.flac must still work with the new regex
        let track = parse_track_filename("d1t01.flac", "fid").unwrap();
        assert_eq!(track.disc_num, 1);
        assert_eq!(track.track_num, 1);
    }

    // --- 3-digit track number decode ---

    #[test]
    fn test_three_digit_decode_101() {
        let track = parse_track_filename("101. Tuning.flac", "fid").unwrap();
        assert_eq!(track.disc_num, 1);
        assert_eq!(track.track_num, 1);
        assert_eq!(track.title, "Tuning");
    }

    #[test]
    fn test_three_digit_decode_209() {
        let track = parse_track_filename("209. Saturday.flac", "fid").unwrap();
        assert_eq!(track.disc_num, 2);
        assert_eq!(track.track_num, 9);
        assert_eq!(track.title, "Saturday");
    }

    #[test]
    fn test_three_digit_no_decode_99() {
        let track = parse_track_filename("99 Encore.flac", "fid").unwrap();
        assert_eq!(track.disc_num, 1);
        assert_eq!(track.track_num, 99);
    }

    #[test]
    fn test_three_digit_no_decode_100() {
        // 100 % 100 == 0, so track 0 is invalid — don't decode
        let track = parse_track_filename("100. Intermission.flac", "fid").unwrap();
        assert_eq!(track.disc_num, 1);
        assert_eq!(track.track_num, 100);
    }

    #[test]
    fn test_three_digit_no_decode_501() {
        // Disc 5 is unrealistic — don't decode
        let track = parse_track_filename("501. Weird.flac", "fid").unwrap();
        assert_eq!(track.disc_num, 1);
        assert_eq!(track.track_num, 501);
    }

    // --- parse_disc_subfolder ---

    #[test]
    fn test_disc_subfolders_all_patterns() {
        assert_eq!(parse_disc_subfolder("disc1"), Some(1));
        assert_eq!(parse_disc_subfolder("Disc 1"), Some(1));
        assert_eq!(parse_disc_subfolder("d1"), Some(1));
        assert_eq!(parse_disc_subfolder("CD1"), Some(1));
        assert_eq!(parse_disc_subfolder("Set 1"), Some(1));
        assert_eq!(parse_disc_subfolder("disk2"), Some(2));
        assert_eq!(parse_disc_subfolder("DISC3"), Some(3));
        assert_eq!(parse_disc_subfolder("cd 2"), Some(2));
        assert_eq!(parse_disc_subfolder("set2"), Some(2));
    }

    #[test]
    fn test_non_disc_folders() {
        assert!(parse_disc_subfolder("1977").is_none());
        assert!(parse_disc_subfolder("Cornell University").is_none());
        assert!(parse_disc_subfolder("gd77-05-08").is_none());
        assert!(parse_disc_subfolder("d1t01.flac").is_none()); // has extra chars
    }

    // --- is_year_folder ---

    #[test]
    fn test_is_year_folder_bare() {
        assert_eq!(is_year_folder("1977"), Some("1977".to_string()));
        assert_eq!(is_year_folder("1965"), Some("1965".to_string()));
        assert_eq!(is_year_folder("2025"), Some("2025".to_string()));
    }

    #[test]
    fn test_is_year_folder_with_suffix() {
        assert_eq!(is_year_folder("1977 (NLL)"), Some("1977".to_string()));
        assert_eq!(is_year_folder("NLL 1977"), Some("1977".to_string()));
        assert_eq!(is_year_folder("Fall 1972"), Some("1972".to_string()));
    }

    #[test]
    fn test_is_year_folder_excludes_show_folders() {
        assert!(is_year_folder("1977-05-08 Cornell University, Ithaca, NY").is_none());
        assert!(is_year_folder("1977-05-08").is_none());
        assert!(is_year_folder("1977.05.08").is_none());
        assert!(is_year_folder("1977.05.08 Cornell University, Ithaca, NY").is_none());
        assert!(is_year_folder("gd77-05-08.sbd.flac16").is_none());
        assert!(is_year_folder("05-08-1977 - Cornell University - Ithaca, NY").is_none());
        assert!(is_year_folder("2-26-73 Pershing Municipal Auditorium").is_none());
    }

    #[test]
    fn test_is_year_folder_out_of_range() {
        assert!(is_year_folder("1900").is_none());
        assert!(is_year_folder("2030").is_none());
        assert!(is_year_folder("1964").is_none());
    }

    // --- is_nll_folder ---

    #[test]
    fn test_is_nll_folder() {
        assert!(is_nll_folder("New Lossless Library"));
        assert!(is_nll_folder("1977 New Lossless"));
        assert!(is_nll_folder("⚡ Lossless Archive"));
        assert!(is_nll_folder("Shows ⚡"));
        assert!(!is_nll_folder("Regular Folder"));
        assert!(!is_nll_folder("1977"));
        assert!(!is_nll_folder("Soundboard Recording"));
    }

    // --- is_jgb_folder ---

    #[test]
    fn test_is_jgb_folder() {
        assert!(is_jgb_folder("Jerry Garcia Band"));
        assert!(is_jgb_folder("JGB"));
        assert!(is_jgb_folder("Jerry Garcia Solo"));
        assert!(is_jgb_folder("jerry garcia band 1976"));
        assert!(!is_jgb_folder("Grateful Dead"));
        assert!(!is_jgb_folder("1977"));
    }

    // --- detect_source_type ---

    #[test]
    fn test_detect_source_type_variants() {
        assert_eq!(detect_source_type("sbd.miller"), SourceType::Sbd);
        assert_eq!(detect_source_type("SBEOK"), SourceType::Sbd);
        assert_eq!(detect_source_type("Betty Board"), SourceType::Sbd);
        assert_eq!(detect_source_type("aud.jones"), SourceType::Aud);
        assert_eq!(detect_source_type("audience recording"), SourceType::Aud);
        assert_eq!(detect_source_type("mtx.smith"), SourceType::Mtx);
        assert_eq!(detect_source_type("matrix mix"), SourceType::Mtx);
        assert_eq!(detect_source_type("flac16"), SourceType::Unknown);
        assert_eq!(detect_source_type("fob.ecm.99a.hopkins"), SourceType::Aud);
        assert_eq!(detect_source_type("fob.akg.d330bt.senn421.hecht"), SourceType::Aud);
    }

    #[test]
    fn test_detect_source_type_real_etree_names() {
        // Real folder names from Google Drive archive
        assert_eq!(
            detect_source_type("gd90-07-04.081523.sbd.miller.tetzeli.fix-26351.sbeok.t-flac16"),
            SourceType::Sbd
        );
        assert_eq!(
            detect_source_type("gd77-05-08.aud.berger.flac"),
            SourceType::Aud
        );
        assert_eq!(
            detect_source_type("gd73-06-10.sbd.shnf.flac16"),
            SourceType::Sbd
        );
        assert_eq!(
            detect_source_type("gd69-02-28.mtx.shnf.flac"),
            SourceType::Mtx
        );
        // No source keyword → Unknown
        assert_eq!(
            detect_source_type("gd77-05-08.berger.flac16"),
            SourceType::Unknown
        );
    }

    #[test]
    fn test_parse_etree_aud_then_dedup_drops_it() {
        // Parse a real AUD etree folder, then dedup with an SBD version → AUD dropped
        let aud = parse_show_folder(
            "gd77-05-08.aud.berger.flac",
            "fid1",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(aud.source_type, SourceType::Aud);

        let sbd = parse_show_folder(
            "gd77-05-08.sbd.miller.flac16",
            "fid2",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(sbd.source_type, SourceType::Sbd);

        let result = dedup_shows(vec![aud, sbd]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source_type, SourceType::Sbd);
    }

    #[test]
    fn test_parse_etree_aud_only_dedup_drops_it() {
        // Only AUD etree folder → dedup drops it entirely
        let aud = parse_show_folder(
            "gd77-05-08.aud.berger.flac",
            "fid1",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(aud.source_type, SourceType::Aud);

        let result = dedup_shows(vec![aud]);
        assert!(result.is_empty(), "AUD-only show must be filtered out");
    }

    // --- dedup_shows ---

    fn make_show(
        date: &str,
        venue: &str,
        source_type: SourceType,
        is_nll: bool,
        artist: BmanArtist,
    ) -> ParsedShow {
        ParsedShow {
            date: date.to_string(),
            venue: venue.to_string(),
            city: String::new(),
            state: String::new(),
            source_type,
            source_tag: String::new(),
            is_nll,
            artist,
            folder_id: "fid".to_string(),
        }
    }

    fn gd_show(date: &str, venue: &str, source_type: SourceType, is_nll: bool) -> ParsedShow {
        make_show(date, venue, source_type, is_nll, BmanArtist::GratefulDead)
    }

    #[test]
    fn test_dedup_nll_overrides_sbd() {
        let shows = vec![
            gd_show("1977-05-08", "Cornell", SourceType::Sbd, false),
            gd_show("1977-05-08", "Cornell", SourceType::Unknown, true),
        ];
        let result = dedup_shows(shows);
        assert_eq!(result.len(), 1);
        assert!(result[0].is_nll);
    }

    #[test]
    fn test_dedup_nll_beats_everything() {
        let shows = vec![
            gd_show("1977-05-08", "Cornell", SourceType::Sbd, false),
            gd_show("1977-05-08", "Cornell", SourceType::Aud, false),
            gd_show("1977-05-08", "Cornell", SourceType::Unknown, true),
        ];
        let result = dedup_shows(shows);
        assert_eq!(result.len(), 1);
        assert!(result[0].is_nll);
    }

    #[test]
    fn test_dedup_keeps_highest_source_type() {
        let shows = vec![
            gd_show("1977-05-08", "Cornell", SourceType::Aud, false),
            gd_show("1977-05-08", "Cornell", SourceType::Sbd, false),
            gd_show("1977-05-08", "Cornell", SourceType::Mtx, false),
        ];
        let result = dedup_shows(shows);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source_type, SourceType::Sbd);
    }

    #[test]
    fn test_dedup_merges_same_date_same_source() {
        // Two SBD copies of the same date → single winner
        let shows = vec![
            gd_show("1977-05-08", "Cornell", SourceType::Sbd, false),
            gd_show("1977-05-08", "Cornell", SourceType::Sbd, false),
        ];
        let result = dedup_shows(shows);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_dedup_different_dates_kept() {
        let shows = vec![
            gd_show("1977-05-08", "Cornell", SourceType::Sbd, false),
            gd_show("1977-09-03", "Englishtown", SourceType::Sbd, false),
        ];
        let result = dedup_shows(shows);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_dedup_venue_normalization() {
        // Different punctuation -> same normalized venue -> deduplicated
        let shows = vec![
            gd_show("1977-05-08", "Cornell University", SourceType::Sbd, false),
            gd_show("1977-05-08", "cornell university!", SourceType::Aud, false),
        ];
        let result = dedup_shows(shows);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source_type, SourceType::Sbd);
    }

    #[test]
    fn test_dedup_different_artists_kept() {
        let shows = vec![
            make_show(
                "1977-05-08",
                "Venue",
                SourceType::Sbd,
                false,
                BmanArtist::GratefulDead,
            ),
            make_show(
                "1977-05-08",
                "Venue",
                SourceType::Sbd,
                false,
                BmanArtist::JerryGarciaBand,
            ),
        ];
        let result = dedup_shows(shows);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_dedup_sorted_by_date() {
        let shows = vec![
            gd_show("1984-06-24", "Greek", SourceType::Sbd, false),
            gd_show("1977-05-08", "Cornell", SourceType::Sbd, false),
            gd_show("1972-04-14", "Tivoli", SourceType::Sbd, false),
        ];
        let result = dedup_shows(shows);
        assert_eq!(result[0].date, "1972-04-14");
        assert_eq!(result[1].date, "1977-05-08");
        assert_eq!(result[2].date, "1984-06-24");
    }

    #[test]
    fn test_dedup_single_show_unchanged() {
        let shows = vec![gd_show("1977-05-08", "Cornell", SourceType::Sbd, false)];
        let result = dedup_shows(shows);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_dedup_empty() {
        let result = dedup_shows(vec![]);
        assert!(result.is_empty());
    }

    // --- expand_year ---

    #[test]
    fn test_expand_year_two_digit_ge_50() {
        assert_eq!(expand_year("65"), "1965");
        assert_eq!(expand_year("77"), "1977");
        assert_eq!(expand_year("99"), "1999");
    }

    #[test]
    fn test_expand_year_two_digit_lt_50() {
        assert_eq!(expand_year("00"), "2000");
        assert_eq!(expand_year("03"), "2003");
        assert_eq!(expand_year("24"), "2024");
    }

    #[test]
    fn test_expand_year_four_digit() {
        assert_eq!(expand_year("1977"), "1977");
        assert_eq!(expand_year("2003"), "2003");
    }

    // --- 1e: JGB in GD folder ---

    #[test]
    fn test_jgb_in_gd_folder_nice() {
        let show = parse_show_folder(
            "1983-06-03 Capitol Theater, Passaic, NJ (Jerry Garcia Band)",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.artist, BmanArtist::JerryGarciaBand);
        assert_eq!(show.date, "1983-06-03");
    }

    #[test]
    fn test_jgb_in_mm_dd_yyyy_format() {
        let show = parse_show_folder(
            "06-10-1973 - JGB Keystone Berkeley",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.artist, BmanArtist::JerryGarciaBand);
        assert_eq!(show.date, "1973-06-10");
    }

    // --- 1f: AUD filtered from dedup ---

    #[test]
    fn test_aud_filtered_from_dedup() {
        let shows = vec![
            gd_show("1977-05-08", "Cornell", SourceType::Aud, false),
            gd_show("1977-05-08", "Cornell", SourceType::Sbd, false),
        ];
        let result = dedup_shows(shows);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source_type, SourceType::Sbd);
    }

    #[test]
    fn test_aud_only_shows_removed() {
        let shows = vec![
            gd_show("1977-05-08", "Cornell", SourceType::Aud, false),
            gd_show("1973-07-18", "Keystone", SourceType::Aud, false),
        ];
        let result = dedup_shows(shows);
        assert!(result.is_empty());
    }

    // --- 1g: Venue cleanup ---

    #[test]
    fn test_venue_cleanup_strip_source_indicators() {
        let show = parse_show_folder(
            "1983-06-03 Capitol Theater Passaic NJ - AUD FLAC",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert!(!show.venue.contains("AUD"));
        assert!(!show.venue.contains("FLAC"));
        assert_eq!(show.source_type, SourceType::Aud);
    }

    // --- 2a: Dot date ---

    #[test]
    fn test_dot_date_format() {
        let show = parse_show_folder(
            "1977.05.08 Cornell University, Ithaca, NY",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.date, "1977-05-08");
        assert_eq!(show.venue, "Cornell University");
        assert_eq!(show.city, "Ithaca");
        assert_eq!(show.state, "NY");
    }

    // --- 2b: Short year US date ---

    #[test]
    fn test_short_year_us_date() {
        let show = parse_show_folder(
            "2-26-73 Pershing Municipal Auditorium",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.date, "1973-02-26");
        assert_eq!(show.venue, "Pershing Municipal Auditorium");
    }

    #[test]
    fn test_single_digit_us_date() {
        let show = parse_show_folder(
            "6-10-73 RFK Stadium",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.date, "1973-06-10");
        assert_eq!(show.venue, "RFK Stadium");
    }

    // --- 2c: Etree jg prefix ---

    #[test]
    fn test_etree_jg_prefix() {
        let show = parse_show_folder(
            "jg1975-03-07.sbd.flac",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.artist, BmanArtist::JerryGarciaBand);
        assert_eq!(show.date, "1975-03-07");
    }

    #[test]
    fn test_etree_uppercase_gd() {
        let show = parse_show_folder(
            "GD70-01-02.sbd.flac",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.artist, BmanArtist::GratefulDead);
        assert_eq!(show.date, "1970-01-02");
    }

    // --- 2d: Nice with dash separator ---

    #[test]
    fn test_nice_with_dash_separator() {
        let show = parse_show_folder(
            "1971-02-18 - Capitol Theater, Port Chester, NY",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.date, "1971-02-18");
        assert_eq!(show.venue, "Capitol Theater");
        assert_eq!(show.city, "Port Chester");
        assert_eq!(show.state, "NY");
    }

    // --- 2e: Bare dot date ---

    #[test]
    fn test_bare_dot_date() {
        let show = parse_show_folder(
            "1967.01.14",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.date, "1967-01-14");
        assert!(show.venue.is_empty());
    }

    // --- 2f: Artist-prefixed folders ---

    #[test]
    fn test_artist_prefixed_gd() {
        let show = parse_show_folder(
            "Grateful Dead 1977-05-08 Cornell University, Ithaca, NY",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.artist, BmanArtist::GratefulDead);
        assert_eq!(show.date, "1977-05-08");
        assert_eq!(show.venue, "Cornell University");
    }

    #[test]
    fn test_artist_prefixed_jgb() {
        let show = parse_show_folder(
            "Jerry Garcia Band 1975-03-07 Great American Music Hall",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.artist, BmanArtist::JerryGarciaBand);
        assert_eq!(show.date, "1975-03-07");
        assert_eq!(show.venue, "Great American Music Hall");
    }

    // --- 2g: Skip folders ---

    #[test]
    fn test_should_skip_folder() {
        assert!(should_skip_folder("VIDEO_TS"));
        assert!(should_skip_folder("Artwork"));
        assert!(should_skip_folder("extras"));
        assert!(should_skip_folder("bonus"));
        assert!(should_skip_folder("info"));
        assert!(should_skip_folder("text"));
        assert!(should_skip_folder("liner notes"));
        assert!(should_skip_folder("covers"));
        assert!(should_skip_folder("cover"));
        assert!(should_skip_folder("Set1"));
        assert!(should_skip_folder("Set 2"));
        assert!(should_skip_folder("disc1"));
        assert!(should_skip_folder("Disc 2"));
        assert!(should_skip_folder("CD1"));
        assert!(should_skip_folder("d1"));
        assert!(!should_skip_folder("1977-05-08"));
        assert!(!should_skip_folder("gd77-05-08.sbd.flac16"));
        assert!(!should_skip_folder("Some Random Folder"));
        // Bman's Picks parent folders
        assert!(should_skip_folder("Bman's Picks Vol 36a: 9/28/75 Lindley Meadows"));
        assert!(should_skip_folder("Bmans Picks Vol 12"));
    }

    // --- clean_location ---

    #[test]
    fn test_clean_location_strips_source_words() {
        assert_eq!(clean_location("Fillmore West SBD FLAC"), "Fillmore West");
    }

    #[test]
    fn test_clean_location_strips_etree_paren() {
        assert_eq!(
            clean_location("(sbd.droncit.127478) San Antonio Civic Auditorium"),
            "San Antonio Civic Auditorium"
        );
    }

    #[test]
    fn test_clean_location_strips_incredible() {
        assert_eq!(clean_location("Yale Bowl INCREDIBLE SBD"), "Yale Bowl");
    }

    #[test]
    fn test_clean_location_strips_c_miller() {
        assert_eq!(
            clean_location("Capitol Theater, Passaic, NJ - C. Miller SBD"),
            "Capitol Theater, Passaic, NJ"
        );
    }

    #[test]
    fn test_clean_location_passthrough_clean_venue() {
        assert_eq!(
            clean_location("Cornell University, Ithaca, NY"),
            "Cornell University, Ithaca, NY"
        );
    }

    #[test]
    fn test_clean_location_dusborn_matrix_stripped() {
        assert_eq!(
            clean_location("Moody Coliseum, Dusborn Matrix, Dan Wainwright Remaster Set 1"),
            "Moody Coliseum"
        );
    }

    #[test]
    fn test_clean_location_the_matrix_venue_preserved() {
        // "The Matrix" is a real venue — bare "Matrix" should NOT be stripped
        assert_eq!(
            clean_location("The Matrix, San Francisco, CA"),
            "The Matrix, San Francisco, CA"
        );
    }

    #[test]
    fn test_clean_location_remaster_stripped() {
        assert_eq!(
            clean_location("Winterland Remastered SBD"),
            "Winterland"
        );
    }

    #[test]
    fn test_clean_location_all_junk_returns_empty() {
        assert_eq!(clean_location("SBD FLAC"), "");
    }

    // --- Square bracket source tags ---

    #[test]
    fn test_bracket_source_tag() {
        let show = parse_show_folder(
            "1989-07-17 Alpine Valley, East Troy, WI [SBD FLAC]",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.date, "1989-07-17");
        assert_eq!(show.source_tag, "SBD FLAC");
        assert_eq!(show.state, "WI");
    }

    // --- Drive zip suffix stripping ---

    #[test]
    fn test_drive_zip_suffix_nice_folder() {
        let show = parse_show_folder(
            "1977-06-09 Winterland SBD-20250509T025938Z-001",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.date, "1977-06-09");
        assert!(show.venue.contains("Winterland"));
    }

    #[test]
    fn test_drive_zip_suffix_etree_folder() {
        let show = parse_show_folder(
            "gd77-05-21.00271.sbd.boyles.sbeok.flac16-20250509T025957Z-001",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.date, "1977-05-21");
    }

    #[test]
    fn test_drive_zip_suffix_no_suffix_unchanged() {
        // Folders without the suffix should parse normally
        let show = parse_show_folder(
            "1977-05-08 Barton Hall, Ithaca, NY (SBD)",
            "fid",
            BmanArtist::GratefulDead,
            false,
        )
        .unwrap();
        assert_eq!(show.date, "1977-05-08");
        assert_eq!(show.state, "NY");
    }

    // --- nice-etree hybrid parser ---

    #[test]
    fn test_parse_nice_etree_hybrid() {
        let show = parse_show_folder(
            "1972-10-18.miller",
            "fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(show.date, "1972-10-18");
        assert_eq!(show.venue, "");
    }

    #[test]
    fn test_parse_nice_etree_hybrid_long() {
        let show = parse_show_folder(
            "1977-05-08.sbd.hicks.jeffm.flac16",
            "fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(show.date, "1977-05-08");
        assert_eq!(show.venue, "");
    }

    // --- bare US date parser ---

    #[test]
    fn test_parse_bare_us_date() {
        let show = parse_show_folder(
            "2-26-73",
            "fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(show.date, "1973-02-26");
        assert_eq!(show.venue, "");
    }

    #[test]
    fn test_parse_bare_us_date_four_digit_year() {
        let show = parse_show_folder(
            "6-10-1973",
            "fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(show.date, "1973-06-10");
    }

    // --- artist-dash-prefixed parser ---

    #[test]
    fn test_parse_artist_dash_prefix() {
        let show = parse_show_folder(
            "Grateful Dead - 1978-07-05 Red Rocks Amphitheatre",
            "fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(show.date, "1978-07-05");
        assert_eq!(show.venue, "Red Rocks Amphitheatre");
    }

    // --- volume-prefixed parser ---

    #[test]
    fn test_parse_volume_prefix() {
        let show = parse_show_folder(
            "Vol. 03 - 1977-05-08 Cornell University",
            "fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(show.date, "1977-05-08");
        assert_eq!(show.venue, "Cornell University");
    }

    // --- dedup: venue merging ---

    #[test]
    fn test_dedup_merges_venue_from_non_empty() {
        // Etree folder (no venue) + nice folder (with venue) → winner has venue
        let shows = vec![
            gd_show("1977-05-08", "", SourceType::Sbd, false),
            gd_show("1977-05-08", "Cornell University", SourceType::Aud, false),
        ];
        let result = dedup_shows(shows);
        assert_eq!(result.len(), 1);
        // SBD wins priority but gets venue from the AUD entry
        assert_eq!(result[0].source_type, SourceType::Sbd);
        assert_eq!(result[0].venue, "Cornell University");
    }

    // --- real-world folder names from log ---

    #[test]
    fn test_real_folder_dot_date_with_venue() {
        let show = parse_show_folder(
            "1977.05.08 Cornell University, Ithaca, NY",
            "fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(show.date, "1977-05-08");
        assert!(show.venue.contains("Cornell University"));
    }

    #[test]
    fn test_real_folder_nice_with_dash_separator() {
        let show = parse_show_folder(
            "1971-02-18 - Capitol Theater, Port Chester, NY",
            "fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(show.date, "1971-02-18");
        assert!(show.venue.contains("Capitol Theater"));
    }

    #[test]
    fn test_real_folder_us_date_with_venue() {
        let show = parse_show_folder(
            "6-10-73 RFK Stadium, Washington D.C.",
            "fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(show.date, "1973-06-10");
        assert!(show.venue.contains("RFK Stadium"));
    }

    // --- NLL priority with real folder names ---

    #[test]
    fn test_nll_folder_with_gdrive_timestamp_suffix_parses() {
        // Real NLL folder name from Google Drive (has download timestamp appended)
        let show = parse_show_folder(
            "1977-05-08 Cornell University Ithica NY - FLAC Dusborne Matrix-20250509T025840Z-001",
            "fid",
            BmanArtist::GratefulDead,
            true, // is_nll
        ).unwrap();
        assert_eq!(show.date, "1977-05-08");
        assert!(show.is_nll);
    }

    #[test]
    fn test_nll_etree_with_gdrive_timestamp_suffix_parses() {
        let show = parse_show_folder(
            "gd77-05-21.00271.sbd.boyles.sbeok.flac16-20250509T025957Z-001",
            "fid",
            BmanArtist::GratefulDead,
            true,
        ).unwrap();
        assert_eq!(show.date, "1977-05-21");
        assert!(show.is_nll);
        assert_eq!(show.source_type, SourceType::Sbd);
    }

    #[test]
    fn test_nll_beats_non_nll_sbd_real_folders() {
        // Real scenario: NLL folder + non-NLL SBD for same date
        // NLL must win
        let nll = parse_show_folder(
            "1977-05-08 Cornell University Ithica NY - FLAC Dusborne Matrix-20250509T025840Z-001",
            "nll_fid",
            BmanArtist::GratefulDead,
            true,
        ).unwrap();
        let non_nll_etree = parse_show_folder(
            "gd77-05-08.137570.mtx.dusborne.flac16",
            "etree_fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        let non_nll_nice = parse_show_folder(
            "1977-05-08 Cornell University Ithica NY - FLAC Dusborne Matrix",
            "nice_fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();

        let result = dedup_shows(vec![nll, non_nll_etree, non_nll_nice]);
        assert_eq!(result.len(), 1, "Should dedup to single show");
        assert!(result[0].is_nll, "NLL version must win");
    }

    #[test]
    fn test_nll_aud_still_filtered_out() {
        // NLL AUD should still be dropped by the final AUD filter
        let nll_aud = parse_show_folder(
            "gd1977-10-09.170283.Denver, CO.set2.aud.tiffany.andrewF.flac1648",
            "nll_aud_fid",
            BmanArtist::GratefulDead,
            true,
        ).unwrap();
        assert!(nll_aud.is_nll);
        assert_eq!(nll_aud.source_type, SourceType::Aud);

        // If only NLL AUD exists → dropped entirely
        let result = dedup_shows(vec![nll_aud]);
        assert!(result.is_empty(), "NLL AUD should still be filtered out");
    }

    #[test]
    fn test_nll_aud_does_not_shadow_non_nll_sbd() {
        // NLL AUD + non-NLL SBD: AUD is dropped first, non-NLL SBD survives
        let nll_aud = parse_show_folder(
            "gd77-05-08.aud.berger.flac",
            "nll_aud_fid",
            BmanArtist::GratefulDead,
            true,
        ).unwrap();
        let non_nll_sbd = parse_show_folder(
            "gd77-05-08.sbd.miller.flac16",
            "sbd_fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();

        let result = dedup_shows(vec![nll_aud, non_nll_sbd]);
        assert_eq!(result.len(), 1, "Non-NLL SBD should survive when NLL is AUD");
        assert_eq!(result[0].source_type, SourceType::Sbd);
        assert!(!result[0].is_nll);
    }

    #[test]
    fn test_nll_fob_loses_to_aaa_sbd() {
        // Real scenario: NLL has FOB recording, AAA year folder has SBD
        // FOB is classified as AUD → dropped first → SBD survives
        let nll_fob = parse_show_folder(
            "gd1983-05-13.fob.akg.d330bt.senn421.hecht.miller.clugston.147896.flac1648",
            "nll_fid",
            BmanArtist::GratefulDead,
            true, // NLL
        ).unwrap();
        assert_eq!(nll_fob.source_type, SourceType::Aud, "FOB should be classified as AUD");
        assert!(nll_fob.is_nll);

        let aaa_sbd = parse_show_folder(
            "gd83-05-13.sbd.miller.sbeok.flac16",
            "aaa_fid",
            BmanArtist::GratefulDead,
            false, // regular AAA folder
        ).unwrap();
        assert_eq!(aaa_sbd.source_type, SourceType::Sbd);

        let result = dedup_shows(vec![nll_fob, aaa_sbd]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source_type, SourceType::Sbd, "AAA SBD must beat NLL FOB");
        assert!(!result[0].is_nll);
    }

    // --- AUD filtering edge cases ---

    #[test]
    fn test_dedup_aud_only_date_is_dropped() {
        // Only AUD entries exist for this date → entire date removed
        let shows = vec![
            gd_show("1977-05-08", "Cornell", SourceType::Aud, false),
            gd_show("1977-05-08", "Cornell", SourceType::Aud, false),
        ];
        let result = dedup_shows(shows);
        assert!(result.is_empty(), "AUD-only dates must be filtered out");
    }

    #[test]
    fn test_dedup_single_aud_show_is_dropped() {
        // Single AUD show (group.len() == 1 fast path) → still filtered
        let shows = vec![
            gd_show("1977-05-08", "Cornell", SourceType::Aud, false),
        ];
        let result = dedup_shows(shows);
        assert!(result.is_empty(), "Single AUD show must be filtered out");
    }

    #[test]
    fn test_dedup_unknown_source_kept() {
        // Unknown source type is NOT dropped (only AUD is)
        let shows = vec![
            gd_show("1977-05-08", "Cornell", SourceType::Unknown, false),
        ];
        let result = dedup_shows(shows);
        assert_eq!(result.len(), 1, "Unknown source should be kept");
    }

    #[test]
    fn test_dedup_sbd_plus_aud_keeps_sbd_only() {
        // SBD + AUD for same date → only SBD survives
        let shows = vec![
            gd_show("1977-05-08", "Cornell", SourceType::Sbd, false),
            gd_show("1977-05-08", "Cornell", SourceType::Aud, false),
        ];
        let result = dedup_shows(shows);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source_type, SourceType::Sbd);
    }

    #[test]
    fn test_dedup_mtx_plus_aud_keeps_mtx() {
        // Matrix + AUD → Matrix wins (higher source type), AUD dropped
        let shows = vec![
            gd_show("1977-05-08", "Cornell", SourceType::Mtx, false),
            gd_show("1977-05-08", "Cornell", SourceType::Aud, false),
        ];
        let result = dedup_shows(shows);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source_type, SourceType::Mtx);
    }

    #[test]
    fn test_dedup_aud_with_different_dates_all_dropped() {
        // Multiple dates, all AUD → all dropped
        let shows = vec![
            gd_show("1977-05-08", "Cornell", SourceType::Aud, false),
            gd_show("1977-09-03", "Englishtown", SourceType::Aud, false),
        ];
        let result = dedup_shows(shows);
        assert!(result.is_empty());
    }

    #[test]
    fn test_dedup_mixed_some_aud_only_dates_dropped() {
        // Date A: SBD + AUD → keeps SBD
        // Date B: AUD only → dropped entirely
        let shows = vec![
            gd_show("1977-05-08", "Cornell", SourceType::Sbd, false),
            gd_show("1977-05-08", "Cornell", SourceType::Aud, false),
            gd_show("1977-09-03", "Englishtown", SourceType::Aud, false),
        ];
        let result = dedup_shows(shows);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].date, "1977-05-08");
        assert_eq!(result[0].source_type, SourceType::Sbd);
    }
}
