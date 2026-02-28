use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Bidirectional mapping between Google Drive file IDs (String) and synthetic
/// negative i64 IDs. Negative namespace prevents collisions with real nugs.net
/// container_ids which are always positive.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BmanIdMap {
    i64_to_drive: HashMap<i64, String>,
    drive_to_i64: HashMap<String, i64>,
}

impl BmanIdMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Compute a negative synthetic ID from a Google Drive file ID.
    pub fn synthetic_id(drive_id: &str) -> i64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        drive_id.hash(&mut hasher);
        let hash = hasher.finish();
        -((hash & (i64::MAX as u64)) as i64).max(1)
    }

    /// Insert a drive_id and return its synthetic i64 ID.
    /// Handles hash collisions by re-hashing with a suffix.
    pub fn insert(&mut self, drive_id: &str) -> i64 {
        if let Some(&existing) = self.drive_to_i64.get(drive_id) {
            return existing;
        }

        let mut id = Self::synthetic_id(drive_id);
        let mut attempt = 0u32;
        while let Some(existing_drive) = self.i64_to_drive.get(&id) {
            if existing_drive == drive_id {
                return id;
            }
            // Hash collision — re-hash with suffix
            attempt += 1;
            tracing::warn!(
                "BmanIdMap hash collision: {} and {} both map to {}. Re-hashing (attempt {})",
                drive_id, existing_drive, id, attempt
            );
            let suffixed = format!("{drive_id}:{attempt}");
            id = Self::synthetic_id(&suffixed);
        }

        self.i64_to_drive.insert(id, drive_id.to_string());
        self.drive_to_i64.insert(drive_id.to_string(), id);
        id
    }

    pub fn get_drive_id(&self, synthetic: i64) -> Option<&str> {
        self.i64_to_drive.get(&synthetic).map(|s| s.as_str())
    }

    #[allow(dead_code)] // used in tests and future CLI lookup
    pub fn get_synthetic_id(&self, drive_id: &str) -> Option<i64> {
        self.drive_to_i64.get(drive_id).copied()
    }

    #[allow(dead_code)] // used in tests
    pub fn len(&self) -> usize {
        self.i64_to_drive.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.i64_to_drive.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_synthetic_id_is_negative() {
        let id = BmanIdMap::synthetic_id("1aK32Uxa56LK2DsQ4ZmgugA9FhoMrlZm7");
        assert!(id < 0, "Synthetic ID should be negative, got {}", id);
    }

    #[test]
    fn test_insert_and_roundtrip() {
        let mut map = BmanIdMap::new();
        let drive_id = "abcdef123456";
        let synthetic = map.insert(drive_id);

        assert!(synthetic < 0);
        assert_eq!(map.get_drive_id(synthetic), Some(drive_id));
        assert_eq!(map.get_synthetic_id(drive_id), Some(synthetic));
    }

    #[test]
    fn test_insert_idempotent() {
        let mut map = BmanIdMap::new();
        let id1 = map.insert("abc");
        let id2 = map.insert("abc");
        assert_eq!(id1, id2);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_different_drive_ids_get_different_synthetics() {
        let mut map = BmanIdMap::new();
        let id1 = map.insert("folder_a");
        let id2 = map.insert("folder_b");
        assert_ne!(id1, id2);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn test_serde_roundtrip() {
        let mut map = BmanIdMap::new();
        map.insert("drive_id_1");
        map.insert("drive_id_2");

        let json = serde_json::to_string(&map).unwrap();
        let loaded: BmanIdMap = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded.get_drive_id(map.get_synthetic_id("drive_id_1").unwrap()),
            Some("drive_id_1")
        );
    }
}
