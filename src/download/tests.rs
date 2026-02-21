use std::path::Path;
use std::sync::atomic::AtomicBool;

use indicatif::ProgressBar;
use tempfile::tempdir;

use super::*;

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
