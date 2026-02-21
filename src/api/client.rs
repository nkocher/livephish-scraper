use std::path::PathBuf;
use std::time::Duration;

use reqwest::StatusCode;
use tokio::time::{sleep, Instant};

use crate::models::StreamParams;
use crate::service::{Service, ServiceAuth};

use super::error::ApiError;
use super::{MAX_RETRIES, RATE_LIMIT_DELAY_MS};

/// Nugs.net API client with authentication and streaming.
pub struct NugsApi {
    pub service: Service,
    pub(super) client: reqwest::Client,
    pub(super) access_token: Option<String>,
    pub(super) legacy_token: Option<String>,
    pub(super) legacy_uguid: Option<String>,
    pub(super) user_id: Option<String>,
    pub(super) email: Option<String>,
    pub(super) password: Option<String>,
    pub stream_params: Option<StreamParams>,
    pub(super) last_request_at: Option<Instant>,

    // Configurable base URLs (overridden in tests via new_for_test)
    pub(super) auth_url: String,
    pub(super) api_base: String,
    pub(super) sub_info_url: String,
    pub(super) user_info_url: String,
    pub(super) player_url: String,

    /// Override session cache file path (for testing).
    pub(super) session_cache_path: Option<PathBuf>,
}

impl NugsApi {
    pub fn new() -> Self {
        Self::new_for_service(Service::Nugs)
    }

    pub fn new_for_service(service: Service) -> Self {
        let cfg = service.config();

        let client = reqwest::Client::builder()
            .user_agent(cfg.user_agent)
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .expect("Failed to create HTTP client");

        // Resolve sub_info_url and user_info_url from service auth config
        let (sub_info_url, user_info_url) = match &cfg.auth {
            ServiceAuth::Nugs {
                sub_info_url,
                user_info_url,
            } => (sub_info_url.to_string(), user_info_url.to_string()),
            ServiceAuth::LivePhish { .. } => {
                // LivePhish uses secureApi.aspx for these — URLs built at call site
                (String::new(), String::new())
            }
        };

        NugsApi {
            service,
            client,
            access_token: None,
            legacy_token: None,
            legacy_uguid: None,
            user_id: None,
            email: None,
            password: None,
            stream_params: None,
            last_request_at: None,
            auth_url: cfg.auth_url.to_string(),
            api_base: cfg.api_base.to_string(),
            sub_info_url,
            user_info_url,
            player_url: cfg.player_url.to_string(),
            session_cache_path: None,
        }
    }

    /// Create a test client pointing all URLs at a mock server.
    #[cfg(test)]
    pub fn new_for_test(base_url: &str) -> Self {
        Self::new_for_test_service(base_url, Service::Nugs)
    }

    /// Create a test client for a specific service pointing all URLs at a mock server.
    #[cfg(test)]
    pub fn new_for_test_service(base_url: &str, service: Service) -> Self {
        let cfg = service.config();

        let client = reqwest::Client::builder()
            .user_agent(cfg.user_agent)
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .expect("Failed to create HTTP client");

        NugsApi {
            service,
            client,
            access_token: None,
            legacy_token: None,
            legacy_uguid: None,
            user_id: None,
            email: None,
            password: None,
            stream_params: None,
            last_request_at: None,
            auth_url: format!("{}/connect/token", base_url),
            api_base: format!("{}/", base_url),
            sub_info_url: format!("{}/api/v1/me/subscriptions", base_url),
            user_info_url: format!("{}/connect/userinfo", base_url),
            player_url: format!("{}/", base_url),
            session_cache_path: None,
        }
    }

    /// Enforce rate limiting between requests (0.5s minimum gap).
    pub(super) async fn rate_limit(&mut self) {
        if let Some(last) = self.last_request_at {
            let elapsed = last.elapsed();
            let delay = Duration::from_millis(RATE_LIMIT_DELAY_MS);
            if elapsed < delay {
                sleep(delay - elapsed).await;
            }
        }
    }

    /// Parse JSON from a response body string.
    pub(super) fn parse_json_value(
        body: &str,
        context: &str,
    ) -> Result<serde_json::Value, ApiError> {
        serde_json::from_str(body).map_err(|e| ApiError::JsonParse {
            context: if context.is_empty() {
                String::new()
            } else {
                format!(" for {}", context)
            },
            source: e,
        })
    }

    /// Make HTTP request with retry logic, rate limiting, and re-auth on 401.
    ///
    /// The `build` closure constructs a fresh `RequestBuilder` on each attempt,
    /// allowing retries without cloning the request.
    pub(super) async fn request(
        &mut self,
        build: impl Fn(&reqwest::Client) -> reqwest::RequestBuilder,
        rate_limit: bool,
        allow_reauth: bool,
    ) -> Result<reqwest::Response, ApiError> {
        if rate_limit {
            self.rate_limit().await;
        }

        // First, try the request with retry logic
        let response = self.request_with_retry(&build).await?;
        self.last_request_at = Some(Instant::now());

        // Re-auth on 401 if credentials are available
        if response.status() == StatusCode::UNAUTHORIZED
            && allow_reauth
            && self.email.is_some()
            && self.password.is_some()
        {
            let email = self.email.clone().unwrap();
            let password = self.password.clone().unwrap();

            // Re-authenticate (login calls request internally with allow_reauth=false)
            self.login(&email, &password).await.map_err(|auth_err| {
                ApiError::UnexpectedResponse(format!("Re-authentication failed: {}", auth_err))
            })?;

            // Retry the original request (no more re-auth)
            let response = self.request_with_retry(&build).await?;
            self.last_request_at = Some(Instant::now());
            return Ok(response);
        }

        Ok(response)
    }

    /// Execute a request with retry on transport errors (exponential backoff).
    ///
    /// Use this directly for auth endpoints that don't need rate limiting or re-auth
    /// (avoids async recursion: authenticate → request → login → authenticate).
    pub(super) async fn request_with_retry(
        &mut self,
        build: &impl Fn(&reqwest::Client) -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, ApiError> {
        for attempt in 0..MAX_RETRIES {
            match build(&self.client).send().await {
                Ok(response) => {
                    return Ok(response);
                }
                Err(e) => {
                    if attempt == MAX_RETRIES - 1 {
                        return Err(ApiError::RetryExhausted {
                            retries: MAX_RETRIES as u32,
                            message: e.to_string(),
                        });
                    }
                    // Exponential backoff: 1s, 2s, 4s
                    sleep(Duration::from_secs(2u64.pow(attempt as u32))).await;
                }
            }
        }

        unreachable!()
    }
}
