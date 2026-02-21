use serde::{Deserialize, Serialize};

use super::sanitize::sanitize_filename;
use super::show::Track;

/// A single item in a playlist (track + parent container metadata).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistItem {
    pub track: Track,
    pub container_id: i64,
    pub container_info: String,
    pub artist_name: String,
    pub venue_name: String,
    pub performance_date: String,
}

impl PlaylistItem {
    pub fn from_json(data: &serde_json::Value) -> Option<Self> {
        let pc = data.get("playlistContainer").unwrap_or(data);
        let track_data = data.get("track").unwrap_or(data);
        let track: Track = serde_json::from_value(track_data.clone()).ok()?;

        Some(PlaylistItem {
            track,
            container_id: pc.get("containerID").and_then(|v| v.as_i64()).unwrap_or(0),
            container_info: pc
                .get("containerInfo")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            artist_name: pc
                .get("artistName")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            venue_name: pc
                .get("venueName")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            performance_date: pc
                .get("performanceDate")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
    }
}

/// A playlist with tracks from potentially different shows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub playlist_name: String,
    pub num_tracks: i64,
    pub items: Vec<PlaylistItem>,
}

impl Playlist {
    pub fn from_json(data: &serde_json::Value) -> Self {
        let resp = data.get("Response").unwrap_or(data);

        let items: Vec<PlaylistItem> = resp
            .get("items")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(PlaylistItem::from_json).collect())
            .unwrap_or_default();

        Playlist {
            playlist_name: resp
                .get("playListName")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            num_tracks: resp.get("numTracks").and_then(|v| v.as_i64()).unwrap_or(0),
            items,
        }
    }

    pub fn folder_name(&self) -> String {
        sanitize_filename(&self.playlist_name, 120)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_playlist_from_json() {
        let data = json!({
            "Response": {
                "playListName": "My Favorite Jams",
                "numTracks": 2,
                "items": [
                    {
                        "track": {
                            "trackID": 100,
                            "songTitle": "Tweezer",
                            "trackNum": 5,
                            "discNum": 1,
                            "setNum": 2
                        },
                        "playlistContainer": {
                            "containerID": 999,
                            "containerInfo": "2024-08-31 Dicks",
                            "artistName": "Phish",
                            "venueName": "Dick's",
                            "performanceDate": "2024-08-31"
                        }
                    },
                    {
                        "track": {
                            "trackID": 200,
                            "songTitle": "Ghost",
                            "trackNum": 3,
                            "discNum": 1,
                            "setNum": 1
                        },
                        "playlistContainer": {
                            "containerID": 888,
                            "containerInfo": "2024-12-31 MSG",
                            "artistName": "Phish",
                            "venueName": "MSG",
                            "performanceDate": "2024-12-31"
                        }
                    }
                ]
            }
        });

        let playlist = Playlist::from_json(&data);
        assert_eq!(playlist.playlist_name, "My Favorite Jams");
        assert_eq!(playlist.num_tracks, 2);
        assert_eq!(playlist.items.len(), 2);
        assert_eq!(playlist.items[0].track.song_title, "Tweezer");
        assert_eq!(playlist.items[0].container_id, 999);
        assert_eq!(playlist.items[0].artist_name, "Phish");
        assert_eq!(playlist.items[1].track.song_title, "Ghost");
        assert_eq!(playlist.items[1].container_id, 888);
    }

    #[test]
    fn test_playlist_folder_name() {
        let data = json!({
            "playListName": "Best Jams 2024!",
            "numTracks": 0,
            "items": []
        });
        let playlist = Playlist::from_json(&data);
        assert_eq!(playlist.folder_name(), "Best Jams 2024!");
    }

    #[test]
    fn test_playlist_folder_name_sanitized() {
        let data = json!({
            "playListName": "My/Playlist:With*Bad?Chars",
            "numTracks": 0,
            "items": []
        });
        let playlist = Playlist::from_json(&data);
        for ch in ['\\', '/', ':', '*', '?', '"', '<', '>', '|'] {
            assert!(
                !playlist.folder_name().contains(ch),
                "folder name contains '{ch}'"
            );
        }
    }
}
