use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::service::Service;

use super::sanitize::sanitize_filename;
use super::serde_helpers::{
    deserialize_safe_i64, deserialize_safe_i64_default_1, deserialize_safe_string,
};

/// Shared display properties for Show and CatalogShow.
pub trait DisplayLocation {
    fn venue_name(&self) -> &str;
    fn venue_city(&self) -> &str;
    fn venue_state(&self) -> &str;
    fn performance_date(&self) -> &str;
    fn performance_date_formatted(&self) -> &str;

    fn display_date(&self) -> &str {
        let formatted = self.performance_date_formatted();
        if !formatted.is_empty() {
            formatted
        } else {
            self.performance_date()
        }
    }

    fn display_location(&self) -> String {
        let parts: Vec<&str> = [self.venue_name(), self.venue_city(), self.venue_state()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
        parts.join(", ")
    }

    fn display_location_short(&self) -> String {
        let parts: Vec<&str> = [self.venue_city(), self.venue_state()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
        parts.join(", ")
    }
}

/// A single track within a show.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    #[serde(rename = "trackID", deserialize_with = "deserialize_safe_i64")]
    pub track_id: i64,

    #[serde(rename = "songID", default, deserialize_with = "deserialize_safe_i64")]
    pub song_id: i64,

    #[serde(
        rename = "songTitle",
        default,
        deserialize_with = "deserialize_safe_string"
    )]
    pub song_title: String,

    #[serde(
        rename = "trackNum",
        default,
        deserialize_with = "deserialize_safe_i64"
    )]
    pub track_num: i64,

    #[serde(
        rename = "discNum",
        default = "default_one",
        deserialize_with = "deserialize_safe_i64_default_1"
    )]
    pub disc_num: i64,

    #[serde(rename = "setNum", default, deserialize_with = "deserialize_safe_i64")]
    pub set_num: i64,

    #[serde(
        rename = "totalRunningTime",
        default,
        deserialize_with = "deserialize_safe_i64"
    )]
    pub duration_seconds: i64,

    #[serde(
        rename = "hhmmssTotalRunningTime",
        default,
        deserialize_with = "deserialize_safe_string"
    )]
    pub duration_display: String,
}

fn default_one() -> i64 {
    1
}

/// Helper to deserialize image URL from nested `{"img": {"url": "..."}}`.
fn deserialize_image_url(data: &serde_json::Value) -> String {
    data.get("img")
        .and_then(|img| img.get("url"))
        .and_then(|url| url.as_str())
        .unwrap_or("")
        .to_string()
}

/// A show/container with full details including tracks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Show {
    #[serde(rename = "containerID", default)]
    pub container_id: i64,

    #[serde(rename = "artistName", default)]
    pub artist_name: String,

    #[serde(rename = "containerInfo", default)]
    pub container_info: String,

    #[serde(rename = "venueName", default)]
    pub venue_name: String,

    #[serde(rename = "venueCity", default)]
    pub venue_city: String,

    #[serde(rename = "venueState", default)]
    pub venue_state: String,

    #[serde(rename = "performanceDate", default)]
    pub performance_date: String,

    #[serde(rename = "performanceDateFormatted", default)]
    pub performance_date_formatted: String,

    #[serde(rename = "performanceDateYear", default)]
    pub performance_date_year: String,

    #[serde(rename = "artistID", default)]
    pub artist_id: i64,

    #[serde(rename = "totalContainerRunningTime", default)]
    pub total_duration_seconds: i64,

    #[serde(rename = "hhmmssTotalRunningTime", default)]
    pub total_duration_display: String,

    #[serde(default, skip)]
    pub tracks: Vec<Track>,

    #[serde(default, skip)]
    pub image_url: String,
}

impl Show {
    /// Parse from API response dict, with defensive handling of malformed data.
    pub fn from_json(data: &serde_json::Value) -> Self {
        // Parse tracks defensively — skip malformed entries
        let tracks: Vec<Track> = data
            .get("tracks")
            .or_else(|| data.get("Tracks"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| serde_json::from_value::<Track>(t.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();

        let image_url = deserialize_image_url(data);

        let coerce_str = |key: &str| -> String {
            data.get(key)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };

        let coerce_i64 = |key: &str| -> i64 {
            data.get(key)
                .and_then(|v| {
                    v.as_i64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                })
                .unwrap_or(0)
        };

        Show {
            container_id: coerce_i64("containerID"),
            artist_name: coerce_str("artistName"),
            container_info: coerce_str("containerInfo").trim().to_string(),
            venue_name: coerce_str("venueName"),
            venue_city: coerce_str("venueCity"),
            venue_state: coerce_str("venueState"),
            performance_date: coerce_str("performanceDate"),
            performance_date_formatted: coerce_str("performanceDateFormatted"),
            performance_date_year: coerce_str("performanceDateYear"),
            artist_id: coerce_i64("artistID"),
            total_duration_seconds: coerce_i64("totalContainerRunningTime"),
            total_duration_display: coerce_str("hhmmssTotalRunningTime"),
            tracks,
            image_url,
        }
    }

    pub fn folder_name(&self) -> String {
        let raw = format!("{} - {}", self.artist_name, self.container_info);
        sanitize_filename(&raw, 120)
    }

    pub fn sets_grouped(&self) -> BTreeMap<i64, Vec<&Track>> {
        let mut sets: BTreeMap<i64, Vec<&Track>> = BTreeMap::new();
        for track in &self.tracks {
            sets.entry(track.set_num).or_default().push(track);
        }
        sets
    }
}

impl DisplayLocation for Show {
    fn venue_name(&self) -> &str {
        &self.venue_name
    }
    fn venue_city(&self) -> &str {
        &self.venue_city
    }
    fn venue_state(&self) -> &str {
        &self.venue_state
    }
    fn performance_date(&self) -> &str {
        &self.performance_date
    }
    fn performance_date_formatted(&self) -> &str {
        &self.performance_date_formatted
    }
}

/// Lightweight show entry from containersAll (no tracks).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CatalogShow {
    #[serde(rename = "containerID", default)]
    pub container_id: i64,

    #[serde(rename = "artistName", default)]
    pub artist_name: String,

    #[serde(rename = "containerInfo", default)]
    pub container_info: String,

    #[serde(rename = "venueName", default)]
    pub venue_name: String,

    #[serde(rename = "venueCity", default)]
    pub venue_city: String,

    #[serde(rename = "venueState", default)]
    pub venue_state: String,

    #[serde(rename = "performanceDate", default)]
    pub performance_date: String,

    #[serde(rename = "performanceDateFormatted", default)]
    pub performance_date_formatted: String,

    #[serde(rename = "performanceDateYear", default)]
    pub performance_date_year: String,

    #[serde(rename = "artistID", default)]
    pub artist_id: i64,

    #[serde(default, skip)]
    pub image_url: String,

    #[serde(rename = "songList", default)]
    pub song_list: String,

    /// Which service this show was fetched from (nugs.net or LivePhish).
    #[serde(default)]
    pub service: Service,
}

impl CatalogShow {
    /// Parse from API response dict with defensive null handling.
    pub fn from_json(data: &serde_json::Value) -> Self {
        let image_url = deserialize_image_url(data);
        let song_list = data
            .get("songList")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let coerce_str = |key: &str| -> String {
            data.get(key)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };

        let coerce_i64 = |key: &str| -> i64 {
            data.get(key)
                .and_then(|v| {
                    v.as_i64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                })
                .unwrap_or(0)
        };

        CatalogShow {
            container_id: coerce_i64("containerID"),
            artist_name: coerce_str("artistName"),
            container_info: coerce_str("containerInfo").trim().to_string(),
            venue_name: coerce_str("venueName"),
            venue_city: coerce_str("venueCity"),
            venue_state: coerce_str("venueState"),
            performance_date: coerce_str("performanceDate"),
            performance_date_formatted: coerce_str("performanceDateFormatted"),
            performance_date_year: coerce_str("performanceDateYear"),
            artist_id: coerce_i64("artistID"),
            image_url,
            song_list,
            service: Service::default(),
        }
    }

    pub fn from_show(show: &Show) -> Self {
        CatalogShow {
            container_id: show.container_id,
            artist_name: show.artist_name.clone(),
            container_info: show.container_info.clone(),
            venue_name: show.venue_name.clone(),
            venue_city: show.venue_city.clone(),
            venue_state: show.venue_state.clone(),
            performance_date: show.performance_date.clone(),
            performance_date_formatted: show.performance_date_formatted.clone(),
            performance_date_year: show.performance_date_year.clone(),
            artist_id: show.artist_id,
            image_url: show.image_url.clone(),
            song_list: String::new(),
            service: Service::default(),
        }
    }
}

impl DisplayLocation for CatalogShow {
    fn venue_name(&self) -> &str {
        &self.venue_name
    }
    fn venue_city(&self) -> &str {
        &self.venue_city
    }
    fn venue_state(&self) -> &str {
        &self.venue_state
    }
    fn performance_date(&self) -> &str {
        &self.performance_date
    }
    fn performance_date_formatted(&self) -> &str {
        &self.performance_date_formatted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_track_json() -> serde_json::Value {
        json!({
            "trackID": 12345,
            "songID": 678,
            "songTitle": "Tweezer",
            "trackNum": 1,
            "discNum": 1,
            "setNum": 1,
            "totalRunningTime": 960,
            "hhmmssTotalRunningTime": "16:00"
        })
    }

    fn sample_show_json() -> serde_json::Value {
        json!({
            "containerID": 99999,
            "artistName": "Phish",
            "artistID": 196,
            "containerInfo": "2024-08-31 Dick's Sporting Goods Park",
            "venueName": "Dick's Sporting Goods Park",
            "venueCity": "Commerce City",
            "venueState": "CO",
            "performanceDate": "2024-08-31",
            "performanceDateFormatted": "08/31/2024",
            "performanceDateYear": "2024",
            "totalContainerRunningTime": 11565,
            "hhmmssTotalRunningTime": "3:12:45",
            "tracks": [],
            "img": {"url": "https://example.com/img.jpg"}
        })
    }

    fn sample_catalog_show_json() -> serde_json::Value {
        json!({
            "containerID": 99999,
            "artistName": "Phish",
            "artistID": 196,
            "containerInfo": "2024-08-31 Dick's Sporting Goods Park",
            "venueName": "Dick's Sporting Goods Park",
            "venueCity": "Commerce City",
            "venueState": "CO",
            "performanceDate": "2024-08-31",
            "performanceDateFormatted": "08/31/2024",
            "performanceDateYear": "2024",
            "img": {"url": "https://example.com/img.jpg"},
            "songList": "Tweezer, Sand, Piper"
        })
    }

    // Track tests
    #[test]
    fn test_track_from_dict_with_all_fields() {
        let track: Track = serde_json::from_value(sample_track_json()).unwrap();
        assert_eq!(track.track_id, 12345);
        assert_eq!(track.song_id, 678);
        assert_eq!(track.song_title, "Tweezer");
        assert_eq!(track.track_num, 1);
        assert_eq!(track.disc_num, 1);
        assert_eq!(track.set_num, 1);
        assert_eq!(track.duration_seconds, 960);
        assert_eq!(track.duration_display, "16:00");
    }

    #[test]
    fn test_track_from_dict_with_minimal_fields() {
        let track: Track = serde_json::from_value(json!({"trackID": 99999})).unwrap();
        assert_eq!(track.track_id, 99999);
        assert_eq!(track.song_id, 0);
        assert_eq!(track.song_title, "");
        assert_eq!(track.track_num, 0);
        assert_eq!(track.disc_num, 1);
        assert_eq!(track.set_num, 0);
        assert_eq!(track.duration_seconds, 0);
        assert_eq!(track.duration_display, "");
    }

    // Show tests
    #[test]
    fn test_show_from_dict_basic() {
        let show = Show::from_json(&sample_show_json());
        assert_eq!(show.container_id, 99999);
        assert_eq!(show.artist_name, "Phish");
        assert_eq!(show.artist_id, 196);
        assert_eq!(show.container_info, "2024-08-31 Dick's Sporting Goods Park");
        assert_eq!(show.venue_name, "Dick's Sporting Goods Park");
        assert_eq!(show.venue_city, "Commerce City");
        assert_eq!(show.venue_state, "CO");
        assert_eq!(show.performance_date, "2024-08-31");
        assert_eq!(show.performance_date_formatted, "08/31/2024");
        assert_eq!(show.performance_date_year, "2024");
        assert_eq!(show.total_duration_seconds, 11565);
        assert_eq!(show.total_duration_display, "3:12:45");
        assert_eq!(show.image_url, "https://example.com/img.jpg");
        assert!(show.tracks.is_empty());
    }

    #[test]
    fn test_show_from_dict_with_tracks() {
        let mut data = sample_show_json();
        data["tracks"] = json!([sample_track_json()]);
        let show = Show::from_json(&data);
        assert_eq!(show.tracks.len(), 1);
        assert_eq!(show.tracks[0].track_id, 12345);
        assert_eq!(show.tracks[0].song_title, "Tweezer");
    }

    #[test]
    fn test_show_artist_id_default() {
        let show = Show::from_json(&json!({
            "containerID": 1,
            "artistName": "Test",
            "containerInfo": "Info",
            "venueName": "Venue",
            "venueCity": "City",
            "venueState": "ST",
            "performanceDate": "2024-01-01",
            "performanceDateFormatted": "01/01/2024",
            "performanceDateYear": "2024"
        }));
        assert_eq!(show.artist_id, 0);
    }

    #[test]
    fn test_show_folder_name_sanitization() {
        let show = Show::from_json(&json!({
            "containerID": 1,
            "artistName": "Test:Artist",
            "containerInfo": "Show/With*Unsafe?Chars\"<>|",
            "venueName": "Venue",
            "venueCity": "City",
            "venueState": "ST",
            "performanceDate": "2024-01-01",
            "performanceDateFormatted": "01/01/2024",
            "performanceDateYear": "2024"
        }));
        let folder = show.folder_name();
        for ch in ['\\', '/', ':', '*', '?', '"', '<', '>', '|'] {
            assert!(!folder.contains(ch), "folder name contains '{ch}'");
        }
    }

    #[test]
    fn test_show_folder_name_truncation() {
        let long_info = "A".repeat(200);
        let show = Show::from_json(&json!({
            "containerID": 1,
            "artistName": "Artist",
            "containerInfo": long_info,
            "venueName": "Venue",
            "venueCity": "City",
            "venueState": "ST",
            "performanceDate": "2024-01-01",
            "performanceDateFormatted": "01/01/2024",
            "performanceDateYear": "2024"
        }));
        assert!(show.folder_name().len() <= 120);
    }

    #[test]
    fn test_show_display_location() {
        let show = Show::from_json(&sample_show_json());
        assert_eq!(
            show.display_location(),
            "Dick's Sporting Goods Park, Commerce City, CO"
        );
    }

    #[test]
    fn test_show_display_location_partial() {
        let show = Show::from_json(&json!({
            "containerID": 1,
            "artistName": "Artist",
            "containerInfo": "Info",
            "venueName": "Venue Only",
            "venueCity": "",
            "venueState": "",
            "performanceDate": "2024-01-01",
            "performanceDateFormatted": "01/01/2024",
            "performanceDateYear": "2024"
        }));
        assert_eq!(show.display_location(), "Venue Only");
    }

    #[test]
    fn test_show_display_location_short() {
        let show = Show::from_json(&sample_show_json());
        assert_eq!(show.display_location_short(), "Commerce City, CO");
    }

    #[test]
    fn test_show_display_location_short_city_only() {
        let show = Show::from_json(&json!({
            "containerID": 1,
            "artistName": "Artist",
            "containerInfo": "Info",
            "venueName": "Venue",
            "venueCity": "Denver",
            "venueState": "",
            "performanceDate": "2024-01-01",
            "performanceDateFormatted": "01/01/2024",
            "performanceDateYear": "2024"
        }));
        assert_eq!(show.display_location_short(), "Denver");
    }

    #[test]
    fn test_show_display_location_short_empty() {
        let show = Show::from_json(&json!({
            "containerID": 1,
            "artistName": "Artist",
            "containerInfo": "Info",
            "venueName": "Venue",
            "venueCity": "",
            "venueState": "",
            "performanceDate": "2024-01-01",
            "performanceDateFormatted": "01/01/2024",
            "performanceDateYear": "2024"
        }));
        assert_eq!(show.display_location_short(), "");
    }

    #[test]
    fn test_show_sets_grouped() {
        let data = json!({
            "containerID": 1,
            "artistName": "Artist",
            "containerInfo": "Info",
            "venueName": "Venue",
            "venueCity": "City",
            "venueState": "ST",
            "performanceDate": "2024-01-01",
            "performanceDateFormatted": "01/01/2024",
            "performanceDateYear": "2024",
            "tracks": [
                {"trackID": 1, "setNum": 1, "songTitle": "A"},
                {"trackID": 2, "setNum": 1, "songTitle": "B"},
                {"trackID": 3, "setNum": 2, "songTitle": "C"}
            ]
        });
        let show = Show::from_json(&data);
        let sets = show.sets_grouped();
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[&1].len(), 2);
        assert_eq!(sets[&2].len(), 1);
    }

    // CatalogShow tests
    #[test]
    fn test_catalog_show_from_json() {
        let cs = CatalogShow::from_json(&sample_catalog_show_json());
        assert_eq!(cs.container_id, 99999);
        assert_eq!(cs.artist_name, "Phish");
        assert_eq!(cs.artist_id, 196);
        assert_eq!(cs.venue_name, "Dick's Sporting Goods Park");
        assert_eq!(cs.song_list, "Tweezer, Sand, Piper");
        assert_eq!(cs.image_url, "https://example.com/img.jpg");
    }

    #[test]
    fn test_catalog_show_from_show() {
        let show = Show::from_json(&sample_show_json());
        let cs = CatalogShow::from_show(&show);
        assert_eq!(cs.container_id, show.container_id);
        assert_eq!(cs.artist_name, show.artist_name);
        assert_eq!(cs.artist_id, show.artist_id);
        assert_eq!(cs.venue_name, show.venue_name);
        assert_eq!(cs.image_url, show.image_url);
    }

    #[test]
    fn test_catalog_show_display_location_short() {
        let cs = CatalogShow::from_json(&sample_catalog_show_json());
        assert_eq!(cs.display_location_short(), "Commerce City, CO");
    }

    // None coercion tests
    #[test]
    fn test_catalog_show_null_fields() {
        let cs = CatalogShow::from_json(&json!({
            "containerID": 123,
            "artistName": null,
            "containerInfo": null,
            "venueName": null,
            "venueCity": null,
            "venueState": null,
            "performanceDate": null,
            "performanceDateFormatted": null,
            "performanceDateYear": null,
            "artistID": null,
            "img": {"url": null},
            "songList": null
        }));
        assert_eq!(cs.artist_name, "");
        assert_eq!(cs.container_info, "");
        assert_eq!(cs.venue_name, "");
        assert_eq!(cs.venue_city, "");
        assert_eq!(cs.venue_state, "");
        assert_eq!(cs.performance_date, "");
        assert_eq!(cs.performance_date_formatted, "");
        assert_eq!(cs.performance_date_year, "");
        assert_eq!(cs.artist_id, 0);
        assert_eq!(cs.image_url, "");
        assert_eq!(cs.song_list, "");
    }

    #[test]
    fn test_show_null_fields() {
        let show = Show::from_json(&json!({
            "containerID": null,
            "artistName": null,
            "containerInfo": null,
            "venueName": null,
            "venueCity": null,
            "venueState": null,
            "performanceDate": null,
            "performanceDateFormatted": null,
            "performanceDateYear": null,
            "artistID": null,
            "totalContainerRunningTime": null,
            "hhmmssTotalRunningTime": null
        }));
        assert_eq!(show.container_id, 0);
        assert_eq!(show.artist_name, "");
        assert_eq!(show.venue_name, "");
        assert_eq!(show.performance_date, "");
        assert_eq!(show.artist_id, 0);
        assert_eq!(show.total_duration_seconds, 0);
        assert_eq!(show.total_duration_display, "");
    }

    #[test]
    fn test_show_display_date_empty() {
        let show = Show::from_json(&json!({
            "containerID": 1,
            "artistName": "",
            "containerInfo": "",
            "venueName": "",
            "venueCity": "",
            "venueState": "",
            "performanceDate": "",
            "performanceDateFormatted": "",
            "performanceDateYear": ""
        }));
        assert_eq!(show.display_date(), "");
    }

    #[test]
    fn test_show_display_location_empty() {
        let show = Show::from_json(&json!({
            "containerID": 1,
            "artistName": "",
            "containerInfo": "",
            "venueName": "",
            "venueCity": "",
            "venueState": "",
            "performanceDate": "2024-01-01",
            "performanceDateFormatted": "",
            "performanceDateYear": "2024"
        }));
        assert_eq!(show.display_location(), "");
    }

    // Malformed track tests
    #[test]
    fn test_malformed_track_skipped() {
        let data = json!({
            "containerID": 100,
            "artistName": "Test",
            "containerInfo": "Info",
            "venueName": "Venue",
            "venueCity": "City",
            "venueState": "ST",
            "performanceDate": "2024-01-01",
            "performanceDateFormatted": "01/01/2024",
            "performanceDateYear": "2024",
            "tracks": [
                {"trackID": 1, "songTitle": "Good Track", "trackNum": 1, "setNum": 1},
                "not a dict",
                null,
                {"trackID": 2, "songTitle": "Also Good", "trackNum": 2, "setNum": 1}
            ]
        });
        let show = Show::from_json(&data);
        assert_eq!(show.tracks.len(), 2);
        assert_eq!(show.tracks[0].song_title, "Good Track");
        assert_eq!(show.tracks[1].song_title, "Also Good");
    }

    #[test]
    fn test_all_malformed_tracks() {
        let data = json!({
            "containerID": 100,
            "artistName": "Test",
            "containerInfo": "Info",
            "venueName": "Venue",
            "venueCity": "City",
            "venueState": "ST",
            "performanceDate": "2024-01-01",
            "performanceDateFormatted": "01/01/2024",
            "performanceDateYear": "2024",
            "tracks": ["bad", null, 42]
        });
        let show = Show::from_json(&data);
        assert_eq!(show.tracks.len(), 0);
    }

    // Track defensive coercion tests
    #[test]
    fn test_track_missing_track_id() {
        // Track without trackID gets default 0
        let track: Track =
            serde_json::from_value(json!({"trackID": null, "songTitle": "Test"})).unwrap();
        assert_eq!(track.track_id, 0);
        assert_eq!(track.song_title, "Test");
    }

    #[test]
    fn test_track_none_numeric_fields() {
        let track: Track = serde_json::from_value(json!({
            "trackID": null,
            "songID": null,
            "trackNum": null,
            "discNum": null,
            "setNum": null,
            "totalRunningTime": null
        }))
        .unwrap();
        assert_eq!(track.track_id, 0);
        assert_eq!(track.song_id, 0);
        assert_eq!(track.track_num, 0);
        assert_eq!(track.disc_num, 1); // default is 1
        assert_eq!(track.set_num, 0);
        assert_eq!(track.duration_seconds, 0);
    }

    #[test]
    fn test_track_string_numeric_fields() {
        let track: Track = serde_json::from_value(json!({
            "trackID": "42",
            "songID": "100",
            "trackNum": "3"
        }))
        .unwrap();
        assert_eq!(track.track_id, 42);
        assert_eq!(track.song_id, 100);
        assert_eq!(track.track_num, 3);
    }

    #[test]
    fn test_track_garbage_numeric_fields() {
        let track: Track = serde_json::from_value(json!({
            "trackID": "abc",
            "songID": "xyz",
            "trackNum": []
        }))
        .unwrap();
        assert_eq!(track.track_id, 0);
        assert_eq!(track.song_id, 0);
        assert_eq!(track.track_num, 0);
    }

    #[test]
    fn test_track_none_string_fields() {
        let track: Track = serde_json::from_value(json!({
            "trackID": 1,
            "songTitle": null,
            "hhmmssTotalRunningTime": null
        }))
        .unwrap();
        assert_eq!(track.song_title, "");
        assert_eq!(track.duration_display, "");
    }

    #[test]
    fn test_track_empty_dict() {
        let track: Track = serde_json::from_value(json!({
            "trackID": null
        }))
        .unwrap();
        assert_eq!(track.track_id, 0);
        assert_eq!(track.song_title, "");
        assert_eq!(track.track_num, 0);
    }
}
