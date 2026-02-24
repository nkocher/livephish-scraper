use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use tracing::debug;

/// Find an ffmpeg/ffprobe binary, preferring siblings of the current exe.
///
/// For release builds (next to the app binary), checks the exe's directory first.
/// Falls back to PATH lookup via `which`.
pub fn find_binary(name: &str) -> Option<PathBuf> {
    // Check next to the current executable (bundled in release zip)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join(name);
            if sibling.is_file() {
                return Some(sibling);
            }
            // Windows: bare name may lack .exe extension
            #[cfg(target_os = "windows")]
            {
                let sibling_exe = dir.join(format!("{name}.exe"));
                if sibling_exe.is_file() {
                    return Some(sibling_exe);
                }
            }
        }
    }
    which(name)
}

/// Check if ffmpeg is available.
pub fn check_ffmpeg() -> bool {
    find_binary("ffmpeg").is_some()
}

/// Detect audio codec of a file via ffprobe.
///
/// Returns codec name (e.g. "aac", "alac", "flac") or None if
/// ffprobe is unavailable or detection fails.
pub fn detect_codec(path: &Path) -> Option<String> {
    let ffprobe = find_binary("ffprobe")?;
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "quiet",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .ok()?;

    if output.status.success() {
        let codec = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if codec.is_empty() {
            None
        } else {
            Some(codec)
        }
    } else {
        None
    }
}

/// Check if an .m4a file was already converted to the target codec.
///
/// Meaningful for codecs that share the .m4a extension (AAC and ALAC).
/// Falls back to true (assume done) if ffprobe is unavailable.
pub fn is_already_converted(path: &Path, target_codec: &str) -> bool {
    if !matches!(target_codec, "alac" | "aac") || !path.exists() {
        return false;
    }
    match detect_codec(path) {
        Some(codec) => codec == target_codec,
        None => true, // Can't detect — assume done
    }
}

/// Resolve the effective FLAC conversion target.
///
/// Priority: explicit `flac_convert` setting, then implicit ALAC when
/// the user requested ALAC but the API serves FLAC, otherwise no conversion.
pub fn effective_flac_target<'a>(flac_convert: &'a str, requested_format: &str) -> &'a str {
    match flac_convert {
        "none" if requested_format == "alac" => "alac",
        "none" => "none",
        other => other,
    }
}

/// Compute the file path after optional postprocessing.
///
/// - AAC→FLAC postprocess: .m4a becomes .flac
/// - AAC→ALAC postprocess: .m4a stays .m4a (same extension, different codec)
/// - FLAC→ALAC or FLAC→AAC: .flac becomes .m4a
pub fn compute_final_path(
    download_path: &Path,
    quality_code: &str,
    postprocess_codec: &str,
    flac_target: &str,
) -> PathBuf {
    if quality_code == "aac" && postprocess_codec != "none" {
        if postprocess_codec == "flac" {
            return download_path.with_extension("flac");
        }
        return download_path.to_path_buf(); // ALAC keeps .m4a
    }
    if quality_code == "flac" && matches!(flac_target, "alac" | "aac") {
        return download_path.with_extension("m4a");
    }
    download_path.to_path_buf()
}

/// Convert an AAC .m4a file to the target codec.
///
/// Returns `(final_path, error)` — error is None on success, a message string
/// on failure. On failure the original source is preserved and returned.
pub fn postprocess_aac(source: &Path, target_codec: &str) -> (PathBuf, Option<String>) {
    if target_codec == "none" {
        return (source.to_path_buf(), None);
    }
    let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !ext.eq_ignore_ascii_case("m4a") {
        return (source.to_path_buf(), None);
    }

    match target_codec {
        "flac" => {
            let dest = source.with_extension("flac");
            transcode(source, &dest, "flac", ".flac.part", None)
        }
        "alac" => transcode(source, source, "alac", ".m4a.converting", None),
        _ => (source.to_path_buf(), None),
    }
}

/// Convert a FLAC file to ALAC (.m4a).
///
/// Lossless-to-lossless conversion, used when user requests ALAC but API serves FLAC.
/// Returns `(final_path, error)` — error is None on success.
pub fn postprocess_flac_to_alac(source: &Path) -> (PathBuf, Option<String>) {
    let dest = source.with_extension("m4a");
    transcode(source, &dest, "alac", ".m4a.converting", None)
}

/// Convert a FLAC file to AAC 256 kbps (.m4a).
///
/// Lossy conversion for smaller file sizes. Source .flac is deleted on success.
/// Returns `(final_path, error)` — error is None on success.
pub fn postprocess_flac_to_aac(source: &Path) -> (PathBuf, Option<String>) {
    let dest = source.with_extension("m4a");
    transcode(source, &dest, "aac", ".m4a.converting", Some("256k"))
}

/// Container format flag for ffmpeg's -f option.
fn container_format(codec: &str) -> Option<&'static str> {
    match codec {
        "flac" => Some("flac"),
        "alac" | "aac" => Some("ipod"),
        _ => None,
    }
}

/// Extract the last meaningful error line from ffmpeg stderr.
fn last_error_line(stderr: &str) -> String {
    for line in stderr.trim().lines().rev() {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();
        if lower.is_empty() || lower == "conversion failed!" {
            continue;
        }
        if trimmed.starts_with("frame=") || trimmed.starts_with("size=") {
            continue;
        }
        let truncated: String = trimmed.chars().take(200).collect();
        return truncated;
    }
    "(no output)".to_string()
}

/// Run ffmpeg transcoding with temp-file safety.
///
/// Writes to a temp file, then atomically renames on success.
/// On failure, cleans up temp and returns the original source path.
fn transcode(
    source: &Path,
    dest: &Path,
    codec: &str,
    temp_suffix: &str,
    bitrate: Option<&str>,
) -> (PathBuf, Option<String>) {
    let temp = source.with_extension(temp_suffix.strip_prefix('.').unwrap_or(temp_suffix));
    safe_unlink(&temp);

    let ffmpeg_bin = match find_binary("ffmpeg") {
        Some(bin) => bin,
        None => return (source.to_path_buf(), Some("ffmpeg not found".to_string())),
    };

    let mut cmd = Command::new(&ffmpeg_bin);
    cmd.args(["-y", "-i"])
        .arg(source)
        .args(["-c:a", codec]);

    if let Some(br) = bitrate {
        cmd.args(["-b:a", br]);
    }

    cmd.args(["-c:v", "copy", "-map_metadata", "0"]);

    if let Some(fmt) = container_format(codec) {
        cmd.args(["-f", fmt]);
    }
    cmd.arg(&temp);

    // Run with timeout (300s like Python)
    let child = match cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            safe_unlink(&temp);
            return (
                source.to_path_buf(),
                Some(format!("failed to spawn ffmpeg: {e}")),
            );
        }
    };

    match wait_with_timeout(child, Duration::from_secs(300)) {
        Ok(output) => {
            if !output.status.success() {
                let err = last_error_line(&String::from_utf8_lossy(&output.stderr));
                debug!("ffmpeg {codec} failed for {}: {err}", source.display());
                safe_unlink(&temp);
                return (source.to_path_buf(), Some(err));
            }

            // Validate output before overwriting
            if !temp.exists() || temp.metadata().map_or(true, |m| m.len() == 0) {
                safe_unlink(&temp);
                return (
                    source.to_path_buf(),
                    Some("ffmpeg produced empty output".to_string()),
                );
            }

            if dest == source {
                // ALAC: overwrite source (same extension, different codec).
                // Uses replace_file for cross-platform atomic overwrite.
                if let Err(e) = replace_file(&temp, dest) {
                    safe_unlink(&temp);
                    return (source.to_path_buf(), Some(e.to_string()));
                }
            } else {
                // FLAC: rename to new extension, remove original .m4a.
                if let Err(e) = std::fs::rename(&temp, dest) {
                    safe_unlink(&temp);
                    return (source.to_path_buf(), Some(e.to_string()));
                }
                safe_unlink(source);
            }

            (dest.to_path_buf(), None)
        }
        Err(TimeoutError::Timeout) => {
            safe_unlink(&temp);
            (source.to_path_buf(), Some("ffmpeg timed out".to_string()))
        }
        Err(TimeoutError::Io(e)) => {
            safe_unlink(&temp);
            (source.to_path_buf(), Some(e.to_string()))
        }
    }
}

enum TimeoutError {
    Timeout,
    Io(std::io::Error),
}

/// Wait for a child process with a timeout. Kills the process on timeout.
fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
) -> Result<std::process::Output, TimeoutError> {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = child
                    .stdout
                    .map(|mut s| {
                        let mut buf = Vec::new();
                        std::io::Read::read_to_end(&mut s, &mut buf).ok();
                        buf
                    })
                    .unwrap_or_default();
                let stderr = child
                    .stderr
                    .map(|mut s| {
                        let mut buf = Vec::new();
                        std::io::Read::read_to_end(&mut s, &mut buf).ok();
                        buf
                    })
                    .unwrap_or_default();
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(TimeoutError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(TimeoutError::Io(e)),
        }
    }
}

/// Cross-platform file replace (rename with overwrite).
///
/// On POSIX, `rename()` atomically overwrites the destination.
/// On Windows, `rename()` fails when the destination exists, so we remove first.
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) if to.exists() => {
            std::fs::remove_file(to)?;
            std::fs::rename(from, to)
        }
        Err(e) => Err(e),
    }
}

/// Remove a file if it exists, ignoring errors.
fn safe_unlink(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Simple PATH lookup (platform-aware).
fn which(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(target_os = "windows")]
        {
            let candidate_exe = dir.join(format!("{name}.exe"));
            if candidate_exe.is_file() {
                return Some(candidate_exe);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests;
