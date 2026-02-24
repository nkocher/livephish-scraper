use std::path::Path;

use tempfile::tempdir;

use crate::transcode::*;

// ====================================
// compute_final_path
// ====================================

#[test]
fn test_compute_final_path_no_postprocess() {
    let path = Path::new("/tmp/track.m4a");
    assert_eq!(compute_final_path(path, "aac", "none", "none"), path);
}

#[test]
fn test_compute_final_path_non_aac_quality() {
    let path = Path::new("/tmp/track.flac");
    assert_eq!(compute_final_path(path, "flac", "flac", "none"), path);
}

#[test]
fn test_compute_final_path_flac_postprocess() {
    let path = Path::new("/tmp/track.m4a");
    let expected = Path::new("/tmp/track.flac");
    assert_eq!(compute_final_path(path, "aac", "flac", "none"), expected);
}

#[test]
fn test_compute_final_path_alac_keeps_m4a() {
    let path = Path::new("/tmp/track.m4a");
    assert_eq!(compute_final_path(path, "aac", "alac", "none"), path);
}

#[test]
fn test_compute_final_path_flac_to_alac() {
    let path = Path::new("/tmp/track.flac");
    let expected = Path::new("/tmp/track.m4a");
    assert_eq!(compute_final_path(path, "flac", "none", "alac"), expected);
}

#[test]
fn test_compute_final_path_flac_to_alac_not_flac_quality() {
    // flac_target set to alac but quality is aac — no conversion
    let path = Path::new("/tmp/track.m4a");
    assert_eq!(compute_final_path(path, "aac", "none", "alac"), path);
}

#[test]
fn test_compute_final_path_flac_to_aac() {
    let path = Path::new("/tmp/track.flac");
    let expected = Path::new("/tmp/track.m4a");
    assert_eq!(compute_final_path(path, "flac", "none", "aac"), expected);
}

#[test]
fn test_compute_final_path_flac_to_aac_not_flac_quality() {
    // flac_target set to aac but quality is aac — no conversion
    let path = Path::new("/tmp/track.m4a");
    assert_eq!(compute_final_path(path, "aac", "none", "aac"), path);
}

// ====================================
// effective_flac_target
// ====================================

#[test]
fn test_effective_flac_target_none_with_alac_format() {
    // Implicit ALAC fallback: flac_convert="none" + format=alac → "alac"
    assert_eq!(effective_flac_target("none", "alac"), "alac");
}

#[test]
fn test_effective_flac_target_none_with_flac_format() {
    // No conversion: flac_convert="none" + format=flac → "none"
    assert_eq!(effective_flac_target("none", "flac"), "none");
}

#[test]
fn test_effective_flac_target_explicit_aac_overrides_alac() {
    // Explicit "aac" takes priority over implicit ALAC
    assert_eq!(effective_flac_target("aac", "alac"), "aac");
}

#[test]
fn test_effective_flac_target_explicit_alac() {
    assert_eq!(effective_flac_target("alac", "flac"), "alac");
}

#[test]
fn test_effective_flac_target_explicit_aac_with_flac_format() {
    assert_eq!(effective_flac_target("aac", "flac"), "aac");
}

// ====================================
// postprocess_aac routing
// ====================================

#[test]
fn test_postprocess_aac_none_codec_passthrough() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("track.m4a");
    std::fs::write(&source, b"aac data").unwrap();

    let (result, err) = postprocess_aac(&source, "none");
    assert_eq!(result, source);
    assert!(err.is_none());
}

#[test]
fn test_postprocess_aac_non_m4a_passthrough() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("track.flac");
    std::fs::write(&source, b"flac data").unwrap();

    let (result, err) = postprocess_aac(&source, "flac");
    assert_eq!(result, source);
    assert!(err.is_none());
}

#[test]
fn test_postprocess_aac_unknown_codec_passthrough() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("track.m4a");
    std::fs::write(&source, b"aac data").unwrap();

    let (result, err) = postprocess_aac(&source, "mp3");
    assert_eq!(result, source);
    assert!(err.is_none());
}

// ====================================
// is_already_converted
// ====================================

#[test]
fn test_is_already_converted_unsupported_target() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("track.m4a");
    std::fs::write(&path, b"data").unwrap();

    assert!(!is_already_converted(&path, "flac"));
}

#[test]
fn test_is_already_converted_nonexistent_file() {
    let path = Path::new("/nonexistent/track.m4a");
    assert!(!is_already_converted(path, "alac"));
}

#[test]
fn test_is_already_converted_aac_nonexistent() {
    let path = Path::new("/nonexistent/track.m4a");
    assert!(!is_already_converted(path, "aac"));
}

// ====================================
// last_error_line (internal)
// ====================================

#[test]
fn test_last_error_line_extracts_meaningful_line() {
    let stderr =
        "frame=0 fps=0 q=0\nsize=0kB time=00:00:00\nInvalid argument\nConversion failed!\n";
    assert_eq!(last_error_line(stderr), "Invalid argument");
}

#[test]
fn test_last_error_line_empty_stderr() {
    assert_eq!(last_error_line(""), "(no output)");
}

#[test]
fn test_last_error_line_only_boilerplate() {
    let stderr = "frame=0 fps=0\nsize=0kB\nConversion failed!\n";
    assert_eq!(last_error_line(stderr), "(no output)");
}

#[test]
fn test_last_error_line_truncates_long_messages() {
    let long_msg = "E".repeat(300);
    let stderr = format!("{long_msg}\nConversion failed!\n");
    assert_eq!(last_error_line(&stderr).len(), 200);
}

// ====================================
// container_format
// ====================================

#[test]
fn test_container_format_flac() {
    assert_eq!(container_format("flac"), Some("flac"));
}

#[test]
fn test_container_format_alac() {
    assert_eq!(container_format("alac"), Some("ipod"));
}

#[test]
fn test_container_format_aac() {
    assert_eq!(container_format("aac"), Some("ipod"));
}

#[test]
fn test_container_format_unknown() {
    assert_eq!(container_format("mp3"), None);
}

// ====================================
// safe_unlink
// ====================================

#[test]
fn test_safe_unlink_existing_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("file.txt");
    std::fs::write(&path, b"data").unwrap();

    safe_unlink(&path);
    assert!(!path.exists());
}

#[test]
fn test_safe_unlink_nonexistent_file() {
    let path = Path::new("/tmp/nonexistent_test_file.txt");
    safe_unlink(path); // Should not panic
}

// ====================================
// which (PATH lookup)
// ====================================

#[test]
fn test_which_finds_existing_binary() {
    // "ls" should exist on all Unix systems
    #[cfg(unix)]
    {
        let result = which("ls");
        assert!(result.is_some());
    }
}

#[test]
fn test_which_returns_none_for_nonexistent() {
    let result = which("totally_nonexistent_binary_xyz123");
    assert!(result.is_none());
}

// ====================================
// check_ffmpeg
// ====================================

#[test]
fn test_check_ffmpeg_returns_bool() {
    // Just verify it doesn't panic — result depends on system
    let _has_ffmpeg = check_ffmpeg();
}

// ====================================
// find_binary
// ====================================

#[test]
fn test_find_binary_returns_none_for_nonexistent() {
    let result = find_binary("totally_nonexistent_binary_abc987");
    assert!(result.is_none());
}
