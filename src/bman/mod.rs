pub mod artwork;
pub mod download;
pub mod gdrive;
pub mod id_map;
pub mod metadata;
pub mod parser;
pub mod setlistfm;
#[cfg(feature = "bman")]
pub mod cover;
#[cfg(test)]
mod integration_tests;

use std::time::Duration;

use thiserror::Error;
use tokio::time::sleep;
use tracing::warn;

use gdrive::{DriveErrorResponse, DriveItem, DriveListResponse};
use id_map::BmanIdMap;

/// Root folder ID for Bman's Google Drive archive.
pub const BMAN_ROOT_FOLDER_ID: &str = "1aK32Uxa56LK2DsQ4ZmgugA9FhoMrlZm7";

const PAGE_SIZE: u32 = 1000;
const PAGE_DELAY_MS: u64 = 100;
const MAX_RETRIES: u32 = 3;
const BACKOFF_BASE_SECS: [u64; 3] = [5, 10, 15];

/// Errors that can occur when calling the Google Drive API.
#[derive(Debug, Error)]
pub enum BmanError {
    #[error("Drive API error: {0}")]
    DriveApi(String),

    #[error("Rate limit exceeded: {0}")]
    RateLimit(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),
}

/// Google Drive API client for Bman's archive.
pub struct BmanApi {
    pub(crate) client: reqwest::Client,
    pub(crate) google_api_key: String,
    pub(crate) root_folder_id: String,
    pub id_map: BmanIdMap,
    /// Date-keyed index of artwork catalog images.
    pub artwork_index: artwork::ArtworkIndex,
    /// Base URL for Drive API (overrideable in tests).
    drive_base_url: String,
}

impl BmanApi {
    pub fn new(google_api_key: String) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
            google_api_key,
            root_folder_id: BMAN_ROOT_FOLDER_ID.to_string(),
            id_map: BmanIdMap::new(),
            artwork_index: artwork::ArtworkIndex::new(),
            drive_base_url: "https://www.googleapis.com/drive/v3".to_string(),
        }
    }

    #[cfg(test)]
    pub fn new_for_test(google_api_key: String, root_folder_id: String) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("Failed to create HTTP client"),
            google_api_key,
            root_folder_id,
            id_map: BmanIdMap::new(),
            artwork_index: artwork::ArtworkIndex::new(),
            drive_base_url: "https://www.googleapis.com/drive/v3".to_string(),
        }
    }

    /// Override the Drive API base URL (for wiremock tests).
    #[cfg(test)]
    pub fn with_drive_base_url(mut self, base_url: &str) -> Self {
        self.drive_base_url = base_url.to_string();
        self
    }

    /// Build a direct download URL for a Google Drive file.
    pub fn download_url(&self, file_id: &str) -> String {
        format!(
            "{}/files/{}?alt=media&key={}",
            self.drive_base_url, file_id, self.google_api_key
        )
    }

    /// List all items in a Google Drive folder.
    pub async fn list_folder(&self, folder_id: &str) -> Result<Vec<DriveItem>, BmanError> {
        self.list_folder_filtered(folder_id, "").await
    }

    /// List items in a folder, filtered by MIME type.
    /// Pass an empty `mime_type` to return all items.
    /// When filtering by folder MIME, also includes shortcuts (which may point to folders).
    pub async fn list_folder_filtered(
        &self,
        folder_id: &str,
        mime_type: &str,
    ) -> Result<Vec<DriveItem>, BmanError> {
        let extra_query = if mime_type.is_empty() {
            String::new()
        } else if mime_type == gdrive::FOLDER_MIME {
            // Also include shortcuts alongside folders
            format!(
                " and (mimeType='{}' or mimeType='{}')",
                gdrive::FOLDER_MIME,
                gdrive::SHORTCUT_MIME
            )
        } else {
            format!(" and mimeType='{mime_type}'")
        };

        let mut items = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let response = self
                .drive_list_page(folder_id, &extra_query, page_token.as_deref())
                .await?;
            items.extend(response.files);

            match response.next_page_token {
                Some(token) => {
                    page_token = Some(token);
                    sleep(Duration::from_millis(PAGE_DELAY_MS)).await;
                }
                None => break,
            }
        }

        Ok(items)
    }

    /// Fetch a single page from the Drive Files.list API with retry logic.
    ///
    /// Retries on transport errors, 5xx, and rate-limit responses (429 / 403
    /// rateLimitExceeded / userRateLimitExceeded) using jittered exponential
    /// backoff (5s, 10s, 15s base).  Fails fast on 403 forbidden and 404.
    pub async fn drive_list_page(
        &self,
        folder_id: &str,
        extra_query: &str,
        page_token: Option<&str>,
    ) -> Result<DriveListResponse, BmanError> {
        let query = format!("'{folder_id}' in parents{extra_query}");
        let page_size_str = PAGE_SIZE.to_string();

        for attempt in 0..MAX_RETRIES {
            let last_attempt = attempt + 1 == MAX_RETRIES;

            let mut req = self
                .client
                .get(format!("{}/files", self.drive_base_url))
                .query(&[
                    ("q", query.as_str()),
                    ("key", self.google_api_key.as_str()),
                    ("fields", "files(id,name,mimeType,size,shortcutDetails),nextPageToken"),
                    ("pageSize", page_size_str.as_str()),
                ]);

            if let Some(token) = page_token {
                req = req.query(&[("pageToken", token)]);
            }

            let response = match req.send().await {
                Ok(r) => r,
                Err(e) if last_attempt => return Err(BmanError::DriveApi(e.to_string())),
                Err(_) => {
                    sleep(jittered_backoff(BACKOFF_BASE_SECS[attempt as usize])).await;
                    continue;
                }
            };

            let status = response.status();

            if status.is_success() {
                return response
                    .json::<DriveListResponse>()
                    .await
                    .map_err(|e| BmanError::DriveApi(e.to_string()));
            }

            let body = response.text().await.unwrap_or_default();
            let (reason, message) = parse_drive_error_reason(&body);
            let msg = message.unwrap_or_else(|| body.clone());

            match (status.as_u16(), reason.as_deref()) {
                // Rate limit: retry with backoff, or fail on last attempt
                (_, Some("rateLimitExceeded"))
                | (_, Some("userRateLimitExceeded"))
                | (429, _) => {
                    if last_attempt {
                        return Err(BmanError::RateLimit(msg));
                    }
                    warn!(
                        "Drive rate limited (attempt {}/{}), retrying",
                        attempt + 1,
                        MAX_RETRIES
                    );
                    sleep(jittered_backoff(BACKOFF_BASE_SECS[attempt as usize])).await;
                }
                // Non-retryable errors: fail immediately
                (_, Some("forbidden")) => return Err(BmanError::Forbidden(msg)),
                (_, Some("notFound")) | (404, _) => {
                    return Err(BmanError::NotFound(folder_id.to_string()))
                }
                // Server errors: retry with backoff, or fail on last attempt
                (s, _) if s >= 500 => {
                    if last_attempt {
                        return Err(BmanError::DriveApi(msg));
                    }
                    sleep(jittered_backoff(BACKOFF_BASE_SECS[attempt as usize])).await;
                }
                _ => return Err(BmanError::DriveApi(format!("HTTP {status}: {body}"))),
            }
        }

        Err(BmanError::DriveApi("Retries exhausted".to_string()))
    }
}

/// Extract the first `reason` and the top-level `message` from a Drive error body.
fn parse_drive_error_reason(body: &str) -> (Option<String>, Option<String>) {
    if let Ok(err) = serde_json::from_str::<DriveErrorResponse>(body) {
        let reason = err.error.errors.first().map(|e| e.reason.clone());
        (reason, Some(err.error.message))
    } else {
        (None, None)
    }
}

/// Jittered exponential backoff: base_secs + 0–2 s random offset.
fn jittered_backoff(base_secs: u64) -> Duration {
    let jitter_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_millis() % 2000)
        .unwrap_or(0);
    Duration::from_secs(base_secs) + Duration::from_millis(u64::from(jitter_ms))
}

// ---- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn drive_files_json(items: &[(&str, &str, &str)]) -> serde_json::Value {
        let files: Vec<_> = items
            .iter()
            .map(|(id, name, mime)| {
                serde_json::json!({
                    "id": id,
                    "name": name,
                    "mimeType": mime,
                })
            })
            .collect();
        serde_json::json!({ "files": files })
    }

    fn error_body(reason: &str, code: u16, message: &str) -> serde_json::Value {
        serde_json::json!({
            "error": {
                "errors": [{"domain": "global", "reason": reason, "message": message}],
                "code": code,
                "message": message
            }
        })
    }

    #[tokio::test]
    async fn test_list_folder_success() {
        let server = MockServer::start().await;
        let response = drive_files_json(&[
            ("id1", "1977", "application/vnd.google-apps.folder"),
            ("id2", "track.flac", "audio/flac"),
        ]);

        Mock::given(method("GET"))
            .and(path("/files"))
            .and(query_param("q", "'root_folder' in parents"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&server)
            .await;

        let api = BmanApi::new_for_test("key".into(), "root_folder".into())
            .with_drive_base_url(&server.uri());

        let items = api.list_folder("root_folder").await.unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "id1");
        assert!(items[0].is_folder());
        assert_eq!(items[1].id, "id2");
        assert!(items[1].is_flac());
    }

    #[tokio::test]
    async fn test_list_folder_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/files"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"files": []})),
            )
            .mount(&server)
            .await;

        let api = BmanApi::new_for_test("key".into(), "folder".into())
            .with_drive_base_url(&server.uri());

        let items = api.list_folder("folder").await.unwrap();
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn test_list_folder_pagination() {
        let server = MockServer::start().await;

        let page1 = serde_json::json!({
            "files": [{"id": "f1", "name": "a.flac", "mimeType": "audio/flac"}],
            "nextPageToken": "token123"
        });
        let page2 = serde_json::json!({
            "files": [{"id": "f2", "name": "b.flac", "mimeType": "audio/flac"}]
        });

        Mock::given(method("GET"))
            .and(path("/files"))
            .and(query_param("q", "'folder' in parents"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page1))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/files"))
            .and(query_param("pageToken", "token123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page2))
            .mount(&server)
            .await;

        let api = BmanApi::new_for_test("key".into(), "folder".into())
            .with_drive_base_url(&server.uri());

        let items = api.list_folder("folder").await.unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "f1");
        assert_eq!(items[1].id, "f2");
    }

    #[tokio::test]
    async fn test_list_folder_filtered_by_mime() {
        let server = MockServer::start().await;
        let response = drive_files_json(&[("id1", "track.flac", "audio/flac")]);

        Mock::given(method("GET"))
            .and(path("/files"))
            .and(query_param(
                "q",
                "'folder' in parents and mimeType='audio/flac'",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&server)
            .await;

        let api = BmanApi::new_for_test("key".into(), "folder".into())
            .with_drive_base_url(&server.uri());

        let items = api
            .list_folder_filtered("folder", "audio/flac")
            .await
            .unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].is_flac());
    }

    #[tokio::test]
    async fn test_list_folder_filtered_empty_mime_is_unfiltered() {
        let server = MockServer::start().await;
        let response = drive_files_json(&[
            ("id1", "1977", "application/vnd.google-apps.folder"),
            ("id2", "track.flac", "audio/flac"),
        ]);

        Mock::given(method("GET"))
            .and(path("/files"))
            .and(query_param("q", "'folder' in parents"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(&server)
            .await;

        let api = BmanApi::new_for_test("key".into(), "folder".into())
            .with_drive_base_url(&server.uri());

        let items = api.list_folder_filtered("folder", "").await.unwrap();
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn test_not_found_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/files"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(error_body("notFound", 404, "Not found")),
            )
            .mount(&server)
            .await;

        let api = BmanApi::new_for_test("key".into(), "missing".into())
            .with_drive_base_url(&server.uri());

        let result = api.list_folder("missing").await;
        assert!(matches!(result, Err(BmanError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_forbidden_fails_fast_no_retry() {
        let server = MockServer::start().await;
        // Forbidden must not be retried — expect exactly 1 call
        Mock::given(method("GET"))
            .and(path("/files"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_json(error_body("forbidden", 403, "Access denied")),
            )
            .expect(1)
            .mount(&server)
            .await;

        let api = BmanApi::new_for_test("key".into(), "folder".into())
            .with_drive_base_url(&server.uri());

        let result = api.list_folder("folder").await;
        assert!(matches!(result, Err(BmanError::Forbidden(_))));
    }

    #[tokio::test]
    async fn test_rate_limit_returned_after_retries() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/files"))
            .respond_with(
                ResponseTemplate::new(429)
                    .set_body_json(error_body("rateLimitExceeded", 429, "Rate limit")),
            )
            .mount(&server)
            .await;

        let api = BmanApi::new_for_test("key".into(), "folder".into())
            .with_drive_base_url(&server.uri());

        // Use drive_list_page directly to avoid pagination delay
        let result = api.drive_list_page("folder", "", None).await;
        assert!(matches!(result, Err(BmanError::RateLimit(_))));
    }

    #[test]
    fn test_download_url_contains_required_parts() {
        let api = BmanApi::new("MY_API_KEY".into());
        let url = api.download_url("file123");
        assert!(url.contains("file123"));
        assert!(url.contains("MY_API_KEY"));
        assert!(url.contains("alt=media"));
    }

    #[test]
    fn test_download_url_format() {
        let api = BmanApi::new("APIKEY".into());
        let url = api.download_url("abc");
        assert!(url.starts_with("https://www.googleapis.com/drive/v3/files/abc?"));
    }
}
