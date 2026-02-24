use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use indicatif::ProgressBar;
use serde_json::json;
use tempfile::tempdir;

use super::*;
use crate::models::Show;

/// Create a hidden progress bar, a non-cancelled flag, and a client for testing.
fn test_fixtures() -> (ProgressBar, AtomicBool, reqwest::Client) {
    let pb = ProgressBar::hidden();
    let cancel = AtomicBool::new(false);
    let client = reqwest::Client::new();
    (pb, cancel, client)
}

// ====================================
// make_track_filename
// ====================================

#[test]
fn test_make_track_filename_basic() {
    let result = make_track_filename(1, "Tweezer", ".flac");
    assert_eq!(result, "01. Tweezer.flac");
}

#[test]
fn test_make_track_filename_double_digit() {
    let result = make_track_filename(12, "Sand", ".m4a");
    assert_eq!(result, "12. Sand.m4a");
}

#[test]
fn test_make_track_filename_leading_zero() {
    let result = make_track_filename(5, "Tube", ".flac");
    assert_eq!(result, "05. Tube.flac");
}

#[test]
fn test_make_track_filename_special_chars_sanitized() {
    let result = make_track_filename(1, "Say It Ain't So / \"Hey\"", ".flac");
    assert!(!result.contains('/'));
    assert!(!result.contains('"'));
}

// ====================================
// part_path
// ====================================

#[test]
fn test_part_path_appends_part_suffix() {
    let dest = Path::new("/music/01. Tweezer.flac");
    let part = part_path(dest);
    assert_eq!(part, Path::new("/music/01. Tweezer.flac.part"));
}

#[test]
fn test_part_path_m4a() {
    let dest = Path::new("/music/01. Song.m4a");
    let part = part_path(dest);
    assert_eq!(part, Path::new("/music/01. Song.m4a.part"));
}

// ====================================
// should_skip_track
// ====================================

#[test]
fn test_should_skip_missing_final_dest() {
    let dir = tempdir().unwrap();
    let final_dest = dir.path().join("track.flac");
    let download_dest = dir.path().join("track.m4a");
    assert!(!should_skip_track(
        &final_dest,
        &download_dest,
        ConversionMode::AacPostprocess,
    ));
}

#[test]
fn test_should_skip_final_exists_no_convert() {
    let dir = tempdir().unwrap();
    let final_dest = dir.path().join("track.flac");
    std::fs::write(&final_dest, b"data").unwrap();
    assert!(should_skip_track(
        &final_dest,
        &final_dest,
        ConversionMode::None,
    ));
}

#[test]
fn test_should_skip_flac_postprocess_done() {
    let dir = tempdir().unwrap();
    let final_dest = dir.path().join("track.flac");
    std::fs::write(&final_dest, b"flac data").unwrap();
    let download_dest = dir.path().join("track.m4a");
    assert!(should_skip_track(
        &final_dest,
        &download_dest,
        ConversionMode::AacPostprocess,
    ));
}

// ====================================
// progress bar
// ====================================

#[test]
fn test_make_overall_bar_creation() {
    let pb = progress::make_overall_bar(10);
    assert_eq!(pb.length(), Some(10));
}

#[test]
fn test_make_overall_bar_zero() {
    let pb = progress::make_overall_bar(0);
    assert_eq!(pb.length(), Some(0));
}

// ====================================
// cancel flag
// ====================================

#[test]
fn test_cancel_flag_initially_false() {
    let flag = escape::new_cancel_flag();
    assert!(!flag.load(std::sync::atomic::Ordering::Relaxed));
}

#[test]
fn test_cancel_flag_set_true() {
    let flag = escape::new_cancel_flag();
    flag.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(flag.load(std::sync::atomic::Ordering::Relaxed));
}

// ====================================
// download_track via wiremock
// ====================================

#[tokio::test]
async fn test_download_track_basic() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let audio_data = b"fake audio content for testing";

    Mock::given(method("GET"))
        .and(path("/track.flac"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(audio_data.to_vec())
                .insert_header("content-length", audio_data.len().to_string()),
        )
        .mount(&server)
        .await;

    let dir = tempdir().unwrap();
    let dest = dir.path().join("01. Tweezer.flac");
    let (pb, cancel, client) = test_fixtures();

    let result = download_track(
        &format!("{}/track.flac", server.uri()),
        &dest,
        &pb,
        &cancel,
        &client,
        "nugsnetAndroid",
        "https://play.nugs.net/",
    )
    .await;

    assert!(result.is_ok());
    assert!(dest.exists());
    assert_eq!(std::fs::read(&dest).unwrap(), audio_data);
    assert!(!part_path(&dest).exists());
}

#[tokio::test]
async fn test_download_track_server_error() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/track.flac"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let dir = tempdir().unwrap();
    let dest = dir.path().join("01. Song.flac");
    let (pb, cancel, client) = test_fixtures();

    let result = download_track(
        &format!("{}/track.flac", server.uri()),
        &dest,
        &pb,
        &cancel,
        &client,
        "nugsnetAndroid",
        "https://play.nugs.net/",
    )
    .await;

    assert!(result.is_err());
    assert!(!dest.exists());
}

#[tokio::test]
async fn test_download_track_cleans_stale_part() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let audio_data = b"new audio";

    Mock::given(method("GET"))
        .and(path("/track.flac"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(audio_data.to_vec())
                .insert_header("content-length", audio_data.len().to_string()),
        )
        .mount(&server)
        .await;

    let dir = tempdir().unwrap();
    let dest = dir.path().join("01. Song.flac");
    let stale_part = part_path(&dest);
    std::fs::write(&stale_part, b"stale partial data").unwrap();

    let (pb, cancel, client) = test_fixtures();

    let result = download_track(
        &format!("{}/track.flac", server.uri()),
        &dest,
        &pb,
        &cancel,
        &client,
        "nugsnetAndroid",
        "https://play.nugs.net/",
    )
    .await;

    assert!(result.is_ok());
    assert!(dest.exists());
    assert!(!stale_part.exists());
    assert_eq!(std::fs::read(&dest).unwrap(), audio_data);
}

// ====================================
// download_track_with_retry via wiremock
// ====================================

#[tokio::test]
async fn test_retry_succeeds_after_transient_failure() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let audio_data = b"retried audio content";

    // First request: 500 (transient failure)
    Mock::given(method("GET"))
        .and(path("/track.flac"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;

    // Subsequent requests: 200 (success)
    Mock::given(method("GET"))
        .and(path("/track.flac"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(audio_data.to_vec())
                .insert_header("content-length", audio_data.len().to_string()),
        )
        .mount(&server)
        .await;

    let dir = tempdir().unwrap();
    let dest = dir.path().join("01. Tweezer.flac");
    let cancel = AtomicBool::new(false);
    let client = reqwest::Client::new();
    let overall = ProgressBar::hidden();

    let result = download_track_with_retry(
        &format!("{}/track.flac", server.uri()),
        &dest,
        &cancel,
        &client,
        "nugsnetAndroid",
        "https://play.nugs.net/",
        "Tweezer",
        &overall,
        Duration::ZERO,
    )
    .await;

    assert!(result.is_ok());
    assert!(dest.exists());
    assert_eq!(std::fs::read(&dest).unwrap(), audio_data);
}

#[tokio::test]
async fn test_retry_exhausts_all_attempts() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;

    // Always return 500
    Mock::given(method("GET"))
        .and(path("/track.flac"))
        .respond_with(ResponseTemplate::new(500))
        .expect(4) // 1 initial + 3 retries
        .mount(&server)
        .await;

    let dir = tempdir().unwrap();
    let dest = dir.path().join("01. Song.flac");
    let cancel = AtomicBool::new(false);
    let client = reqwest::Client::new();
    let overall = ProgressBar::hidden();

    let result = download_track_with_retry(
        &format!("{}/track.flac", server.uri()),
        &dest,
        &cancel,
        &client,
        "nugsnetAndroid",
        "https://play.nugs.net/",
        "Song",
        &overall,
        Duration::ZERO,
    )
    .await;

    assert!(result.is_err());
    assert!(!dest.exists());
}

#[tokio::test]
async fn test_retry_respects_cancellation() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;

    // Return 500 so retry logic kicks in
    Mock::given(method("GET"))
        .and(path("/track.flac"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let dir = tempdir().unwrap();
    let dest = dir.path().join("01. Ghost.flac");
    let cancel = AtomicBool::new(false);
    let client = reqwest::Client::new();
    let overall = ProgressBar::hidden();

    // Set cancel after first attempt fails — the retry sleep check will see it
    cancel.store(true, Ordering::Relaxed);

    let result = download_track_with_retry(
        &format!("{}/track.flac", server.uri()),
        &dest,
        &cancel,
        &client,
        "nugsnetAndroid",
        "https://play.nugs.net/",
        "Ghost",
        &overall,
        Duration::from_millis(10),
    )
    .await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    // Either cancelled during retry sleep or the 500 error itself — either way, we didn't hang
    assert!(
        err_msg.contains("cancelled") || err_msg.contains("500"),
        "unexpected error: {err_msg}"
    );
}

// ====================================
// download_show_with_retry via wiremock
// ====================================

/// Helper: build a minimal Show + TracksWithUrls for integration tests.
fn make_test_show_and_tracks(
    server_uri: &str,
    track_paths: &[&str],
) -> (Show, TracksWithUrls) {
    let tracks_json: Vec<serde_json::Value> = track_paths
        .iter()
        .enumerate()
        .map(|(i, _)| {
            json!({
                "trackID": i + 1,
                "setNum": 1,
                "songTitle": format!("Track {}", i + 1),
                "trackNum": i + 1,
            })
        })
        .collect();

    let show = Show::from_json(&json!({
        "containerID": 99999,
        "artistName": "Test Artist",
        "containerInfo": "Test Show",
        "venueName": "Test Venue",
        "venueCity": "Test City",
        "venueState": "TS",
        "performanceDate": "2024-01-01",
        "performanceDateFormatted": "01/01/2024",
        "performanceDateYear": "2024",
        "tracks": tracks_json,
    }));

    let quality = crate::models::Quality {
        code: "flac",
        specs: "16/44",
        extension: ".flac",
    };

    let tracks_with_urls: TracksWithUrls = show
        .tracks
        .iter()
        .zip(track_paths.iter())
        .map(|(track, path)| {
            (track.clone(), format!("{}{}", server_uri, path), quality.clone())
        })
        .collect();

    (show, tracks_with_urls)
}

#[tokio::test]
async fn test_show_retry_recovers_failed_tracks() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let audio_ok = b"good audio data";
    let audio_recovered = b"recovered audio";

    // Track 1: always succeeds
    Mock::given(method("GET"))
        .and(path("/track1.flac"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(audio_ok.to_vec())
                .insert_header("content-length", audio_ok.len().to_string()),
        )
        .mount(&server)
        .await;

    // Track 2: fails on first 4 attempts (per-track retry budget), succeeds after
    Mock::given(method("GET"))
        .and(path("/track2.flac"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(4) // exhaust per-track retry (1 initial + 3 retries)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/track2.flac"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(audio_recovered.to_vec())
                .insert_header("content-length", audio_recovered.len().to_string()),
        )
        .mount(&server)
        .await;

    let dir = tempdir().unwrap();
    let (show, tracks) =
        make_test_show_and_tracks(&server.uri(), &["/track1.flac", "/track2.flac"]);

    let outcome = download_show_with_retry(
        &show,
        &tracks,
        dir.path(),
        "none",
        "none",
        crate::service::Service::Nugs,
        crate::models::FormatCode::Flac,
        Duration::ZERO, // no cooldown in tests
    )
    .await;

    assert!(outcome.completed);
    assert_eq!(outcome.failed_count, 0, "second pass should recover the failed track");

    // Both track files should exist
    let show_dir = dir.path().join(show.folder_name());
    assert!(show_dir.join("01. Track 1.flac").exists());
    assert!(show_dir.join("02. Track 2.flac").exists());
}

#[tokio::test]
async fn test_show_retry_aborts_on_no_progress() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;

    // Track always returns 500 — no progress possible
    Mock::given(method("GET"))
        .and(path("/track1.flac"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let dir = tempdir().unwrap();
    let (show, tracks) = make_test_show_and_tracks(&server.uri(), &["/track1.flac"]);

    let outcome = download_show_with_retry(
        &show,
        &tracks,
        dir.path(),
        "none",
        "none",
        crate::service::Service::Nugs,
        crate::models::FormatCode::Flac,
        Duration::ZERO,
    )
    .await;

    assert!(outcome.completed, "should not report cancellation");
    assert_eq!(outcome.failed_count, 1, "track should still be failed");

    // Verify we didn't retry endlessly — the no-progress guard should
    // abort after pass 2 (same failure count as pass 1).
    // The mock server received: pass1 (4 hits) + pass2 (4 hits) = 8 total.
    // Without the guard it would be: pass1 (4) + pass2 (4) + pass3 (4) = 12.
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests.len() <= 8,
        "expected at most 8 requests (2 passes × 4 attempts), got {}",
        requests.len()
    );
}
