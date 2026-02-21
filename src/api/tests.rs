use std::sync::atomic::{AtomicUsize, Ordering};

use base64::Engine;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::config::session::save_session_cache_to;
use crate::models::StreamParams;

use super::auth::parse_timestamp;
use super::client::NugsApi;
use super::error::{ApiError, AuthError, SubscriptionError};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a test JWT token with the given payload.
fn make_jwt(payload: &serde_json::Value) -> String {
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(json!({"alg": "RS256"}).to_string().as_bytes());
    let payload_b64 =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
    let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"signature");
    format!("{}.{}.{}", header, payload_b64, sig)
}

/// Create a JWT where the unpadded payload base64 has len%4 == target_mod.
fn make_jwt_with_padding(
    base_payload: &serde_json::Value,
    target_mod: usize,
) -> (String, serde_json::Value) {
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(json!({"alg": "RS256"}).to_string().as_bytes());
    let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"signature");

    for i in 0..20 {
        let mut payload = base_payload.as_object().unwrap().clone();
        if i > 0 {
            payload.insert("pad".to_string(), json!("x".repeat(i)));
        }
        let payload_val = serde_json::Value::Object(payload.clone());
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(payload_val.to_string().as_bytes());
        if encoded.len() % 4 == target_mod {
            return (format!("{}.{}.{}", header, encoded, sig), payload_val);
        }
    }
    panic!("Could not construct JWT with payload len%4=={}", target_mod);
}

/// Standard subscriber info response JSON.
fn sub_info_json(sub_id: &str, plan_id: &str, plan_desc: &str) -> serde_json::Value {
    json!({
        "isContentAccessible": true,
        "legacySubscriptionId": sub_id,
        "startedAt": "01/15/2025 12:00:00",
        "endsAt": "01/15/2026 12:00:00",
        "plan": {
            "planId": plan_id,
            "description": plan_desc,
        }
    })
}

/// Mount full auth flow mocks (POST token, GET userinfo, GET sub info).
async fn mount_auth_flow(server: &MockServer, jwt_token: &str, sub_id: &str, plan_desc: &str) {
    Mock::given(method("POST"))
        .and(path("/connect/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": jwt_token})))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/connect/userinfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"sub": "user_123"})))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api/v1/me/subscriptions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(sub_info_json(sub_id, "plan_456", plan_desc)),
        )
        .mount(server)
        .await;
}

// ---------------------------------------------------------------------------
// Authentication tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_authenticate_success() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/connect/token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"access_token": "test_token"})),
        )
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    let token = api
        .authenticate("user@example.com", "password123")
        .await
        .unwrap();

    assert_eq!(token, "test_token");
    assert_eq!(api.access_token.as_deref(), Some("test_token"));
}

#[tokio::test]
async fn test_authenticate_failure_401() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/connect/token"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    let err = api
        .authenticate("user@example.com", "wrong_password")
        .await
        .unwrap_err();

    assert!(matches!(err, AuthError::HttpStatus(401)));
}

#[tokio::test]
async fn test_authenticate_invalid_grant() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/connect/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})))
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    let err = api
        .authenticate("user@example.com", "wrong_password")
        .await
        .unwrap_err();

    assert!(matches!(err, AuthError::InvalidCredentials));
}

#[tokio::test]
async fn test_authenticate_invalid_client() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/connect/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_client"})))
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    let err = api
        .authenticate("user@example.com", "password123")
        .await
        .unwrap_err();

    assert!(matches!(err, AuthError::ServiceUnavailable));
}

// ---------------------------------------------------------------------------
// JWT / legacy token tests
// ---------------------------------------------------------------------------

#[test]
fn test_extract_legacy_tokens() {
    let payload = json!({
        "legacy_token": "legacy_abc123",
        "legacy_uguid": "guid-456",
        "sub": "user_789",
    });
    let mut api = NugsApi::new_for_test("http://unused");
    api.access_token = Some(make_jwt(&payload));

    let (legacy_token, legacy_uguid) = api.extract_legacy_tokens().unwrap();

    assert_eq!(legacy_token, "legacy_abc123");
    assert_eq!(legacy_uguid, "guid-456");
}

#[test]
fn test_jwt_padding_aligned() {
    let (jwt_token, payload) = make_jwt_with_padding(
        &json!({"legacy_token": "tok_a", "legacy_uguid": "uid_a"}),
        0,
    );
    let parts: Vec<&str> = jwt_token.split('.').collect();
    assert_eq!(parts[1].len() % 4, 0);

    let mut api = NugsApi::new_for_test("http://unused");
    api.access_token = Some(jwt_token);
    let (lt, lg) = api.extract_legacy_tokens().unwrap();
    assert_eq!(lt, payload["legacy_token"].as_str().unwrap());
    assert_eq!(lg, payload["legacy_uguid"].as_str().unwrap());
}

#[test]
fn test_jwt_padding_needs_two() {
    let (jwt_token, payload) = make_jwt_with_padding(
        &json!({"legacy_token": "tok_b", "legacy_uguid": "uid_b"}),
        2,
    );
    let parts: Vec<&str> = jwt_token.split('.').collect();
    assert_eq!(parts[1].len() % 4, 2);

    let mut api = NugsApi::new_for_test("http://unused");
    api.access_token = Some(jwt_token);
    let (lt, lg) = api.extract_legacy_tokens().unwrap();
    assert_eq!(lt, payload["legacy_token"].as_str().unwrap());
    assert_eq!(lg, payload["legacy_uguid"].as_str().unwrap());
}

#[test]
fn test_jwt_padding_needs_one() {
    let (jwt_token, payload) = make_jwt_with_padding(
        &json!({"legacy_token": "tok_c", "legacy_uguid": "uid_c"}),
        3,
    );
    let parts: Vec<&str> = jwt_token.split('.').collect();
    assert_eq!(parts[1].len() % 4, 3);

    let mut api = NugsApi::new_for_test("http://unused");
    api.access_token = Some(jwt_token);
    let (lt, lg) = api.extract_legacy_tokens().unwrap();
    assert_eq!(lt, payload["legacy_token"].as_str().unwrap());
    assert_eq!(lg, payload["legacy_uguid"].as_str().unwrap());
}

// ---------------------------------------------------------------------------
// User info & subscriber tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_user_id() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/connect/userinfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"sub": "user_123"})))
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    api.access_token = Some("test_token".to_string());

    let user_id = api.get_user_id().await.unwrap();
    assert_eq!(user_id, "user_123");
}

#[tokio::test]
async fn test_get_subscriber_info_success() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/me/subscriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(sub_info_json(
            "sub_123",
            "plan_456",
            "Premium Annual",
        )))
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    api.access_token = Some("test_token".to_string());

    let (params, plan_name) = api.get_subscriber_info("user_123").await.unwrap();

    assert_eq!(params.subscription_id, "sub_123");
    assert_eq!(params.sub_costplan_id_access_list, "plan_456");
    assert_eq!(params.user_id, "user_123");
    assert_eq!(plan_name, "Premium Annual");
}

#[tokio::test]
async fn test_get_subscriber_info_no_subscription() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/me/subscriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "isContentAccessible": false,
            "legacySubscriptionId": "sub_123",
            "startedAt": "01/15/2025 12:00:00",
            "endsAt": "01/15/2026 12:00:00",
        })))
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    api.access_token = Some("test_token".to_string());

    let err = api.get_subscriber_info("user_123").await.unwrap_err();
    assert!(matches!(err, SubscriptionError::NoStreamingAccess));
}

#[test]
fn test_timestamp_parsing_utc() {
    let timestamp = parse_timestamp("01/15/2025 12:00:00").unwrap();

    use chrono::DateTime;
    let dt = DateTime::from_timestamp(timestamp, 0).unwrap();
    assert_eq!(
        dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        "2025-01-15 12:00:00"
    );
}

// ---------------------------------------------------------------------------
// Catalog tests
// ---------------------------------------------------------------------------

/// Custom wiremock responder for paginated catalog results.
struct PaginatedCatalog;

impl wiremock::Respond for PaginatedCatalog {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let offset = request
            .url
            .query_pairs()
            .find(|(k, _)| k == "startOffset")
            .and_then(|(_, v)| v.parse::<i64>().ok())
            .unwrap_or(1);

        if offset == 1 {
            ResponseTemplate::new(200).set_body_json(json!({
                "Response": {
                    "containers": [
                        {"containerID": 1, "artistName": "Dead & Company"},
                        {"containerID": 2, "artistName": "Dead & Company"},
                    ]
                }
            }))
        } else {
            ResponseTemplate::new(200).set_body_json(json!({"Response": {"containers": []}}))
        }
    }
}

#[tokio::test]
async fn test_get_artist_catalog() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api.aspx"))
        .and(query_param("method", "catalog.containersAll"))
        .respond_with(PaginatedCatalog)
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    let containers = api.get_artist_catalog(42, 100).await.unwrap();

    assert_eq!(containers.len(), 2);
    assert_eq!(containers[0]["containerID"], 1);
    assert_eq!(containers[1]["containerID"], 2);
}

#[tokio::test]
async fn test_get_artist_catalog_defensive_parsing() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api.aspx"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"error": "something wrong"})))
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    let containers = api.get_artist_catalog(42, 100).await.unwrap();
    assert_eq!(containers.len(), 0);
}

#[tokio::test]
async fn test_get_show_detail() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api.aspx"))
        .and(query_param("method", "catalog.container"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Response": {
                "containerID": 12345,
                "artistName": "Dead & Company",
                "containerInfo": "Sphere Las Vegas",
                "venueName": "Sphere",
                "venueCity": "Las Vegas",
                "venueState": "NV",
                "performanceDate": "2024-05-16",
                "performanceDateFormatted": "May 16, 2024",
                "performanceDateYear": "2024",
                "tracks": [
                    {
                        "trackID": 1,
                        "songID": 100,
                        "songTitle": "Dark Star",
                        "trackNum": 1,
                        "discNum": 1,
                        "setNum": 1,
                    }
                ],
            }
        })))
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    let show = api.get_show_detail(12345).await.unwrap();

    assert_eq!(show.container_id, 12345);
    assert_eq!(show.artist_name, "Dead & Company");
    assert_eq!(show.venue_name, "Sphere");
    assert_eq!(show.tracks.len(), 1);
    assert_eq!(show.tracks[0].song_title, "Dark Star");
}

// ---------------------------------------------------------------------------
// Stream URL tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_stream_url_no_signature() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/bigriver/subPlayer.aspx"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"streamLink": "https://stream.url/track.flac"})),
        )
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    let stream_params = StreamParams {
        subscription_id: "sub_123".to_string(),
        sub_costplan_id_access_list: "plan_456".to_string(),
        user_id: "user_789".to_string(),
        start_stamp: "1700000000".to_string(),
        end_stamp: "1800000000".to_string(),
    };

    let url = api.get_stream_url(999, 2, &stream_params).await.unwrap();
    assert_eq!(url, "https://stream.url/track.flac");

    // Verify request was made (wiremock would fail if no mock matched)
    let received = server.received_requests().await.unwrap();
    let stream_req = received
        .iter()
        .find(|r| r.url.path() == "/bigriver/subPlayer.aspx")
        .unwrap();

    // Verify NO signature params (unlike livephish)
    let has_tk = stream_req.url.query_pairs().any(|(k, _)| k == "tk");
    let has_lxp = stream_req.url.query_pairs().any(|(k, _)| k == "lxp");
    assert!(!has_tk, "stream URL should not contain tk param");
    assert!(!has_lxp, "stream URL should not contain lxp param");

    // Verify expected params
    let track_id: String = stream_req
        .url
        .query_pairs()
        .find(|(k, _)| k == "trackID")
        .map(|(_, v)| v.to_string())
        .unwrap();
    assert_eq!(track_id, "999");
}

// ---------------------------------------------------------------------------
// Playlist tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_playlist_catalog() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api.aspx"))
        .and(query_param("method", "catalog.playlist"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Response": {
                "playListName": "Best of 2024",
                "numTracks": 2,
                "items": [
                    {
                        "track": {
                            "trackID": 1, "songID": 100, "songTitle": "Song 1",
                            "trackNum": 1, "discNum": 1, "setNum": 1,
                        },
                        "playlistContainer": {
                            "containerID": 123, "containerInfo": "Show 1",
                            "artistName": "Dead & Company", "venueName": "Venue 1",
                            "performanceDate": "2024-01-01",
                        },
                    },
                    {
                        "track": {
                            "trackID": 2, "songID": 101, "songTitle": "Song 2",
                            "trackNum": 1, "discNum": 1, "setNum": 1,
                        },
                        "playlistContainer": {
                            "containerID": 124, "containerInfo": "Show 2",
                            "artistName": "Dead & Company", "venueName": "Venue 2",
                            "performanceDate": "2024-01-02",
                        },
                    },
                ],
            }
        })))
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    let playlist = api.get_playlist("pl_guid_123", true).await.unwrap();

    assert_eq!(playlist.playlist_name, "Best of 2024");
    assert_eq!(playlist.num_tracks, 2);
    assert_eq!(playlist.items.len(), 2);
    assert_eq!(playlist.items[0].track.song_title, "Song 1");
}

#[tokio::test]
async fn test_get_playlist_user() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/secureApi.aspx"))
        .and(query_param("method", "user.playlist"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Response": {
                "playListName": "My Playlist",
                "numTracks": 1,
                "items": [
                    {
                        "track": {
                            "trackID": 1, "songID": 100, "songTitle": "Test Song",
                            "trackNum": 1, "discNum": 1, "setNum": 1,
                        },
                        "playlistContainer": {
                            "containerID": 123, "containerInfo": "Test Show",
                            "artistName": "Test Artist", "venueName": "Test Venue",
                            "performanceDate": "2024-01-01",
                        },
                    },
                ],
            }
        })))
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    api.email = Some("user@example.com".to_string());
    api.legacy_token = Some("legacy_token_abc".to_string());

    let playlist = api.get_playlist("123", false).await.unwrap();

    assert_eq!(playlist.playlist_name, "My Playlist");
    assert_eq!(playlist.num_tracks, 1);
}

// ---------------------------------------------------------------------------
// Login flow tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_login_convenience() {
    let server = MockServer::start().await;

    let payload = json!({
        "legacy_token": "legacy_abc123",
        "legacy_uguid": "guid-456",
        "sub": "user_789",
    });
    let jwt_token = make_jwt(&payload);

    mount_auth_flow(&server, &jwt_token, "sub_123", "Premium Annual").await;

    let mut api = NugsApi::new_for_test(&server.uri());
    let (stream_params, plan_name) = api.login("user@example.com", "password123").await.unwrap();

    assert_eq!(plan_name, "Premium Annual");
    assert_eq!(api.access_token.as_deref(), Some(jwt_token.as_str()));
    assert_eq!(api.legacy_token.as_deref(), Some("legacy_abc123"));
    assert_eq!(api.legacy_uguid.as_deref(), Some("guid-456"));
    assert_eq!(api.user_id.as_deref(), Some("user_123"));
    assert_eq!(stream_params.subscription_id, "sub_123");
}

#[tokio::test]
async fn test_login_stores_stream_params() {
    let server = MockServer::start().await;

    let payload = json!({"legacy_token": "lt", "legacy_uguid": "lg", "sub": "u"});
    let jwt_token = make_jwt(&payload);

    mount_auth_flow(&server, &jwt_token, "sub1", "Plan").await;

    let mut api = NugsApi::new_for_test(&server.uri());
    let (params, _) = api.login("u@test.com", "pw").await.unwrap();

    assert!(api.stream_params.is_some());
    assert_eq!(api.stream_params.as_ref().unwrap().subscription_id, "sub1");
    assert_eq!(params.subscription_id, "sub1");
}

// ---------------------------------------------------------------------------
// Cached login tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_login_cached_cache_miss() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let cache_path = tmp.path().join("session.json");

    let payload =
        json!({"legacy_token": "legacy_new", "legacy_uguid": "guid_new", "sub": "user_new"});
    let jwt_token = make_jwt(&payload);

    mount_auth_flow(&server, &jwt_token, "sub_new", "New Plan").await;

    let mut api = NugsApi::new_for_test(&server.uri());
    api.session_cache_path = Some(cache_path.clone());

    let (params, status) = api.login_cached("user@example.com", "pass").await.unwrap();

    assert_eq!(status, "New Plan");
    assert_eq!(params.subscription_id, "sub_new");
    assert_eq!(api.email.as_deref(), Some("user@example.com"));
    assert_eq!(api.password.as_deref(), Some("pass"));
    // Cache file should have been created
    assert!(cache_path.exists());
}

#[tokio::test]
async fn test_login_cached_cache_hit() {
    let tmp = tempfile::tempdir().unwrap();
    let cache_path = tmp.path().join("session.json");

    // Pre-populate cache
    let cached_params = StreamParams {
        subscription_id: "sub_cached".to_string(),
        sub_costplan_id_access_list: "plan_cached".to_string(),
        user_id: "cached_user".to_string(),
        start_stamp: "100".to_string(),
        end_stamp: "200".to_string(),
    };
    save_session_cache_to(
        &cache_path,
        "cached_token",
        "cached_legacy",
        "cached_guid",
        "cached_user",
        &cached_params,
    );

    // No mock server needed — cache hit should not make API calls
    let mut api = NugsApi::new_for_test("http://unused:9999");
    api.session_cache_path = Some(cache_path);

    let (params, status) = api.login_cached("user@example.com", "pass").await.unwrap();

    assert_eq!(status, "Cached session");
    assert_eq!(params.subscription_id, "sub_cached");
    assert_eq!(api.access_token.as_deref(), Some("cached_token"));
    assert_eq!(api.legacy_token.as_deref(), Some("cached_legacy"));
    assert_eq!(api.email.as_deref(), Some("user@example.com"));
}

#[tokio::test]
async fn test_login_cached_stores_stream_params() {
    let tmp = tempfile::tempdir().unwrap();
    let cache_path = tmp.path().join("session.json");

    let cached_params = StreamParams {
        subscription_id: "cached_sub".to_string(),
        sub_costplan_id_access_list: "p".to_string(),
        user_id: "u".to_string(),
        start_stamp: "1".to_string(),
        end_stamp: "2".to_string(),
    };
    save_session_cache_to(&cache_path, "t", "lt", "lg", "u", &cached_params);

    let mut api = NugsApi::new_for_test("http://unused:9999");
    api.session_cache_path = Some(cache_path);

    let (params, _) = api.login_cached("u@test.com", "pw").await.unwrap();

    assert!(api.stream_params.is_some());
    assert_eq!(
        api.stream_params.as_ref().unwrap().subscription_id,
        "cached_sub"
    );
    assert_eq!(params.subscription_id, "cached_sub");
}

#[tokio::test]
async fn test_login_cached_corrupt_cache() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let cache_path = tmp.path().join("session.json");

    // Write corrupt cache (missing required fields)
    std::fs::write(&cache_path, r#"{"access_token": "old", "not_valid": true}"#).unwrap();

    let payload =
        json!({"legacy_token": "legacy_fresh", "legacy_uguid": "guid_fresh", "sub": "user_fresh"});
    let jwt_token = make_jwt(&payload);

    mount_auth_flow(&server, &jwt_token, "sub_fresh", "Fresh Plan").await;

    let mut api = NugsApi::new_for_test(&server.uri());
    api.session_cache_path = Some(cache_path);

    let (_, status) = api.login_cached("user@example.com", "pass").await.unwrap();

    // Corrupt cache is treated as cache miss → full auth
    assert_eq!(status, "Fresh Plan");
}

// ---------------------------------------------------------------------------
// Re-auth on 401 tests
// ---------------------------------------------------------------------------

/// Custom responder: first call returns 401, subsequent calls return 200.
struct ReauthResponder {
    call_count: AtomicUsize,
}

impl wiremock::Respond for ReauthResponder {
    fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
        let n = self.call_count.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            ResponseTemplate::new(401).set_body_string("Unauthorized")
        } else {
            ResponseTemplate::new(200).set_body_json(json!({
                "Response": {
                    "containerID": 1,
                    "artistName": "Dead & Company",
                    "containerInfo": "Test",
                    "venueName": "Venue",
                    "venueCity": "City",
                    "venueState": "ST",
                    "performanceDate": "2024-01-01",
                    "performanceDateFormatted": "01/01/2024",
                    "performanceDateYear": "2024",
                    "tracks": [],
                }
            }))
        }
    }
}

#[tokio::test]
async fn test_reauth_on_401() {
    let server = MockServer::start().await;

    let payload = json!({"legacy_token": "legacy_reauth", "legacy_uguid": "guid_reauth", "sub": "user_reauth"});
    let jwt_token = make_jwt(&payload);

    // Mock auth flow for re-auth
    mount_auth_flow(&server, &jwt_token, "sub_reauth", "Reauth Plan").await;

    // Mock api.aspx: first call 401, second 200
    Mock::given(method("GET"))
        .and(path("/api.aspx"))
        .and(query_param("method", "catalog.container"))
        .respond_with(ReauthResponder {
            call_count: AtomicUsize::new(0),
        })
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    api.email = Some("user@example.com".to_string());
    api.password = Some("pass".to_string());
    api.access_token = Some("stale_token".to_string());

    let show = api.get_show_detail(1).await.unwrap();

    assert_eq!(show.container_id, 1);
    assert_eq!(api.access_token.as_deref(), Some(jwt_token.as_str()));
}

#[tokio::test]
async fn test_reauth_not_triggered_without_credentials() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api.aspx"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    // No credentials stored
    api.email = None;
    api.password = None;

    // get_show_detail checks status != 200, so this should return an error
    let err = api.get_show_detail(1).await.unwrap_err();
    // The 401 response is returned (not re-authed) and get_show_detail
    // returns UnexpectedResponse for non-200 status
    assert!(format!("{}", err).contains("HTTP 401"));
}

// ---------------------------------------------------------------------------
// get_all_artists tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_all_artists() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api.aspx"))
        .and(query_param("method", "catalog.artists"))
        .and(query_param("availType", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Response": {
                "artists": [
                    {"artistID": 62, "artistName": "Phish", "numShows": 1632},
                    {"artistID": 1045, "artistName": "Dead and Company", "numShows": 207},
                    {"artistID": 924, "artistName": "Bob Weir", "numShows": 15},
                ]
            }
        })))
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    let artists = api.get_all_artists().await;

    assert_eq!(artists.len(), 3);
    assert_eq!(artists[&62], "Phish");
    assert_eq!(artists[&1045], "Dead and Company");
    assert_eq!(artists[&924], "Bob Weir");
}

#[tokio::test]
async fn test_get_all_artists_coerces_string_ids() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api.aspx"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Response": {
                "artists": [
                    {"artistID": "62", "artistName": "Phish"},
                    {"artistID": "bad", "artistName": "Broken"},
                ]
            }
        })))
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    let artists = api.get_all_artists().await;

    assert_eq!(artists.len(), 1);
    assert_eq!(artists[&62], "Phish");
}

#[tokio::test]
async fn test_get_all_artists_skips_malformed_entries() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api.aspx"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "Response": {
                "artists": [
                    {"artistID": 62, "artistName": "Phish"},
                    {"artistName": "No ID"},
                    {"artistID": 100, "artistName": ""},
                    {"artistID": 101, "artistName": "  "},
                    {"artistID": 0, "artistName": "Zero ID"},
                ]
            }
        })))
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    let artists = api.get_all_artists().await;

    assert_eq!(artists.len(), 1);
    assert_eq!(artists[&62], "Phish");
}

#[tokio::test]
async fn test_get_all_artists_handles_missing_response_key() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api.aspx"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"error": "something"})))
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    let artists = api.get_all_artists().await;
    assert!(artists.is_empty());
}

#[tokio::test]
async fn test_get_all_artists_handles_empty_artists_list() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api.aspx"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"Response": {"artists": []}})),
        )
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    let artists = api.get_all_artists().await;
    assert!(artists.is_empty());
}

#[tokio::test]
async fn test_get_all_artists_handles_invalid_json() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api.aspx"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not valid json"))
        .mount(&server)
        .await;

    let mut api = NugsApi::new_for_test(&server.uri());
    let artists = api.get_all_artists().await;
    assert!(artists.is_empty());
}

// ---------------------------------------------------------------------------
// refresh_session tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_refresh_session_success() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let cache_path = tmp.path().join("session.json");

    let payload = json!({"legacy_token": "lt2", "legacy_uguid": "lg2", "sub": "u2"});
    let jwt_token = make_jwt(&payload);

    mount_auth_flow(&server, &jwt_token, "sub_fresh", "Fresh").await;

    let mut api = NugsApi::new_for_test(&server.uri());
    api.email = Some("u@test.com".to_string());
    api.password = Some("pw".to_string());
    api.session_cache_path = Some(cache_path.clone());

    let params = api.refresh_session().await.unwrap();

    assert_eq!(params.subscription_id, "sub_fresh");
    assert!(api.stream_params.is_some());
    assert_eq!(
        api.stream_params.as_ref().unwrap().subscription_id,
        "sub_fresh"
    );
    // Cache should have been saved
    assert!(cache_path.exists());
}

#[tokio::test]
async fn test_refresh_session_no_credentials() {
    let mut api = NugsApi::new_for_test("http://unused:9999");
    api.email = None;
    api.password = None;

    let err = api.refresh_session().await.unwrap_err();
    assert!(matches!(err, AuthError::NoCredentials));
}

// ---------------------------------------------------------------------------
// Retry exhaustion test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_retry_exhaustion_on_transport_error() {
    // Point at a port that's not listening — connection refused on every attempt.
    // Exercises request_with_retry()'s exponential backoff and RetryExhausted error.
    // Note: takes ~3s due to backoff sleeps (1s + 2s).
    let mut api = NugsApi::new_for_test("http://127.0.0.1:1");

    let err = api.get_show_detail(1).await.unwrap_err();
    assert!(
        matches!(err, ApiError::RetryExhausted { retries: 3, .. }),
        "Expected RetryExhausted, got: {:?}",
        err
    );
}
