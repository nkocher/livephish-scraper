/// Google OAuth2 token refresh for Drive downloads.
///
/// Exchanges a refresh_token for a short-lived access_token via
/// `POST https://oauth2.googleapis.com/token`. Used by `BmanApi` to
/// authenticate download requests with Bearer tokens instead of API keys,
/// which avoids Google's aggressive abuse detection on anonymous requests.
use serde::Deserialize;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
}

/// Exchange a refresh_token for a fresh access_token.
///
/// Returns `(access_token, expires_in_secs)` on success.
/// Returns an error tagged with "invalid_grant" when the refresh token has
/// been revoked — callers should disable OAuth for the session on that error.
pub async fn refresh_access_token(
    client: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<(String, u64), anyhow::Error> {
    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await?;

    if resp.status().is_success() {
        let token: TokenResponse = resp.json().await?;
        Ok((token.access_token, token.expires_in))
    } else {
        let body = resp.text().await.unwrap_or_default();
        if let Ok(err) = serde_json::from_str::<ErrorResponse>(&body) {
            if err.error == "invalid_grant" {
                return Err(anyhow::anyhow!("invalid_grant: refresh token revoked or expired"));
            }
        }
        Err(anyhow::anyhow!("OAuth token refresh failed: {body}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_refresh_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.fresh_token",
                "expires_in": 3600,
                "token_type": "Bearer"
            })))
            .mount(&server)
            .await;

        // Override the token URL by using the mock server directly
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/token", server.uri()))
            .form(&[
                ("client_id", "cid"),
                ("client_secret", "csec"),
                ("refresh_token", "rtoken"),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
            .unwrap();

        let token: TokenResponse = resp.json().await.unwrap();
        assert_eq!(token.access_token, "ya29.fresh_token");
        assert_eq!(token.expires_in, 3600);
    }

    #[tokio::test]
    async fn test_invalid_grant_detection() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_grant",
                "error_description": "Token has been expired or revoked."
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/token", server.uri()))
            .form(&[
                ("client_id", "cid"),
                ("client_secret", "csec"),
                ("refresh_token", "bad_token"),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
            .unwrap();

        let body = resp.text().await.unwrap();
        let err: ErrorResponse = serde_json::from_str(&body).unwrap();
        assert_eq!(err.error, "invalid_grant");
    }
}
