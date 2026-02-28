//! setlist.fm API client for fetching song titles by show date + artist.
//!
//! Returns `Ok(None)` on any failure — the caller treats this as a non-fatal
//! miss and falls back to lower-ranked title sources.

/// Query setlist.fm for the track listing of a show.
///
/// `date` must be "YYYY-MM-DD". The function converts it to "DD-MM-YYYY" for
/// the API. Returns the flattened list of song names from all sets, or
/// `Ok(None)` if no results are found or the request fails.
pub async fn fetch_setlist(
    client: &reqwest::Client,
    api_key: &str,
    artist_name: &str,
    date: &str, // "YYYY-MM-DD"
) -> Result<Option<Vec<String>>, String> {
    let body = query_setlistfm(client, api_key, artist_name, date).await?;
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
pub async fn fetch_setlist_venue(
    client: &reqwest::Client,
    api_key: &str,
    artist_name: &str,
    date: &str,
) -> Result<Option<SetlistVenue>, String> {
    let body = query_setlistfm(client, api_key, artist_name, date).await?;
    let Some(body) = body else { return Ok(None) };
    Ok(extract_venue(&body))
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
}
