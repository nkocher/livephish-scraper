use chrono::{NaiveDateTime, TimeZone, Utc};

use crate::config::session::{
    load_session_cache, load_session_cache_from, save_session_cache, save_session_cache_to,
};
use crate::models::StreamParams;
use crate::service::Service;

use super::client::NugsApi;
use super::error::{ApiError, AuthError, SubscriptionError};

/// Coerce a JSON value to String, supporting both string and integer JSON types.
fn json_to_string(v: &serde_json::Value) -> String {
    v.as_str()
        .map(String::from)
        .unwrap_or_else(|| v.as_i64().map(|n| n.to_string()).unwrap_or_default())
}

/// Parse "MM/DD/YYYY HH:MM:SS" timestamp to Unix epoch seconds (UTC).
pub fn parse_timestamp(s: &str) -> Result<i64, String> {
    let naive = NaiveDateTime::parse_from_str(s, "%m/%d/%Y %H:%M:%S").map_err(|e| e.to_string())?;
    Ok(Utc.from_utc_datetime(&naive).timestamp())
}

impl NugsApi {
    /// Authenticate with OAuth2 password grant and get access token.
    pub async fn authenticate(&mut self, email: &str, password: &str) -> Result<String, AuthError> {
        let auth_url = self.auth_url.clone();
        let email_owned = email.to_string();
        let password_owned = password.to_string();
        let client_id = self.service.config().client_id.to_string();
        let scope = self.service.config().oauth_scope.to_string();

        // Auth endpoints use request_with_retry directly (no rate limit, no re-auth)
        // to avoid async recursion: authenticate → request → login → authenticate
        let response = self
            .request_with_retry(&|client| {
                client
                    .post(&auth_url)
                    .form(&[
                        ("client_id", client_id.as_str()),
                        ("grant_type", "password"),
                        ("scope", scope.as_str()),
                        ("username", email_owned.as_str()),
                        ("password", password_owned.as_str()),
                    ])
                    .header("Content-Type", "application/x-www-form-urlencoded")
            })
            .await?;

        let status = response.status().as_u16();
        if status != 200 {
            let body = response.text().await.unwrap_or_default();
            let error_key = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
                .unwrap_or_default();

            return Err(match error_key.as_str() {
                "invalid_grant" => AuthError::InvalidCredentials,
                "invalid_client" => AuthError::ServiceUnavailable,
                _ => AuthError::HttpStatus(status),
            });
        }

        let body = response.text().await.map_err(|e| {
            ApiError::UnexpectedResponse(format!("Failed to read response body: {}", e))
        })?;
        let data = NugsApi::parse_json_value(&body, "authentication")?;

        let access_token = data
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or(AuthError::MissingAccessToken)?
            .to_string();

        self.access_token = Some(access_token.clone());
        Ok(access_token)
    }

    /// Extract legacy_token and legacy_uguid from JWT access token.
    ///
    /// JWT payload is base64url-decoded (with padding fix) then parsed as JSON.
    pub fn extract_legacy_tokens(&self) -> Result<(String, String), AuthError> {
        let token = self.access_token.as_ref().ok_or(AuthError::NoAccessToken)?;

        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() < 2 {
            return Err(AuthError::InvalidJwt);
        }

        // JWT payloads are base64url-encoded without padding (RFC 7515).
        // URL_SAFE_NO_PAD handles this natively — no padding fix needed.
        let decoded =
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, parts[1])
                .map_err(|e| AuthError::LegacyTokenExtraction(e.to_string()))?;

        let data: serde_json::Value = serde_json::from_slice(&decoded)
            .map_err(|e| AuthError::LegacyTokenExtraction(e.to_string()))?;

        let legacy_token = data
            .get("legacy_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AuthError::LegacyTokenExtraction("missing legacy_token".to_string()))?
            .to_string();

        let legacy_uguid = data
            .get("legacy_uguid")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AuthError::LegacyTokenExtraction("missing legacy_uguid".to_string()))?
            .to_string();

        Ok((legacy_token, legacy_uguid))
    }

    /// Get user ID from OpenID Connect userinfo endpoint.
    pub async fn get_user_id(&mut self) -> Result<String, ApiError> {
        let user_info_url = self.user_info_url.clone();
        let token = self.access_token.clone().unwrap_or_default();

        let response = self
            .request_with_retry(&|client| {
                client
                    .get(&user_info_url)
                    .header("Authorization", format!("Bearer {}", token))
            })
            .await?;

        if response.status().as_u16() != 200 {
            return Err(ApiError::UnexpectedResponse(format!(
                "Failed to get user info: HTTP {}",
                response.status().as_u16()
            )));
        }

        let body = response.text().await.map_err(|e| {
            ApiError::UnexpectedResponse(format!("Failed to read response body: {}", e))
        })?;
        let data = NugsApi::parse_json_value(&body, "user info")?;

        data.get("sub")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| {
                ApiError::UnexpectedResponse("Missing sub field in user info response".to_string())
            })
    }

    /// Get subscriber info and stream parameters.
    ///
    /// Returns (StreamParams, plan_name).
    pub async fn get_subscriber_info(
        &mut self,
        user_id: &str,
    ) -> Result<(StreamParams, String), SubscriptionError> {
        let sub_info_url = self.sub_info_url.clone();
        let token = self.access_token.clone().unwrap_or_default();

        let response = self
            .request_with_retry(&|client| {
                client
                    .get(&sub_info_url)
                    .header("Authorization", format!("Bearer {}", token))
            })
            .await?;

        if response.status().as_u16() != 200 {
            return Err(ApiError::UnexpectedResponse(format!(
                "Failed to get subscriber info: HTTP {}",
                response.status().as_u16()
            ))
            .into());
        }

        let body = response.text().await.map_err(|e| {
            ApiError::UnexpectedResponse(format!("Failed to read response body: {}", e))
        })?;
        let data = NugsApi::parse_json_value(&body, "subscriber info")?;

        // Check streaming access
        if !data
            .get("isContentAccessible")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Err(SubscriptionError::NoStreamingAccess);
        }

        // Parse timestamps: "MM/DD/YYYY HH:MM:SS" → Unix epoch
        let started_at = data
            .get("startedAt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SubscriptionError::InvalidTimestamp("missing startedAt".to_string()))?;

        let ends_at = data
            .get("endsAt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SubscriptionError::InvalidTimestamp("missing endsAt".to_string()))?;

        let start_stamp =
            parse_timestamp(started_at).map_err(SubscriptionError::InvalidTimestamp)?;
        let end_stamp = parse_timestamp(ends_at).map_err(SubscriptionError::InvalidTimestamp)?;

        // Determine plan (regular vs promo)
        let is_promo = data
            .get("plan")
            .and_then(|p| p.get("planId"))
            .and_then(|v| v.as_str())
            .is_none_or(|s| s.is_empty());

        let empty_obj = serde_json::Value::Object(serde_json::Map::new());
        let plan = if is_promo {
            data.get("promo")
                .and_then(|p| p.get("plan"))
                .unwrap_or(&empty_obj)
        } else {
            data.get("plan").unwrap_or(&empty_obj)
        };

        let user_id_owned = user_id.to_string();
        let stream_params = StreamParams {
            subscription_id: data
                .get("legacySubscriptionId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            sub_costplan_id_access_list: plan
                .get("planId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            user_id: user_id_owned,
            start_stamp: start_stamp.to_string(),
            end_stamp: end_stamp.to_string(),
        };

        let plan_name = plan
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Plan")
            .to_string();

        Ok((stream_params, plan_name))
    }

    /// Dispatch login based on service type.
    ///
    /// Stores credentials for later re-auth and delegates to the appropriate flow.
    pub async fn login(
        &mut self,
        email: &str,
        password: &str,
    ) -> Result<(StreamParams, String), AuthError> {
        self.email = Some(email.to_string());
        self.password = Some(password.to_string());
        match self.service {
            Service::Nugs => self.login_nugs(email, password).await,
            Service::LivePhish => self.login_livephish(email, password).await,
            Service::Bman => unreachable!("Bman uses Google API key auth, not NugsApi login"),
        }
    }

    /// Complete nugs.net login flow: authenticate → extract legacy tokens → get user ID → get subscriber info.
    async fn login_nugs(
        &mut self,
        email: &str,
        password: &str,
    ) -> Result<(StreamParams, String), AuthError> {
        self.authenticate(email, password).await?;

        let (legacy_token, legacy_uguid) = self.extract_legacy_tokens()?;
        self.legacy_token = Some(legacy_token);
        self.legacy_uguid = Some(legacy_uguid);

        let user_id = self.get_user_id().await?;
        self.user_id = Some(user_id.clone());

        let (params, plan_name) = self.get_subscriber_info(&user_id).await?;
        self.stream_params = Some(params.clone());

        Ok((params, plan_name))
    }

    /// Complete LivePhish login flow: OAuth2 → legacy session token → subscriber info.
    async fn login_livephish(
        &mut self,
        email: &str,
        password: &str,
    ) -> Result<(StreamParams, String), AuthError> {
        // Step 1: OAuth2 password grant (same mechanism as nugs, uses service config)
        self.authenticate(email, password).await?;

        // Step 2: Legacy session token via secureApi.aspx
        let session_token = self.get_livephish_user_token(email, password).await?;
        self.legacy_token = Some(session_token.clone());

        // Step 3: Subscriber info via secureApi.aspx
        let (params, plan_name) = self
            .get_livephish_subscriber_info(email, &session_token)
            .await?;
        self.stream_params = Some(params.clone());

        Ok((params, plan_name))
    }

    /// Fetch legacy session token from LivePhish secureApi.aspx.
    ///
    /// Returns `Response.tokenValue`.
    async fn get_livephish_user_token(
        &mut self,
        email: &str,
        password: &str,
    ) -> Result<String, AuthError> {
        let api_base = self.api_base.clone();
        let client_id = self.service.config().client_id.to_string();
        let developer_key = self.service.config().developer_key.to_string();
        let email_owned = email.to_string();
        let password_owned = password.to_string();

        let url = format!(
            "{}secureApi.aspx?method=session.getUserToken&clientID={}&developerKey={}&user={}&pw={}",
            api_base, client_id, developer_key, email_owned, password_owned
        );

        // Auth endpoint: bypass rate limit and re-auth to avoid async recursion
        let response = self.request_with_retry(&|client| client.get(&url)).await?;

        let status = response.status().as_u16();
        if status != 200 {
            return Err(AuthError::HttpStatus(status));
        }

        let body = response.text().await.map_err(|e| {
            ApiError::UnexpectedResponse(format!("Failed to read user token response: {}", e))
        })?;
        let data = NugsApi::parse_json_value(&body, "livephish user token")?;

        data.get("Response")
            .and_then(|r| r.get("tokenValue"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| {
                AuthError::Api(ApiError::UnexpectedResponse(
                    "Missing Response.tokenValue in LivePhish user token response".to_string(),
                ))
            })
    }

    /// Fetch subscriber info from LivePhish secureApi.aspx.
    ///
    /// Returns `(StreamParams, plan_name)`.
    async fn get_livephish_subscriber_info(
        &mut self,
        email: &str,
        session_token: &str,
    ) -> Result<(StreamParams, String), AuthError> {
        let api_base = self.api_base.clone();
        let developer_key = self.service.config().developer_key.to_string();
        let access_token = self.access_token.clone().unwrap_or_default();
        let email_owned = email.to_string();
        let session_token_owned = session_token.to_string();

        let url = format!(
            "{}secureApi.aspx?method=user.getSubscriberInfo&developerKey={}&user={}&token={}",
            api_base, developer_key, email_owned, session_token_owned
        );

        let response = self
            .request_with_retry(&|client| {
                client
                    .get(&url)
                    .header("Authorization", format!("Bearer {}", access_token))
            })
            .await?;

        let status = response.status().as_u16();
        if status != 200 {
            return Err(AuthError::HttpStatus(status));
        }

        let body = response.text().await.map_err(|e| {
            ApiError::UnexpectedResponse(format!("Failed to read subscriber info response: {}", e))
        })?;
        let data = NugsApi::parse_json_value(&body, "livephish subscriber info")?;

        let sub_info = data
            .get("Response")
            .and_then(|r| r.get("subscriptionInfo"))
            .ok_or_else(|| {
                AuthError::Api(ApiError::UnexpectedResponse(
                    "Missing Response.subscriptionInfo in LivePhish subscriber info".to_string(),
                ))
            })?;

        // Check streaming access
        if !sub_info
            .get("canStreamSubContent")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Err(AuthError::Api(ApiError::UnexpectedResponse(
                "LivePhish subscription does not allow streaming content".to_string(),
            )));
        }

        let stream_params = StreamParams {
            subscription_id: json_to_string(
                sub_info
                    .get("subscriptionID")
                    .unwrap_or(&serde_json::Value::Null),
            ),
            sub_costplan_id_access_list: sub_info
                .get("subCostplanIDAccessList")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            user_id: json_to_string(sub_info.get("userID").unwrap_or(&serde_json::Value::Null)),
            start_stamp: json_to_string(
                sub_info
                    .get("startDateStamp")
                    .unwrap_or(&serde_json::Value::Null),
            ),
            end_stamp: json_to_string(
                sub_info
                    .get("endDateStamp")
                    .unwrap_or(&serde_json::Value::Null),
            ),
        };

        let plan_name = sub_info
            .get("planName")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Plan")
            .to_string();

        Ok((stream_params, plan_name))
    }

    /// Try cached session first, fall back to full login.
    ///
    /// Trusts cached tokens within TTL without a validation call.
    /// Returns (StreamParams, status_message).
    pub async fn login_cached(
        &mut self,
        email: &str,
        password: &str,
    ) -> Result<(StreamParams, String), AuthError> {
        // Store credentials for re-auth in request()
        self.email = Some(email.to_string());
        self.password = Some(password.to_string());

        // Try loading cached session
        let cached = match &self.session_cache_path {
            Some(path) => load_session_cache_from(path),
            None => load_session_cache(),
        };

        if let Some(cache) = cached {
            self.access_token = Some(cache.access_token);
            self.legacy_token = Some(cache.legacy_token);
            self.legacy_uguid = Some(cache.legacy_uguid);
            self.user_id = Some(cache.user_id);
            let params = cache.stream_params;
            self.stream_params = Some(params.clone());
            return Ok((params, "Cached session".to_string()));
        }

        // Full auth flow
        let (params, plan_name) = self.login(email, password).await?;

        // Save to cache
        match &self.session_cache_path {
            Some(path) => save_session_cache_to(
                path,
                self.access_token.as_deref().unwrap_or(""),
                self.legacy_token.as_deref().unwrap_or(""),
                self.legacy_uguid.as_deref().unwrap_or(""),
                self.user_id.as_deref().unwrap_or(""),
                &params,
            ),
            None => save_session_cache(
                self.access_token.as_deref().unwrap_or(""),
                self.legacy_token.as_deref().unwrap_or(""),
                self.legacy_uguid.as_deref().unwrap_or(""),
                self.user_id.as_deref().unwrap_or(""),
                &params,
            ),
        }

        Ok((params, plan_name))
    }

    /// Re-authenticate and update stream_params atomically.
    ///
    /// Calls login() with stored credentials, persists new session to cache.
    pub async fn refresh_session(&mut self) -> Result<StreamParams, AuthError> {
        let email = self.email.clone().ok_or(AuthError::NoCredentials)?;
        let password = self.password.clone().ok_or(AuthError::NoCredentials)?;

        let (params, _) = self.login(&email, &password).await?;

        // Persist refreshed session to cache
        match &self.session_cache_path {
            Some(path) => save_session_cache_to(
                path,
                self.access_token.as_deref().unwrap_or(""),
                self.legacy_token.as_deref().unwrap_or(""),
                self.legacy_uguid.as_deref().unwrap_or(""),
                self.user_id.as_deref().unwrap_or(""),
                &params,
            ),
            None => save_session_cache(
                self.access_token.as_deref().unwrap_or(""),
                self.legacy_token.as_deref().unwrap_or(""),
                self.legacy_uguid.as_deref().unwrap_or(""),
                self.user_id.as_deref().unwrap_or(""),
                &params,
            ),
        }

        Ok(params)
    }
}
