use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::paths::cache_dir;

const RECENTS_CAP: usize = 50;

fn recents_file() -> PathBuf {
    cache_dir().join("recents.json")
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Load recents cache: {artist_id: unix_timestamp}.
/// Returns empty map on missing/corrupt file.
pub fn load_recents() -> HashMap<i64, f64> {
    load_recents_from(&recents_file())
}

/// Load from a specific file (for testing).
pub fn load_recents_from(path: &PathBuf) -> HashMap<i64, f64> {
    let contents = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };

    // JSON stores keys as strings — parse them as i64
    let raw: HashMap<String, f64> = match serde_json::from_str(&contents) {
        Ok(m) => m,
        Err(_) => return HashMap::new(),
    };

    raw.into_iter()
        .filter_map(|(k, v)| k.parse::<i64>().ok().map(|id| (id, v)))
        .collect()
}

/// Record an artist access timestamp. Caps at RECENTS_CAP oldest-pruned.
pub fn record_recent(artist_id: i64) {
    record_recent_to(&recents_file(), artist_id);
}

/// Record to a specific file (for testing).
pub fn record_recent_to(path: &PathBuf, artist_id: i64) {
    let mut recents = load_recents_from(path);
    recents.insert(artist_id, now_secs());

    // Prune to cap, keeping most recent
    if recents.len() > RECENTS_CAP {
        let mut entries: Vec<(i64, f64)> = recents.into_iter().collect();
        entries.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        entries.truncate(RECENTS_CAP);
        recents = entries.into_iter().collect();
    }

    // Serialize with string keys (JSON compat with Python version)
    let serializable: HashMap<String, f64> =
        recents.iter().map(|(k, v)| (k.to_string(), *v)).collect();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }

    if let Ok(json) = serde_json::to_string(&serializable) {
        fs::write(path, json).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_load_recents_empty() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("recents.json");
        assert_eq!(load_recents_from(&path), HashMap::new());
    }

    #[test]
    fn test_load_recents_corrupt() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("recents.json");
        fs::write(&path, "not valid json{{{").unwrap();
        assert_eq!(load_recents_from(&path), HashMap::new());
    }

    #[test]
    fn test_load_recents_roundtrip() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("recents.json");
        fs::write(
            &path,
            serde_json::to_string(&HashMap::from([
                ("196".to_string(), 1000.5),
                ("82".to_string(), 2000.0),
            ]))
            .unwrap(),
        )
        .unwrap();

        let result = load_recents_from(&path);
        assert_eq!(result.len(), 2);
        assert_eq!(result[&196], 1000.5);
        assert_eq!(result[&82], 2000.0);
    }

    #[test]
    fn test_record_recent_creates_file() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("cache").join("recents.json");

        record_recent_to(&path, 196);

        assert!(path.exists());
        let data = load_recents_from(&path);
        assert!(data.contains_key(&196));
    }

    #[test]
    fn test_record_recent_updates_timestamp() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("recents.json");

        record_recent_to(&path, 196);
        let first = load_recents_from(&path)[&196];
        std::thread::sleep(std::time::Duration::from_millis(10));
        record_recent_to(&path, 196);
        let second = load_recents_from(&path)[&196];
        assert!(second > first);
    }

    #[test]
    fn test_record_recent_cap() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("recents.json");

        // Write 50 existing entries with ascending timestamps
        let existing: HashMap<String, f64> = (1..=50).map(|i| (i.to_string(), i as f64)).collect();
        fs::write(&path, serde_json::to_string(&existing).unwrap()).unwrap();

        // Record one more — should evict artist_id=1 (timestamp=1.0, oldest)
        record_recent_to(&path, 999);

        let result = load_recents_from(&path);
        assert_eq!(result.len(), 50);
        assert!(!result.contains_key(&1)); // oldest pruned
        assert!(result.contains_key(&999));
    }

    #[test]
    fn test_record_recent_canonicalizes_keys() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("recents.json");

        // Write with string keys
        fs::write(
            &path,
            serde_json::to_string(&HashMap::from([("196".to_string(), 1000.0)])).unwrap(),
        )
        .unwrap();

        // Record same artist — should update, not duplicate
        record_recent_to(&path, 196);
        let result = load_recents_from(&path);
        assert_eq!(result.len(), 1);
        assert!(result.contains_key(&196));
        assert!(result[&196] > 1000.0);
    }
}
