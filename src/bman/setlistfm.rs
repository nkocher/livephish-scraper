//! setlist.fm API client for fetching song titles by show date + artist.
//!
//! Supports multi-key rotation: when a key hits 429 (rate limit), rotates to
//! the next key and continues. All keys exhausted → error.
//!
//! `SfmCache` provides a query-keyed response cache (`setlistfm_cache.json`).
//! Keys are `"{artist_name_lc}:{api_date}"` — parser changes produce different
//! keys so the cache is never invalidated by parser fixes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Current time as Unix seconds. Returns 0 on clock error (safe: entries just
/// appear permanently fresh, which is the less-harmful failure mode).
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Multiple setlist.fm API keys with automatic rotation on 429.
///
/// Build once per workflow and pass by reference so rotation state persists
/// across calls. Uses `AtomicUsize` (not `Cell`) for `Sync` safety across
/// `.await` points.
pub struct SetlistFmKeys {
    keys: Vec<String>,
    current: AtomicUsize,
}

impl SetlistFmKeys {
    /// Parse comma-separated API keys, trimming whitespace and filtering empties.
    /// Duplicate keys are removed (keeps first occurrence).
    pub fn from_comma_separated(s: &str) -> Self {
        let mut seen = std::collections::HashSet::new();
        let keys: Vec<String> = s
            .split(',')
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .filter(|k| seen.insert(k.clone()))
            .collect();
        Self {
            keys,
            current: AtomicUsize::new(0),
        }
    }

    /// The currently active API key, or `None` if no keys were provided.
    pub fn current_key(&self) -> Option<&str> {
        let idx = self.current.load(Ordering::Relaxed);
        self.keys.get(idx).map(|s| s.as_str())
    }

    /// Advance to the next key (wraps around). Returns the new index.
    pub fn rotate(&self) -> usize {
        let len = self.keys.len();
        if len == 0 {
            return 0;
        }
        let next = (self.current.load(Ordering::Relaxed) + 1) % len;
        self.current.store(next, Ordering::Relaxed);
        next
    }

    /// True when no keys were provided.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Human-readable label like "key 2/4" (never logs raw key values).
    pub fn key_label(&self) -> String {
        let idx = self.current.load(Ordering::Relaxed) + 1;
        format!("key {}/{}", idx, self.keys.len())
    }

    /// Total number of keys.
    pub fn len(&self) -> usize {
        self.keys.len()
    }
}

// ---- Cache types ------------------------------------------------------------

/// Schema version for `setlistfm_cache.json`. Bump on struct changes.
const SFM_CACHE_SCHEMA_VERSION: u32 = 1;

/// TTL for `NotFound` entries: 90 days in seconds.
const NOT_FOUND_TTL_SECS: u64 = 90 * 24 * 3600;

/// Outer JSON wrapper with schema version for forward-compatibility.
#[derive(Serialize, Deserialize)]
struct SfmCacheFile {
    version: u32,
    entries: HashMap<String, SfmResponse>,
}

/// A cached setlist.fm response (either a successful result or a 404).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SfmResponse {
    pub status: SfmStatus,
    pub fetched_at: u64,
}

/// The payload of a cached setlist.fm query.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum SfmStatus {
    /// 404 — no setlist found for this query.
    NotFound,
    /// 200 — full venue + songs response.
    Found {
        venue: SetlistVenue,
        songs: Vec<String>,
    },
}

/// Combined venue + songs result from a single setlist.fm API call.
#[derive(Debug, Clone)]
pub struct SetlistFullResult {
    pub venue: SetlistVenue,
    pub songs: Vec<String>,
}

/// Query-keyed setlist.fm response cache.
///
/// Keys: `"{artist_name_lc}:{api_date}"` (e.g. `"grateful dead:08-05-1977"`).
/// Parser changes produce different keys → cache never invalidated by parser fixes.
pub struct SfmCache {
    entries: HashMap<String, SfmResponse>,
    dirty: bool,
    cache_dir: PathBuf,
}

impl SfmCache {
    /// Build a cache key from artist name and YYYY-MM-DD date.
    pub fn cache_key(artist_name: &str, date: &str) -> Result<String, String> {
        let api_date = convert_date(date)?;
        Ok(format!("{}:{}", artist_name.trim().to_lowercase(), api_date))
    }

    /// Load cache from disk, or create empty if missing/corrupt/version-mismatch.
    pub fn load(cache_dir: &Path) -> Self {
        let path = cache_dir.join("setlistfm_cache.json");
        let bak = cache_dir.join("setlistfm_cache.json.bak");
        let backup = || { let _ = std::fs::rename(&path, &bak); };

        let entries = match std::fs::read_to_string(&path) {
            Err(_) => HashMap::new(), // missing file is the common case — no warning needed
            Ok(data) => match serde_json::from_str::<SfmCacheFile>(&data) {
                Ok(f) if f.version == SFM_CACHE_SCHEMA_VERSION => f.entries,
                Ok(f) => {
                    tracing::warn!(
                        "setlistfm_cache.json: schema v{} != v{}, backing up and starting fresh",
                        f.version,
                        SFM_CACHE_SCHEMA_VERSION
                    );
                    backup();
                    HashMap::new()
                }
                Err(e) => {
                    tracing::warn!("setlistfm_cache.json: parse error ({e}), starting fresh");
                    backup();
                    HashMap::new()
                }
            },
        };

        Self {
            entries,
            dirty: false,
            cache_dir: cache_dir.to_path_buf(),
        }
    }

    /// Look up a cached response. Returns `None` on miss or expired `NotFound`.
    pub fn lookup(&self, artist_name: &str, date: &str) -> Option<&SfmStatus> {
        let key = Self::cache_key(artist_name, date).ok()?;
        let entry = self.entries.get(&key)?;

        // NotFound entries expire after TTL
        if matches!(entry.status, SfmStatus::NotFound)
            && now_secs().saturating_sub(entry.fetched_at) > NOT_FOUND_TTL_SECS
        {
            return None;
        }

        Some(&entry.status)
    }

    /// Insert a response into the cache.
    pub fn insert(&mut self, artist_name: &str, date: &str, status: SfmStatus) {
        let Ok(key) = Self::cache_key(artist_name, date) else {
            return;
        };
        self.entries.insert(key, SfmResponse {
            status,
            fetched_at: now_secs(),
        });
        self.dirty = true;
    }

    /// Clear entries matching a predicate on the cache key.
    pub fn clear_matching<F: Fn(&str) -> bool>(&mut self, predicate: F) {
        let before = self.entries.len();
        self.entries.retain(|key, _| !predicate(key));
        if self.entries.len() != before {
            self.dirty = true;
        }
    }

    /// Clear all `NotFound` entries (re-query 404s on next enrichment).
    pub fn clear_not_found(&mut self) {
        let before = self.entries.len();
        self.entries
            .retain(|_, v| !matches!(v.status, SfmStatus::NotFound));
        if self.entries.len() != before {
            self.dirty = true;
        }
    }

    /// Return `(cached_hits, expired_notfound, total)` for logging.
    pub fn stats(&self) -> (usize, usize, usize) {
        let now = now_secs();
        let total = self.entries.len();
        let expired = self
            .entries
            .values()
            .filter(|e| {
                matches!(e.status, SfmStatus::NotFound)
                    && now.saturating_sub(e.fetched_at) > NOT_FOUND_TTL_SECS
            })
            .count();
        (total - expired, expired, total)
    }

    /// Write cache to disk if dirty. Uses `.part` + rename for atomicity.
    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let path = self.cache_dir.join("setlistfm_cache.json");
        let part = self.cache_dir.join("setlistfm_cache.json.part");
        let wrapper = SfmCacheFile {
            version: SFM_CACHE_SCHEMA_VERSION,
            entries: self.entries.clone(),
        };
        match serde_json::to_string(&wrapper) {
            Ok(data) => {
                if std::fs::write(&part, &data).is_ok() {
                    let _ = std::fs::rename(&part, &path);
                    self.dirty = false;
                    tracing::debug!("setlistfm_cache.json: saved {} entries", self.entries.len());
                }
            }
            Err(e) => tracing::warn!("setlistfm_cache.json: serialize error ({e})"),
        }
    }

    /// Number of entries in the cache.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

// ---- Unified fetch ----------------------------------------------------------

/// Query setlist.fm for venue + songs in a single API call.
///
/// Returns `Ok(Some(result))` on 200, `Ok(None)` on 404, `Err` on other errors.
/// Does NOT check or update the cache — that's the caller's responsibility.
pub async fn fetch_setlist_full(
    client: &reqwest::Client,
    keys: &SetlistFmKeys,
    artist_name: &str,
    date: &str,
) -> Result<Option<SetlistFullResult>, String> {
    let body = query_with_rotation(client, keys, artist_name, date).await?;
    let Some(body) = body else {
        return Ok(None);
    };
    let venue = extract_venue(&body);
    let songs = extract_songs(&body);

    // Treat as NotFound only when both venue and songs are absent.
    if venue.is_none() && songs.is_none() {
        return Ok(None);
    }

    Ok(Some(SetlistFullResult {
        venue: venue.unwrap_or_else(|| SetlistVenue {
            venue_name: String::new(),
            city: String::new(),
            state: String::new(),
        }),
        songs: songs.unwrap_or_default(),
    }))
}

/// Venue information extracted from a setlist.fm response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetlistVenue {
    pub venue_name: String,
    pub city: String,
    pub state: String,
}

/// Internal: query setlist.fm with automatic key rotation on 429 and
/// single retry on 5xx server errors.
///
/// Escalating backoff on consecutive 429s addresses IP-level burst penalty
/// when rapid-fire 429s from depleted keys poison fresh keys.
async fn query_with_rotation(
    client: &reqwest::Client,
    keys: &SetlistFmKeys,
    artist_name: &str,
    date: &str,
) -> Result<Option<serde_json::Value>, String> {
    let mut retried_5xx = false;
    let mut consecutive_429s = 0u32;
    let num_keys = keys.len();
    loop {
        let key = keys
            .current_key()
            .ok_or_else(|| "setlist.fm: all API keys exhausted".to_string())?;

        match query_setlistfm(client, key, artist_name, date).await {
            Ok(v) => return Ok(v),
            Err(e) if e.contains("HTTP 429") => {
                consecutive_429s += 1;
                // Full cycle: every key got 429 without a single success
                if num_keys > 0 && consecutive_429s as usize >= num_keys {
                    return Err("setlist.fm: all API keys exhausted (429 on every key)".to_string());
                }
                keys.rotate();
                let backoff = std::cmp::min(2u64.pow(consecutive_429s.min(5)), 30);
                tracing::warn!(
                    "setlist.fm 429 — rotating to {} (backoff {}s)",
                    keys.key_label(),
                    backoff
                );
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
            }
            Err(e) if !retried_5xx && e.contains("HTTP 5") => {
                retried_5xx = true;
                tracing::warn!("setlist.fm 5xx — retrying once after 3s");
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Shared HTTP request to setlist.fm search API.
///
/// Returns `Ok(None)` on 404 (no results), `Err` on other HTTP errors.
async fn query_setlistfm(
    client: &reqwest::Client,
    api_key: &str,
    artist_name: &str,
    date: &str,
) -> Result<Option<serde_json::Value>, String> {
    let api_date = convert_date(date)?;

    let url = format!(
        "https://api.setlist.fm/rest/1.0/search/setlists?artistName={}&date={}",
        urlencoding::encode(artist_name),
        urlencoding::encode(&api_date),
    );

    let response = client
        .get(&url)
        .header("x-api-key", api_key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("setlist.fm request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        return if status.as_u16() == 404 {
            Ok(None)
        } else {
            Err(format!("setlist.fm returned HTTP {status}"))
        };
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("setlist.fm JSON parse error: {e}"))?;

    Ok(Some(body))
}

/// Convert "YYYY-MM-DD" to "DD-MM-YYYY" for the setlist.fm API.
pub(crate) fn convert_date(date: &str) -> Result<String, String> {
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3
        || parts[0].len() != 4
        || parts[1].len() != 2
        || parts[2].len() != 2
    {
        return Err(format!("Invalid date format (expected YYYY-MM-DD): {date}"));
    }
    Ok(format!("{}-{}-{}", parts[2], parts[1], parts[0]))
}

/// Extract venue information from a setlist.fm search response.
///
/// Response structure: `{ "setlist": [{ "venue": { "name": "...", "city": { "name": "...", "stateCode": "...", "state": "..." } } }] }`
pub(crate) fn extract_venue(body: &serde_json::Value) -> Option<SetlistVenue> {
    let setlists = body.get("setlist")?.as_array()?;
    let first = setlists.first()?;
    let venue = first.get("venue")?;

    let venue_name = venue
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let city_obj = venue.get("city");
    let city = city_obj
        .and_then(|c| c.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    // Use stateCode (e.g., "NY") if available, fall back to full state name
    let state = city_obj
        .and_then(|c| c.get("stateCode").or_else(|| c.get("state")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if venue_name.is_empty() && city.is_empty() {
        None
    } else {
        Some(SetlistVenue {
            venue_name,
            city,
            state,
        })
    }
}

/// Extract song names from a setlist.fm search response, flattening all sets.
fn extract_songs(body: &serde_json::Value) -> Option<Vec<String>> {
    let sets = body
        .get("setlist")?
        .as_array()?
        .first()?
        .get("sets")?
        .get("set")?
        .as_array()?;

    let songs: Vec<String> = sets
        .iter()
        .flat_map(|set| set.get("song").and_then(|s| s.as_array()).into_iter().flatten())
        .filter_map(|song| song.get("name").and_then(|n| n.as_str()))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect();

    if songs.is_empty() { None } else { Some(songs) }
}

// ---- URL encoding helper (avoid adding a new crate) -------------------------

mod urlencoding {
    /// Percent-encode a string for use in a URL query parameter value.
    pub(crate) fn encode(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for b in s.bytes() {
            match b {
                // Unreserved characters per RFC 3986
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char);
                }
                _ => {
                    out.push('%');
                    out.push(hex_nibble(b >> 4));
                    out.push(hex_nibble(b & 0x0F));
                }
            }
        }
        out
    }

    fn hex_nibble(n: u8) -> char {
        match n {
            0..=9 => (b'0' + n) as char,
            _ => (b'A' + n - 10) as char,
        }
    }
}

// ---- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- convert_date -------------------------------------------------------

    #[test]
    fn test_convert_date_standard() {
        assert_eq!(convert_date("1977-05-08").unwrap(), "08-05-1977");
    }

    #[test]
    fn test_convert_date_leading_zeros() {
        assert_eq!(convert_date("2003-01-02").unwrap(), "02-01-2003");
    }

    #[test]
    fn test_convert_date_invalid_format() {
        assert!(convert_date("1977/05/08").is_err());
        assert!(convert_date("77-05-08").is_err());
        assert!(convert_date("").is_err());
        assert!(convert_date("not-a-date").is_err());
    }

    // ---- extract_songs ------------------------------------------------------

    fn make_setlist_body(songs: &[&str]) -> serde_json::Value {
        let song_arr: Vec<serde_json::Value> = songs
            .iter()
            .map(|name| serde_json::json!({"name": name}))
            .collect();
        serde_json::json!({
            "setlist": [{
                "sets": {
                    "set": [{"song": song_arr}]
                }
            }]
        })
    }

    #[test]
    fn test_extract_songs_basic() {
        let body = make_setlist_body(&["Scarlet Begonias", "Fire on the Mountain", "Eyes of the World"]);
        let songs = extract_songs(&body).unwrap();
        assert_eq!(songs, vec!["Scarlet Begonias", "Fire on the Mountain", "Eyes of the World"]);
    }

    #[test]
    fn test_extract_songs_multi_set() {
        let body = serde_json::json!({
            "setlist": [{
                "sets": {
                    "set": [
                        {"song": [{"name": "Song A"}, {"name": "Song B"}]},
                        {"song": [{"name": "Song C"}]}
                    ]
                }
            }]
        });
        let songs = extract_songs(&body).unwrap();
        assert_eq!(songs, vec!["Song A", "Song B", "Song C"]);
    }

    #[test]
    fn test_extract_songs_empty_sets() {
        let body = serde_json::json!({
            "setlist": [{
                "sets": {"set": []}
            }]
        });
        assert!(extract_songs(&body).is_none());
    }

    #[test]
    fn test_extract_songs_no_setlists() {
        let body = serde_json::json!({"setlist": []});
        assert!(extract_songs(&body).is_none());
    }

    #[test]
    fn test_extract_songs_missing_setlist_key() {
        let body = serde_json::json!({});
        assert!(extract_songs(&body).is_none());
    }

    #[test]
    fn test_extract_songs_skips_empty_names() {
        let body = serde_json::json!({
            "setlist": [{
                "sets": {
                    "set": [{
                        "song": [{"name": ""}, {"name": "Real Song"}, {"name": "  "}]
                    }]
                }
            }]
        });
        let songs = extract_songs(&body).unwrap();
        assert_eq!(songs, vec!["Real Song"]);
    }

    // ---- extract_venue ------------------------------------------------------

    fn make_venue_body(venue: &str, city: &str, state_code: &str) -> serde_json::Value {
        serde_json::json!({
            "setlist": [{
                "venue": {
                    "name": venue,
                    "city": {
                        "name": city,
                        "stateCode": state_code,
                        "state": "Full State Name",
                        "country": {"code": "US", "name": "United States"}
                    }
                },
                "sets": {"set": []}
            }]
        })
    }

    #[test]
    fn test_extract_venue_full() {
        let body = make_venue_body("Barton Hall", "Ithaca", "NY");
        let venue = extract_venue(&body).unwrap();
        assert_eq!(venue.venue_name, "Barton Hall");
        assert_eq!(venue.city, "Ithaca");
        assert_eq!(venue.state, "NY");
    }

    #[test]
    fn test_extract_venue_no_state_code_uses_state() {
        let body = serde_json::json!({
            "setlist": [{
                "venue": {
                    "name": "Wembley Stadium",
                    "city": {
                        "name": "London",
                        "state": "England",
                        "country": {"code": "GB", "name": "United Kingdom"}
                    }
                },
                "sets": {"set": []}
            }]
        });
        let venue = extract_venue(&body).unwrap();
        assert_eq!(venue.venue_name, "Wembley Stadium");
        assert_eq!(venue.city, "London");
        assert_eq!(venue.state, "England");
    }

    #[test]
    fn test_extract_venue_empty_venue_name_with_city() {
        let body = serde_json::json!({
            "setlist": [{
                "venue": {
                    "name": "",
                    "city": {"name": "San Francisco", "stateCode": "CA"}
                },
                "sets": {"set": []}
            }]
        });
        let venue = extract_venue(&body).unwrap();
        assert_eq!(venue.venue_name, "");
        assert_eq!(venue.city, "San Francisco");
        assert_eq!(venue.state, "CA");
    }

    #[test]
    fn test_extract_venue_no_venue_object() {
        let body = serde_json::json!({"setlist": [{"sets": {"set": []}}]});
        assert!(extract_venue(&body).is_none());
    }

    #[test]
    fn test_extract_venue_empty_response() {
        let body = serde_json::json!({"setlist": []});
        assert!(extract_venue(&body).is_none());
    }

    #[test]
    fn test_extract_venue_all_empty() {
        let body = serde_json::json!({
            "setlist": [{
                "venue": {"name": "", "city": {"name": ""}},
                "sets": {"set": []}
            }]
        });
        assert!(extract_venue(&body).is_none());
    }

    // ---- urlencoding --------------------------------------------------------

    #[test]
    fn test_url_encode_plain() {
        assert_eq!(urlencoding::encode("Grateful Dead"), "Grateful%20Dead");
    }

    #[test]
    fn test_url_encode_special_chars() {
        assert_eq!(urlencoding::encode("08-05-1977"), "08-05-1977");
    }

    #[test]
    fn test_url_encode_already_safe() {
        assert_eq!(urlencoding::encode("abc123"), "abc123");
    }

    // ---- SetlistFmKeys --------------------------------------------------

    #[test]
    fn test_keys_from_comma_separated_basic() {
        let keys = SetlistFmKeys::from_comma_separated("aaa,bbb,ccc");
        assert_eq!(keys.len(), 3);
        assert_eq!(keys.current_key(), Some("aaa"));
    }

    #[test]
    fn test_keys_trims_whitespace() {
        let keys = SetlistFmKeys::from_comma_separated("  aaa , bbb , ccc  ");
        assert_eq!(keys.len(), 3);
        assert_eq!(keys.current_key(), Some("aaa"));
    }

    #[test]
    fn test_keys_filters_empty() {
        let keys = SetlistFmKeys::from_comma_separated("aaa,,bbb,,,");
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn test_keys_dedup() {
        let keys = SetlistFmKeys::from_comma_separated("aaa,bbb,aaa,ccc,bbb");
        assert_eq!(keys.len(), 3);
        assert_eq!(keys.current_key(), Some("aaa"));
    }

    #[test]
    fn test_keys_single_key_compat() {
        let keys = SetlistFmKeys::from_comma_separated("single-key");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys.current_key(), Some("single-key"));
        assert_eq!(keys.rotate(), 0); // wraps back to index 0
        assert_eq!(keys.current_key(), Some("single-key"));
    }

    #[test]
    fn test_keys_empty_string() {
        let keys = SetlistFmKeys::from_comma_separated("");
        assert!(keys.is_empty());
        assert_eq!(keys.current_key(), None);
    }

    #[test]
    fn test_keys_rotation_wraps() {
        let keys = SetlistFmKeys::from_comma_separated("k1,k2,k3");
        assert_eq!(keys.current_key(), Some("k1"));
        assert_eq!(keys.key_label(), "key 1/3");

        assert_eq!(keys.rotate(), 1);
        assert_eq!(keys.current_key(), Some("k2"));
        assert_eq!(keys.key_label(), "key 2/3");

        assert_eq!(keys.rotate(), 2);
        assert_eq!(keys.current_key(), Some("k3"));
        assert_eq!(keys.key_label(), "key 3/3");

        // Wraps back to first key
        assert_eq!(keys.rotate(), 0);
        assert_eq!(keys.current_key(), Some("k1"));
        assert_eq!(keys.key_label(), "key 1/3");
    }

    #[test]
    fn test_keys_single_wraps() {
        let keys = SetlistFmKeys::from_comma_separated("only");
        assert_eq!(keys.rotate(), 0);
        assert_eq!(keys.current_key(), Some("only"));
    }

    // ---- SfmCache -----------------------------------------------------------

    fn make_temp_cache_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn test_cache_key_normalization() {
        let key = SfmCache::cache_key("Grateful Dead", "1977-05-08").unwrap();
        assert_eq!(key, "grateful dead:08-05-1977");

        // Whitespace trimmed
        let key2 = SfmCache::cache_key("  Grateful Dead  ", "1977-05-08").unwrap();
        assert_eq!(key2, "grateful dead:08-05-1977");

        // Case insensitive
        let key3 = SfmCache::cache_key("GRATEFUL DEAD", "1977-05-08").unwrap();
        assert_eq!(key3, "grateful dead:08-05-1977");
    }

    #[test]
    fn test_cache_key_invalid_date() {
        assert!(SfmCache::cache_key("Grateful Dead", "bad-date").is_err());
    }

    #[test]
    fn test_cache_load_empty() {
        let dir = make_temp_cache_dir();
        let cache = SfmCache::load(dir.path());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_cache_insert_and_lookup_found() {
        let dir = make_temp_cache_dir();
        let mut cache = SfmCache::load(dir.path());

        let venue = SetlistVenue {
            venue_name: "Barton Hall".to_string(),
            city: "Ithaca".to_string(),
            state: "NY".to_string(),
        };
        cache.insert(
            "Grateful Dead",
            "1977-05-08",
            SfmStatus::Found {
                venue: venue.clone(),
                songs: vec!["Scarlet Begonias".to_string()],
            },
        );

        let status = cache.lookup("Grateful Dead", "1977-05-08");
        assert!(status.is_some());
        match status.unwrap() {
            SfmStatus::Found { venue: v, songs } => {
                assert_eq!(v.venue_name, "Barton Hall");
                assert_eq!(songs.len(), 1);
            }
            SfmStatus::NotFound => panic!("Expected Found"),
        }
    }

    #[test]
    fn test_cache_lookup_not_found_within_ttl() {
        let dir = make_temp_cache_dir();
        let mut cache = SfmCache::load(dir.path());

        cache.insert("Grateful Dead", "1965-01-01", SfmStatus::NotFound);

        // Should still be present (just inserted, well within TTL)
        let status = cache.lookup("Grateful Dead", "1965-01-01");
        assert!(matches!(status, Some(SfmStatus::NotFound)));
    }

    #[test]
    fn test_cache_lookup_not_found_expired() {
        let dir = make_temp_cache_dir();
        let mut cache = SfmCache::load(dir.path());

        // Manually insert with an old timestamp
        let key = SfmCache::cache_key("Grateful Dead", "1965-01-01").unwrap();
        cache.entries.insert(
            key,
            SfmResponse {
                status: SfmStatus::NotFound,
                fetched_at: 0, // epoch — definitely expired
            },
        );

        // Should return None (expired)
        assert!(cache.lookup("Grateful Dead", "1965-01-01").is_none());
    }

    #[test]
    fn test_cache_found_never_expires() {
        let dir = make_temp_cache_dir();
        let mut cache = SfmCache::load(dir.path());

        let key = SfmCache::cache_key("Grateful Dead", "1977-05-08").unwrap();
        cache.entries.insert(
            key,
            SfmResponse {
                status: SfmStatus::Found {
                    venue: SetlistVenue {
                        venue_name: "Barton Hall".to_string(),
                        city: "Ithaca".to_string(),
                        state: "NY".to_string(),
                    },
                    songs: vec!["Scarlet Begonias".to_string()],
                },
                fetched_at: 0, // epoch — old, but Found never expires
            },
        );

        assert!(cache.lookup("Grateful Dead", "1977-05-08").is_some());
    }

    #[test]
    fn test_cache_save_and_reload() {
        let dir = make_temp_cache_dir();
        let mut cache = SfmCache::load(dir.path());

        cache.insert(
            "Grateful Dead",
            "1977-05-08",
            SfmStatus::Found {
                venue: SetlistVenue {
                    venue_name: "Barton Hall".to_string(),
                    city: "Ithaca".to_string(),
                    state: "NY".to_string(),
                },
                songs: vec!["Scarlet Begonias".to_string(), "Fire on the Mountain".to_string()],
            },
        );
        cache.insert("Grateful Dead", "1965-01-01", SfmStatus::NotFound);
        cache.save();

        // Reload from disk
        let cache2 = SfmCache::load(dir.path());
        assert_eq!(cache2.len(), 2);
        assert!(cache2.lookup("Grateful Dead", "1977-05-08").is_some());
        assert!(matches!(
            cache2.lookup("Grateful Dead", "1965-01-01"),
            Some(SfmStatus::NotFound)
        ));
    }

    #[test]
    fn test_cache_save_noop_when_clean() {
        let dir = make_temp_cache_dir();
        let mut cache = SfmCache::load(dir.path());
        cache.save();
        // No file should be created since nothing was inserted
        assert!(!dir.path().join("setlistfm_cache.json").exists());
    }

    #[test]
    fn test_cache_clear_matching() {
        let dir = make_temp_cache_dir();
        let mut cache = SfmCache::load(dir.path());

        cache.insert("Grateful Dead", "1977-05-08", SfmStatus::NotFound);
        cache.insert("Grateful Dead", "1977-05-09", SfmStatus::NotFound);
        cache.insert("Jerry Garcia Band", "1977-05-08", SfmStatus::NotFound);

        cache.clear_matching(|key| key.contains("05-1977"));
        // All three contain "05-1977" in their api_date portion
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_cache_clear_matching_by_date() {
        let dir = make_temp_cache_dir();
        let mut cache = SfmCache::load(dir.path());

        cache.insert("Grateful Dead", "1977-05-08", SfmStatus::NotFound);
        cache.insert("Grateful Dead", "1978-11-24", SfmStatus::NotFound);

        cache.clear_matching(|key| key.contains("08-05-1977"));
        assert_eq!(cache.len(), 1);
        assert!(cache.lookup("Grateful Dead", "1978-11-24").is_some());
    }

    #[test]
    fn test_cache_clear_not_found() {
        let dir = make_temp_cache_dir();
        let mut cache = SfmCache::load(dir.path());

        cache.insert(
            "Grateful Dead",
            "1977-05-08",
            SfmStatus::Found {
                venue: SetlistVenue {
                    venue_name: "Barton Hall".to_string(),
                    city: "Ithaca".to_string(),
                    state: "NY".to_string(),
                },
                songs: vec![],
            },
        );
        cache.insert("Grateful Dead", "1965-01-01", SfmStatus::NotFound);

        cache.clear_not_found();
        assert_eq!(cache.len(), 1);
        assert!(cache.lookup("Grateful Dead", "1977-05-08").is_some());
        assert!(cache.lookup("Grateful Dead", "1965-01-01").is_none());
    }

    #[test]
    fn test_cache_stats() {
        let dir = make_temp_cache_dir();
        let mut cache = SfmCache::load(dir.path());

        cache.insert(
            "Grateful Dead",
            "1977-05-08",
            SfmStatus::Found {
                venue: SetlistVenue {
                    venue_name: "V".to_string(),
                    city: "C".to_string(),
                    state: "S".to_string(),
                },
                songs: vec![],
            },
        );
        cache.insert("Grateful Dead", "1965-01-01", SfmStatus::NotFound);

        // Manually expire one NotFound
        let key = SfmCache::cache_key("Grateful Dead", "1960-01-01").unwrap();
        cache.entries.insert(
            key,
            SfmResponse {
                status: SfmStatus::NotFound,
                fetched_at: 0,
            },
        );

        let (cached, expired, total) = cache.stats();
        assert_eq!(total, 3);
        assert_eq!(expired, 1); // the 1960 entry
        assert_eq!(cached, 2); // Found + non-expired NotFound
    }

    #[test]
    fn test_cache_schema_version_mismatch() {
        let dir = make_temp_cache_dir();

        // Write a cache file with wrong version
        let bad = serde_json::json!({
            "version": 999,
            "entries": {}
        });
        std::fs::write(
            dir.path().join("setlistfm_cache.json"),
            serde_json::to_string(&bad).unwrap(),
        )
        .unwrap();

        let cache = SfmCache::load(dir.path());
        assert_eq!(cache.len(), 0);
        // Backup should exist
        assert!(dir.path().join("setlistfm_cache.json.bak").exists());
    }

    #[test]
    fn test_cache_corrupt_json() {
        let dir = make_temp_cache_dir();
        std::fs::write(
            dir.path().join("setlistfm_cache.json"),
            "not valid json!!!",
        )
        .unwrap();

        let cache = SfmCache::load(dir.path());
        assert_eq!(cache.len(), 0);
        assert!(dir.path().join("setlistfm_cache.json.bak").exists());
    }
}
