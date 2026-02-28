//! setlist.fm API client for fetching song titles by show date + artist.
//!
//! Supports multi-key rotation: when a key hits 429 (rate limit), rotates to
//! the next key and continues. All keys exhausted → error.

use std::sync::atomic::{AtomicUsize, Ordering};

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

    /// Advance to the next key. Returns `false` if all keys are exhausted.
    pub fn rotate(&self) -> bool {
        let next = self.current.load(Ordering::Relaxed) + 1;
        if next < self.keys.len() {
            self.current.store(next, Ordering::Relaxed);
            true
        } else {
            false
        }
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

/// Query setlist.fm for the track listing of a show.
///
/// `date` must be "YYYY-MM-DD". The function converts it to "DD-MM-YYYY" for
/// the API. Returns the flattened list of song names from all sets, or
/// `Ok(None)` if no results are found or the request fails.
/// On 429, rotates to the next key and retries.
pub async fn fetch_setlist(
    client: &reqwest::Client,
    keys: &SetlistFmKeys,
    artist_name: &str,
    date: &str, // "YYYY-MM-DD"
) -> Result<Option<Vec<String>>, String> {
    let body = query_with_rotation(client, keys, artist_name, date).await?;
    let Some(body) = body else { return Ok(None) };
    Ok(extract_songs(&body))
}

/// Venue information extracted from a setlist.fm response.
#[derive(Debug, Clone)]
pub struct SetlistVenue {
    pub venue_name: String,
    pub city: String,
    pub state: String,
}

/// Query setlist.fm for venue information for a show.
///
/// `date` must be "YYYY-MM-DD". Returns venue/city/state or `Ok(None)` on miss.
/// On 429, rotates to the next key and retries.
pub async fn fetch_setlist_venue(
    client: &reqwest::Client,
    keys: &SetlistFmKeys,
    artist_name: &str,
    date: &str,
) -> Result<Option<SetlistVenue>, String> {
    let body = query_with_rotation(client, keys, artist_name, date).await?;
    let Some(body) = body else { return Ok(None) };
    Ok(extract_venue(&body))
}

/// Internal: query setlist.fm with automatic key rotation on 429 and
/// single retry on 5xx server errors.
async fn query_with_rotation(
    client: &reqwest::Client,
    keys: &SetlistFmKeys,
    artist_name: &str,
    date: &str,
) -> Result<Option<serde_json::Value>, String> {
    let mut retried_5xx = false;
    loop {
        let key = keys
            .current_key()
            .ok_or_else(|| "setlist.fm: all API keys exhausted".to_string())?;

        match query_setlistfm(client, key, artist_name, date).await {
            Ok(v) => return Ok(v),
            Err(e) if e.contains("HTTP 429") => {
                if !keys.rotate() {
                    return Err("setlist.fm: all API keys exhausted (429 on every key)".to_string());
                }
                tracing::warn!("setlist.fm 429 — rotating to {}", keys.key_label());
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
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
    let setlists = body.get("setlist")?.as_array()?;
    let first = setlists.first()?;

    let sets = first.get("sets")?.get("set")?.as_array()?;

    let mut songs: Vec<String> = Vec::new();
    for set in sets {
        if let Some(song_arr) = set.get("song").and_then(|s| s.as_array()) {
            for song in song_arr {
                if let Some(name) = song.get("name").and_then(|n| n.as_str()) {
                    let name = name.trim();
                    if !name.is_empty() {
                        songs.push(name.to_string());
                    }
                }
            }
        }
    }

    if songs.is_empty() {
        None
    } else {
        Some(songs)
    }
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
        assert!(!keys.rotate()); // only one key, can't rotate
    }

    #[test]
    fn test_keys_empty_string() {
        let keys = SetlistFmKeys::from_comma_separated("");
        assert!(keys.is_empty());
        assert_eq!(keys.current_key(), None);
    }

    #[test]
    fn test_keys_rotation() {
        let keys = SetlistFmKeys::from_comma_separated("k1,k2,k3");
        assert_eq!(keys.current_key(), Some("k1"));
        assert_eq!(keys.key_label(), "key 1/3");

        assert!(keys.rotate());
        assert_eq!(keys.current_key(), Some("k2"));
        assert_eq!(keys.key_label(), "key 2/3");

        assert!(keys.rotate());
        assert_eq!(keys.current_key(), Some("k3"));
        assert_eq!(keys.key_label(), "key 3/3");

        assert!(!keys.rotate()); // exhausted
        assert_eq!(keys.current_key(), Some("k3")); // stays on last
    }

    #[test]
    fn test_keys_exhaustion() {
        let keys = SetlistFmKeys::from_comma_separated("only");
        assert!(!keys.rotate());
        // current_key still works after failed rotate
        assert_eq!(keys.current_key(), Some("only"));
    }
}
