use std::time::{SystemTime, UNIX_EPOCH};

use crate::models::{Playlist, StreamParams};
use crate::service::{Service, ServiceAuth};

use super::client::NugsApi;
use super::error::ApiError;

impl NugsApi {
    /// Get streaming URL for a track.
    ///
    /// Returns the stream URL string (may be empty if format unavailable).
    pub async fn get_stream_url(
        &mut self,
        track_id: i64,
        format_code: i64,
        stream_params: &StreamParams,
    ) -> Result<String, ApiError> {
        let api_base = self.api_base.clone();
        let sub_id = stream_params.subscription_id.clone();
        let plan_id = stream_params.sub_costplan_id_access_list.clone();
        let user_id = stream_params.user_id.clone();
        let start_stamp = stream_params.start_stamp.clone();
        let end_stamp = stream_params.end_stamp.clone();

        // Stream URL endpoint is separate from catalog — skip rate limiting
        // to match Go upstream behavior and avoid unnecessary latency.
        let stream_ua = self.service.config().stream_user_agent.to_string();

        let response = self
            .request(
                |client| {
                    client
                        .get(format!("{}bigriver/subPlayer.aspx", api_base))
                        .query(&[
                            ("trackID", track_id.to_string()),
                            ("platformID", format_code.to_string()),
                            ("app", "1".to_string()),
                            ("subscriptionID", sub_id.clone()),
                            ("subCostplanIDAccessList", plan_id.clone()),
                            ("nn_userID", user_id.clone()),
                            ("startDateStamp", start_stamp.clone()),
                            ("endDateStamp", end_stamp.clone()),
                        ])
                        .header("User-Agent", stream_ua.as_str())
                },
                false,
                true,
            )
            .await?;

        let body = response.text().await.map_err(|e| {
            ApiError::UnexpectedResponse(format!("Failed to read response body: {}", e))
        })?;
        let data = NugsApi::parse_json_value(&body, "stream URL")?;

        Ok(data
            .get("streamLink")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
    }

    /// Get streaming URL for a LivePhish track using MD5-signed requests.
    ///
    /// Retries with epoch compensation values [3, 5, 7, 10] to handle
    /// server clock skew. Returns `Err(StreamUrlFailed)` if all fail.
    pub async fn get_stream_url_livephish(
        &mut self,
        track_id: i64,
        format_code: i64,
        stream_params: &StreamParams,
    ) -> Result<String, ApiError> {
        let sig_key = match &self.service.config().auth {
            ServiceAuth::LivePhish { sig_key } => *sig_key,
            _ => unreachable!(),
        };

        let api_base = self.api_base.clone();
        let sub_id = stream_params.subscription_id.clone();
        let plan_id = stream_params.sub_costplan_id_access_list.clone();
        let user_id = stream_params.user_id.clone();
        let start_stamp = stream_params.start_stamp.clone();
        let end_stamp = stream_params.end_stamp.clone();
        let stream_ua = self.service.config().stream_user_agent.to_string();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        for compensation in [3i64, 5, 7, 10] {
            let epoch = now + compensation;
            let signature = format!("{:x}", md5::compute(format!("{}{}", sig_key, epoch)));
            let epoch_str = epoch.to_string();
            let sig_clone = signature.clone();

            let sub_id_c = sub_id.clone();
            let plan_id_c = plan_id.clone();
            let user_id_c = user_id.clone();
            let start_stamp_c = start_stamp.clone();
            let end_stamp_c = end_stamp.clone();
            let stream_ua_c = stream_ua.clone();
            let api_base_c = api_base.clone();

            let response = self
                .request(
                    |client| {
                        client
                            .get(format!("{}bigriver/subPlayer.aspx", api_base_c))
                            .query(&[
                                ("trackID", track_id.to_string()),
                                ("app", "1".to_string()),
                                ("platformID", format_code.to_string()),
                                ("subscriptionID", sub_id_c.clone()),
                                ("subCostplanIDAccessList", plan_id_c.clone()),
                                ("nn_userID", user_id_c.clone()),
                                ("startDateStamp", start_stamp_c.clone()),
                                ("endDateStamp", end_stamp_c.clone()),
                                ("tk", sig_clone.clone()),
                                ("lxp", epoch_str.clone()),
                            ])
                            .header("User-Agent", stream_ua_c.as_str())
                    },
                    false,
                    true,
                )
                .await?;

            let body = response.text().await.map_err(|e| {
                ApiError::UnexpectedResponse(format!("Failed to read response body: {}", e))
            })?;
            let data = NugsApi::parse_json_value(&body, "stream URL")?;

            let link = data
                .get("streamLink")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if !link.is_empty() {
                return Ok(link);
            }
        }

        Err(ApiError::StreamUrlFailed(track_id))
    }

    /// Dispatch stream URL resolution based on service.
    pub async fn get_stream_url_for_service(
        &mut self,
        track_id: i64,
        format_code: i64,
        stream_params: &StreamParams,
    ) -> Result<String, ApiError> {
        match self.service {
            Service::Nugs => {
                self.get_stream_url(track_id, format_code, stream_params)
                    .await
            }
            Service::LivePhish => {
                self.get_stream_url_livephish(track_id, format_code, stream_params)
                    .await
            }
            Service::Bman => unreachable!("Bman uses direct Google Drive URLs, not stream API"),
        }
    }

    /// Resolve numeric playlist URL ID to plGUID via redirect.
    pub async fn resolve_playlist_id(&mut self, numeric_id: &str) -> Result<String, ApiError> {
        let player_url = self.player_url.clone();
        let url = format!("{}#/playlists/playlist/{}", player_url, numeric_id);

        let response = self.request(|client| client.get(&url), false, true).await?;

        let final_url = response.url().to_string();
        let parsed =
            url::Url::parse(&final_url).map_err(|e| ApiError::UnexpectedResponse(e.to_string()))?;

        let pl_guid = parsed
            .query_pairs()
            .find(|(k, _)| k == "plGUID")
            .map(|(_, v)| v.to_string())
            .unwrap_or_default();

        if pl_guid.is_empty() {
            return Err(ApiError::UnexpectedResponse(
                "Could not resolve playlist ID".to_string(),
            ));
        }

        Ok(pl_guid)
    }

    /// Fetch playlist metadata and tracks.
    ///
    /// `is_catalog=true` for catalog playlists (plGUID), `false` for user playlists (numeric ID).
    pub async fn get_playlist(
        &mut self,
        playlist_id: &str,
        is_catalog: bool,
    ) -> Result<Playlist, ApiError> {
        let api_base = self.api_base.clone();
        let playlist_id_owned = playlist_id.to_string();

        let response = if is_catalog {
            self.request(
                |client| {
                    client.get(format!("{}api.aspx", api_base)).query(&[
                        ("method", "catalog.playlist"),
                        ("plGUID", playlist_id_owned.as_str()),
                    ])
                },
                true,
                true,
            )
            .await?
        } else {
            let email = self.email.clone().unwrap_or_default();
            let legacy_token = self.legacy_token.clone().unwrap_or_default();
            let developer_key = self.service.config().developer_key.to_string();
            let stream_ua = self.service.config().stream_user_agent.to_string();

            self.request(
                |client| {
                    client
                        .get(format!("{}secureApi.aspx", api_base))
                        .query(&[
                            ("method", "user.playlist"),
                            ("playlistID", playlist_id_owned.as_str()),
                            ("developerKey", developer_key.as_str()),
                            ("user", email.as_str()),
                            ("token", legacy_token.as_str()),
                        ])
                        .header("User-Agent", stream_ua.as_str())
                },
                true,
                true,
            )
            .await?
        };

        if response.status().as_u16() != 200 {
            return Err(ApiError::UnexpectedResponse(format!(
                "Failed to fetch playlist: HTTP {}",
                response.status().as_u16()
            )));
        }

        let body = response.text().await.map_err(|e| {
            ApiError::UnexpectedResponse(format!("Failed to read response body: {}", e))
        })?;
        let data = NugsApi::parse_json_value(&body, "playlist")?;

        Ok(Playlist::from_json(&data))
    }
}
