use thiserror::Error;

#[derive(Error, Debug)]
pub enum ApiError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Invalid JSON response{context}: {source}")]
    JsonParse {
        context: String,
        source: serde_json::Error,
    },

    #[error("Request failed after {retries} retries: {message}")]
    RetryExhausted { retries: u32, message: String },

    #[error("Unexpected API response: {0}")]
    UnexpectedResponse(String),

    #[error("Failed to resolve stream URL for track {0}")]
    StreamUrlFailed(i64),
}

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("Invalid email or password")]
    InvalidCredentials,

    #[error("Service temporarily unavailable")]
    ServiceUnavailable,

    #[error("Invalid JWT token format")]
    InvalidJwt,

    #[error("Failed to extract legacy tokens: {0}")]
    LegacyTokenExtraction(String),

    #[error("No access token available")]
    NoAccessToken,

    #[error("No stored credentials for session refresh")]
    NoCredentials,

    #[error("Authentication failed: HTTP {0}")]
    HttpStatus(u16),

    #[error("Missing access_token in response")]
    MissingAccessToken,

    #[error("{0}")]
    Api(#[from] ApiError),
}

#[derive(Error, Debug)]
pub enum SubscriptionError {
    #[error("Subscription does not allow streaming content")]
    NoStreamingAccess,

    #[error("Invalid timestamp in subscriber info: {0}")]
    InvalidTimestamp(String),

    #[error("{0}")]
    Api(#[from] ApiError),
}

/// Allow `SubscriptionError` to convert to `AuthError` (used in `login()` flow).
impl From<SubscriptionError> for AuthError {
    fn from(e: SubscriptionError) -> Self {
        match e {
            SubscriptionError::Api(api_err) => AuthError::Api(api_err),
            other => AuthError::Api(ApiError::UnexpectedResponse(other.to_string())),
        }
    }
}
