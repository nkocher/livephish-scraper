use std::collections::HashMap;

use crate::models::Show;

use super::client::NugsApi;
use super::error::ApiError;

impl NugsApi {
    /// Paginate through containersAll for a specific artist.
    ///
    /// Returns raw container JSON values (parsed into CatalogShow by the catalog layer).
    pub async fn get_artist_catalog(
        &mut self,
        artist_id: i64,
        limit: i64,
    ) -> Result<Vec<serde_json::Value>, ApiError> {
        let mut all_containers = Vec::new();
        let mut offset: i64 = 1;
        let api_base = self.api_base.clone();

        loop {
            let current_offset = offset;
            let response = self
                .request(
                    |client| {
                        client.get(format!("{}api.aspx", api_base)).query(&[
                            ("method", "catalog.containersAll"),
                            ("artistList", &artist_id.to_string()),
                            ("availType", "1"),
                            ("limit", &limit.to_string()),
                            ("startOffset", &current_offset.to_string()),
                            ("vdisp", "1"),
                        ])
                    },
                    true,
                    true,
                )
                .await?;

            let body = response.text().await.map_err(|e| {
                ApiError::UnexpectedResponse(format!("Failed to read response body: {}", e))
            })?;
            let data = NugsApi::parse_json_value(&body, "artist catalog")?;

            let containers = data
                .get("Response")
                .and_then(|r| r.get("containers"))
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();

            if containers.is_empty() {
                break;
            }

            offset += containers.len() as i64;
            all_containers.extend(containers);
        }

        Ok(all_containers)
    }

    /// Paginate through containersAll for LivePhish (single-artist service, no artistList param).
    pub async fn get_all_catalog(
        &mut self,
        limit: i64,
    ) -> Result<Vec<serde_json::Value>, ApiError> {
        let mut all_containers = Vec::new();
        let mut offset: i64 = 1;
        let api_base = self.api_base.clone();

        loop {
            let current_offset = offset;
            let response = self
                .request(
                    |client| {
                        client.get(format!("{}api.aspx", api_base)).query(&[
                            ("method", "catalog.containersAll"),
                            ("availType", "1"),
                            ("limit", &limit.to_string()),
                            ("startOffset", &current_offset.to_string()),
                            ("vdisp", "1"),
                        ])
                    },
                    true,
                    true,
                )
                .await?;

            let body = response.text().await.map_err(|e| {
                ApiError::UnexpectedResponse(format!("Failed to read response body: {}", e))
            })?;
            let data = NugsApi::parse_json_value(&body, "livephish catalog")?;

            let containers = data
                .get("Response")
                .and_then(|r| r.get("containers"))
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();

            if containers.is_empty() {
                break;
            }

            offset += containers.len() as i64;
            all_containers.extend(containers);
        }

        Ok(all_containers)
    }

    /// Discover all available artists via catalog.artists endpoint.
    ///
    /// Returns {artist_id: artist_name}. Returns empty map on any failure.
    pub async fn get_all_artists(&mut self) -> HashMap<i64, String> {
        let api_base = self.api_base.clone();

        let result: Result<HashMap<i64, String>, ApiError> = async {
            let response = self
                .request(
                    |client| {
                        client
                            .get(format!("{}api.aspx", api_base))
                            .query(&[("method", "catalog.artists"), ("availType", "1")])
                    },
                    true,
                    true,
                )
                .await?;

            let body = response.text().await.map_err(|e| {
                ApiError::UnexpectedResponse(format!("Failed to read response body: {}", e))
            })?;
            let data = NugsApi::parse_json_value(&body, "artist discovery")?;

            let artists_list = data
                .get("Response")
                .and_then(|r| r.get("artists"))
                .and_then(|a| a.as_array())
                .cloned()
                .unwrap_or_default();

            let mut artists = HashMap::new();
            for a in &artists_list {
                // Coerce artistID from int or string
                let aid = a
                    .get("artistID")
                    .and_then(|v| {
                        v.as_i64()
                            .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
                    })
                    .unwrap_or(0);

                let name = a
                    .get("artistName")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();

                if aid > 0 && !name.is_empty() {
                    artists.insert(aid, name);
                }
            }

            Ok(artists)
        }
        .await;

        result.unwrap_or_default()
    }

    /// Get detailed show information including tracks.
    pub async fn get_show_detail(&mut self, container_id: i64) -> Result<Show, ApiError> {
        let api_base = self.api_base.clone();

        let response = self
            .request(
                |client| {
                    client.get(format!("{}api.aspx", api_base)).query(&[
                        ("method", "catalog.container"),
                        ("containerID", &container_id.to_string()),
                        ("vdisp", "1"),
                    ])
                },
                true,
                true,
            )
            .await?;

        if response.status().as_u16() != 200 {
            return Err(ApiError::UnexpectedResponse(format!(
                "Failed to fetch show {}: HTTP {}",
                container_id,
                response.status().as_u16()
            )));
        }

        let body = response.text().await.map_err(|e| {
            ApiError::UnexpectedResponse(format!("Failed to read response body: {}", e))
        })?;
        let data = NugsApi::parse_json_value(&body, &format!("show {}", container_id))?;

        let response_data = data.get("Response").ok_or_else(|| {
            ApiError::UnexpectedResponse(format!(
                "Unexpected API response for show {}",
                container_id
            ))
        })?;

        Ok(Show::from_json(response_data))
    }
}
