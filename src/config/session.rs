use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::models::StreamParams;
use crate::service::Service;

use super::paths::cache_dir;

const SESSION_TTL_SECS: f64 = 86400.0; // 24 hours

/// Cached session data (JSON, 24h TTL).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCache {
    pub access_token: String,
    pub legacy_token: String,
    pub legacy_uguid: String,
    pub user_id: String,
    pub stream_params: StreamParams,
    pub cached_at: f64,
}

/// Return the session cache file path for a specific service.
/// - `session_nugs.json` for Service::Nugs
/// - `session_livephish.json` for Service::LivePhish
pub fn session_cache_path_for(service: Service) -> PathBuf {
    let dir = cache_dir();
    match service {
        Service::Nugs => dir.join("session_nugs.json"),
        Service::LivePhish => dir.join("session_livephish.json"),
    }
}

/// Default session cache file path (nugs, for backward compat).
fn session_cache_file() -> PathBuf {
    // Prefer the new per-service file; migrate from the legacy session.json if needed.
    let new_path = session_cache_path_for(Service::Nugs);
    let legacy_path = cache_dir().join("session.json");

    if !new_path.exists() && legacy_path.exists() {
        // Migrate: copy legacy file to new location and leave the old one in place
        // (removal on next successful auth is fine; don't break currently-valid sessions).
        if let Ok(contents) = fs::read_to_string(&legacy_path) {
            if let Some(parent) = new_path.parent() {
                fs::create_dir_all(parent).ok();
            }
            fs::write(&new_path, &contents).ok();
        }
    }

    new_path
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Load cached session tokens if still valid (24h TTL).
/// Defaults to the nugs session file, with automatic migration from the legacy `session.json`.
pub fn load_session_cache() -> Option<SessionCache> {
    load_session_cache_from(&session_cache_file())
}

/// Load the session cache for a specific service.
#[allow(dead_code)] // Phase 5: used by multi-service router
pub fn load_session_cache_for(service: Service) -> Option<SessionCache> {
    load_session_cache_from(&session_cache_path_for(service))
}

/// Load from a specific file (for testing).
pub fn load_session_cache_from(path: &PathBuf) -> Option<SessionCache> {
    let contents = fs::read_to_string(path).ok()?;
    let cache: SessionCache = serde_json::from_str(&contents).ok()?;

    if now_secs() - cache.cached_at > SESSION_TTL_SECS {
        return None;
    }

    Some(cache)
}

/// Save session tokens to cache file with 0600 permissions (nugs service, backward compat).
pub fn save_session_cache(
    access_token: &str,
    legacy_token: &str,
    legacy_uguid: &str,
    user_id: &str,
    stream_params: &StreamParams,
) {
    save_session_cache_to(
        &session_cache_file(),
        access_token,
        legacy_token,
        legacy_uguid,
        user_id,
        stream_params,
    );
}

/// Save session tokens for a specific service.
#[allow(dead_code)] // Phase 5: used by multi-service router
pub fn save_session_cache_for(
    service: Service,
    access_token: &str,
    legacy_token: &str,
    legacy_uguid: &str,
    user_id: &str,
    stream_params: &StreamParams,
) {
    save_session_cache_to(
        &session_cache_path_for(service),
        access_token,
        legacy_token,
        legacy_uguid,
        user_id,
        stream_params,
    );
}

/// Save to a specific file (for testing).
pub fn save_session_cache_to(
    path: &PathBuf,
    access_token: &str,
    legacy_token: &str,
    legacy_uguid: &str,
    user_id: &str,
    stream_params: &StreamParams,
) {
    let cache = SessionCache {
        access_token: access_token.to_string(),
        legacy_token: legacy_token.to_string(),
        legacy_uguid: legacy_uguid.to_string(),
        user_id: user_id.to_string(),
        stream_params: stream_params.clone(),
        cached_at: now_secs(),
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }

    if let Ok(json) = serde_json::to_string(&cache) {
        if fs::write(path, &json).is_ok() {
            // Set 0600 permissions (Unix only)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o600)).ok();
            }
        }
    }
}

/// Remove cached session (nugs, backward compat).
#[allow(dead_code)] // Phase 4: `nugs refresh` subcommand
pub fn clear_session_cache() {
    clear_session_cache_at(&session_cache_file());
}

/// Remove cached session for a specific service.
#[allow(dead_code)]
pub fn clear_session_cache_for(service: Service) {
    clear_session_cache_at(&session_cache_path_for(service));
}

/// Clear a specific cache file (for testing).
#[allow(dead_code)] // Used by tests + clear_session_cache
pub fn clear_session_cache_at(path: &PathBuf) {
    if path.exists() {
        fs::remove_file(path).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_stream_params() -> StreamParams {
        StreamParams {
            subscription_id: "sub_123".to_string(),
            sub_costplan_id_access_list: "1,2,3".to_string(),
            user_id: "456".to_string(),
            start_stamp: "1700000000".to_string(),
            end_stamp: "1800000000".to_string(),
        }
    }

    #[test]
    fn test_save_and_load_session_cache() {
        let tmp = tempdir().unwrap();
        let cache_file = tmp.path().join("session.json");

        let params = test_stream_params();
        save_session_cache_to(
            &cache_file,
            "test_access",
            "test_legacy",
            "test_uguid",
            "test_uid",
            &params,
        );

        assert!(cache_file.exists());

        let cached = load_session_cache_from(&cache_file);
        assert!(cached.is_some());
        let cached = cached.unwrap();
        assert_eq!(cached.access_token, "test_access");
        assert_eq!(cached.legacy_token, "test_legacy");
        assert_eq!(cached.legacy_uguid, "test_uguid");
        assert_eq!(cached.user_id, "test_uid");
        assert_eq!(cached.stream_params.subscription_id, "sub_123");
    }

    #[test]
    fn test_session_cache_expiry() {
        let tmp = tempdir().unwrap();
        let cache_file = tmp.path().join("session.json");

        let params = test_stream_params();
        save_session_cache_to(
            &cache_file,
            "old_token",
            "old_legacy",
            "old_uguid",
            "old_uid",
            &params,
        );

        // Manually backdate the cached_at to 25 hours ago
        let mut data: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&cache_file).unwrap()).unwrap();
        data["cached_at"] = serde_json::Value::from(now_secs() - (25.0 * 3600.0));
        fs::write(&cache_file, serde_json::to_string(&data).unwrap()).unwrap();

        assert!(load_session_cache_from(&cache_file).is_none());
    }

    #[test]
    fn test_session_cache_missing() {
        let tmp = tempdir().unwrap();
        let cache_file = tmp.path().join("session.json");
        assert!(load_session_cache_from(&cache_file).is_none());
    }

    #[test]
    fn test_clear_session_cache() {
        let tmp = tempdir().unwrap();
        let cache_file = tmp.path().join("session.json");

        let params = test_stream_params();
        save_session_cache_to(&cache_file, "tok", "leg", "guid", "uid", &params);
        assert!(cache_file.exists());

        clear_session_cache_at(&cache_file);
        assert!(!cache_file.exists());
    }

    #[test]
    fn test_clear_session_cache_noop_if_missing() {
        let tmp = tempdir().unwrap();
        let cache_file = tmp.path().join("session.json");
        clear_session_cache_at(&cache_file); // Should not panic
    }

    #[test]
    fn test_session_cache_corrupt_json() {
        let tmp = tempdir().unwrap();
        let cache_file = tmp.path().join("session.json");
        fs::write(&cache_file, "not valid json{{{").unwrap();
        assert!(load_session_cache_from(&cache_file).is_none());
    }

    #[test]
    #[cfg(unix)]
    fn test_session_cache_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempdir().unwrap();
        let cache_file = tmp.path().join("session.json");

        let params = test_stream_params();
        save_session_cache_to(&cache_file, "tok", "leg", "guid", "uid", &params);

        let mode = fs::metadata(&cache_file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn test_session_cache_path_for_service() {
        // Just verify the paths contain the expected filenames.
        // (Actual cache_dir() is the system cache dir — we only check the filename.)
        let nugs_path = session_cache_path_for(Service::Nugs);
        let lp_path = session_cache_path_for(Service::LivePhish);

        assert_eq!(nugs_path.file_name().unwrap(), "session_nugs.json");
        assert_eq!(lp_path.file_name().unwrap(), "session_livephish.json");
    }
}
