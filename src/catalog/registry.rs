use std::collections::HashMap;
use std::fs;
use std::path::Path;

use once_cell::sync::Lazy;
use regex::Regex;

pub const ARTIST_REGISTRY_SCHEMA_VERSION: i64 = 1;

/// Seed artists as offline fallback.
pub fn seed_artists() -> HashMap<i64, &'static str> {
    HashMap::from([
        (62, "Phish"),
        (461, "Grateful Dead"),
        (1125, "Billy Strings"),
        (1205, "Goose"),
        (58, "Widespread Panic"),
        (22, "Umphrey's McGee"),
        (2, "The String Cheese Incident"),
        (676, "Trey Anastasio"),
        (709, "Gov't Mule"),
        (1138, "Bobby Weir & Wolf Bros"),
        (1020, "Joe Russo's Almost Dead"),
        (196, "moe."),
        (991, "Bruce Springsteen"),
        (321, "Pearl Jam"),
        (628, "Metallica"),
        (823, "Tedeschi Trucks Band"),
        (128, "The Disco Biscuits"),
        (32, "Lettuce"),
    ])
}

/// Normalize artist names for dedup/matching.
///
/// Collapses punctuation/case and treats '&' as 'and' so
/// variants like 'Dead & Company' and 'Dead and Company' resolve together.
pub fn normalize_artist_name(name: &str) -> String {
    static RE_APOSTROPHE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[`'\.]").unwrap());
    static RE_NON_ALNUM: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^a-z0-9]+").unwrap());

    let mut s = name.to_lowercase();
    s = s.replace('&', " and ");
    s = s.replace('+', " and ");
    // Normalize right single quotation mark (U+2019) to ASCII apostrophe before stripping
    s = s.replace('\u{2019}', "'");
    s = RE_APOSTROPHE.replace_all(&s, "").to_string();
    s = RE_NON_ALNUM.replace_all(&s, " ").to_string();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Load artist registry from cache_dir/artists.json.
/// Merges seed artists (don't overwrite existing).
pub fn load_artist_registry(cache_dir: &Path) -> HashMap<i64, String> {
    let registry_file = cache_dir.join("artists.json");
    let mut registry: HashMap<i64, String> = if registry_file.exists() {
        fs::read_to_string(&registry_file)
            .ok()
            .and_then(|s| {
                let map: HashMap<String, String> = serde_json::from_str(&s).ok()?;
                Some(
                    map.into_iter()
                        .filter_map(|(k, v)| k.parse::<i64>().ok().map(|id| (id, v)))
                        .collect(),
                )
            })
            .unwrap_or_default()
    } else {
        HashMap::new()
    };

    // Merge seed artists (don't overwrite)
    for (aid, aname) in seed_artists() {
        registry.entry(aid).or_insert_with(|| aname.to_string());
    }

    registry
}

/// Save artist registry to cache_dir/artists.json.
pub fn save_artist_registry(cache_dir: &Path, registry: &HashMap<i64, String>) {
    let _ = fs::create_dir_all(cache_dir);
    let registry_file = cache_dir.join("artists.json");
    let map: HashMap<String, &String> = registry.iter().map(|(k, v)| (k.to_string(), v)).collect();
    if let Ok(content) = serde_json::to_string_pretty(&map) {
        let _ = fs::write(registry_file, content);
    }
}

/// Group artist IDs by normalized name.
pub fn registry_groups(registry: &HashMap<i64, String>) -> HashMap<String, Vec<i64>> {
    let mut groups: HashMap<String, Vec<i64>> = HashMap::new();
    for (&artist_id, artist_name) in registry {
        let normalized = normalize_artist_name(artist_name);
        groups.entry(normalized).or_default().push(artist_id);
    }
    groups
}

/// Find all artist IDs matching a name (by normalization).
pub fn find_artist_ids(registry: &HashMap<i64, String>, name: &str) -> Vec<i64> {
    let normalized = normalize_artist_name(name);
    if normalized.is_empty() {
        return Vec::new();
    }
    registry_groups(registry)
        .remove(&normalized)
        .unwrap_or_default()
}
