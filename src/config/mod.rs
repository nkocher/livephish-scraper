pub mod credentials;
pub mod paths;
pub mod recents;
pub mod session;

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::service::Service;

pub use paths::{cache_dir, config_dir, expand_tilde};

/// Keyring service name for the nugs service (shared with Python version for credential migration).
pub const KEYRING_SERVICE: &str = "nugs";

/// Per-service email configuration section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServiceSection {
    #[serde(default)]
    pub email: String,
}

/// User configuration (persisted as TOML).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Legacy top-level email field (read-only for migration, never written back).
    #[serde(default, skip_serializing)]
    pub email: String,

    #[serde(default = "default_format")]
    pub format: String,

    #[serde(default = "default_output_dir")]
    pub output_dir: String,

    #[serde(default = "default_none_string")]
    pub postprocess_codec: String,

    #[serde(default = "default_none_string")]
    pub flac_convert: String,

    #[serde(default)]
    pub nugs: ServiceSection,

    #[serde(default)]
    pub livephish: ServiceSection,
}

fn default_format() -> String {
    "flac".to_string()
}

fn default_output_dir() -> String {
    "~/Music/Nugs".to_string()
}

fn default_none_string() -> String {
    "none".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Config {
            email: String::new(),
            format: "flac".to_string(),
            output_dir: "~/Music/Nugs".to_string(),
            postprocess_codec: "none".to_string(),
            flac_convert: "none".to_string(),
            nugs: ServiceSection::default(),
            livephish: ServiceSection::default(),
        }
    }
}

impl Config {
    /// Validate and normalize config fields.
    fn normalize(&mut self) {
        if !["none", "flac", "alac"].contains(&self.postprocess_codec.as_str()) {
            self.postprocess_codec = "none".to_string();
        }
        if !["none", "alac", "aac"].contains(&self.flac_convert.as_str()) {
            self.flac_convert = "none".to_string();
        }
    }

    /// Return the configured email for the given service.
    pub fn email_for(&self, service: Service) -> &str {
        match service {
            Service::Nugs => &self.nugs.email,
            Service::LivePhish => &self.livephish.email,
        }
    }
}

/// Load configuration from TOML file.
/// Creates default config and necessary directories if they don't exist.
pub fn load_config() -> Config {
    let config_dir = config_dir();
    let cache_dir = cache_dir();
    let config_file = config_dir.join("config.toml");

    fs::create_dir_all(&config_dir).ok();
    fs::create_dir_all(&cache_dir).ok();

    if config_file.exists() {
        if let Ok(contents) = fs::read_to_string(&config_file) {
            if let Ok(mut config) = toml::from_str::<Config>(&contents) {
                config.normalize();
                // Migrate legacy top-level email to [nugs] section.
                if !config.email.is_empty() && config.nugs.email.is_empty() {
                    config.nugs.email = config.email.clone();
                    config.email = String::new();
                    save_config(&config);
                }
                return config;
            }
        }
    }

    let config = Config::default();
    save_config(&config);
    config
}

/// Load config from a specific directory (for testing).
#[allow(dead_code)] // Used by tests
pub fn load_config_from(config_dir: &PathBuf, cache_dir: &PathBuf) -> Config {
    let config_file = config_dir.join("config.toml");

    fs::create_dir_all(config_dir).ok();
    fs::create_dir_all(cache_dir).ok();

    if config_file.exists() {
        if let Ok(contents) = fs::read_to_string(&config_file) {
            if let Ok(mut config) = toml::from_str::<Config>(&contents) {
                config.normalize();
                // Migrate legacy top-level email to [nugs] section.
                if !config.email.is_empty() && config.nugs.email.is_empty() {
                    config.nugs.email = config.email.clone();
                    config.email = String::new();
                    save_config_to(&config, config_dir);
                }
                return config;
            }
        }
    }

    let config = Config::default();
    save_config_to(&config, config_dir);
    config
}

/// Save configuration to TOML file.
pub fn save_config(config: &Config) {
    let config_dir = config_dir();
    save_config_to(config, &config_dir);
}

/// Save config to a specific directory (for testing).
pub fn save_config_to(config: &Config, config_dir: &PathBuf) {
    let config_file = config_dir.join("config.toml");
    fs::create_dir_all(config_dir).ok();

    if let Ok(contents) = toml::to_string_pretty(config) {
        fs::write(config_file, contents).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_config_defaults() {
        let cfg = Config::default();
        assert_eq!(cfg.email, "");
        assert_eq!(cfg.format, "flac");
        assert_eq!(cfg.output_dir, "~/Music/Nugs");
        assert_eq!(cfg.postprocess_codec, "none");
        assert_eq!(cfg.flac_convert, "none");
        assert_eq!(cfg.nugs.email, "");
        assert_eq!(cfg.livephish.email, "");
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let tmp = tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        let cache_dir = tmp.path().join("cache");

        let test_config = Config {
            email: String::new(),
            format: "alac".to_string(),
            output_dir: "~/Downloads/Music".to_string(),
            postprocess_codec: "none".to_string(),
            flac_convert: "none".to_string(),
            nugs: ServiceSection {
                email: "test@example.com".to_string(),
            },
            livephish: ServiceSection {
                email: "lp@example.com".to_string(),
            },
        };

        save_config_to(&test_config, &config_dir);
        let loaded = load_config_from(&config_dir, &cache_dir);

        assert_eq!(loaded.nugs.email, "test@example.com");
        assert_eq!(loaded.livephish.email, "lp@example.com");
        assert_eq!(loaded.format, "alac");
        assert_eq!(loaded.output_dir, "~/Downloads/Music");
    }

    #[test]
    fn test_load_config_creates_directories() {
        let tmp = tempdir().unwrap();
        let config_dir = tmp.path().join("new_config");
        let cache_dir = tmp.path().join("new_cache");

        assert!(!config_dir.exists());
        assert!(!cache_dir.exists());

        let cfg = load_config_from(&config_dir, &cache_dir);

        assert!(config_dir.exists());
        assert!(cache_dir.exists());
        assert!(config_dir.join("config.toml").exists());
        assert_eq!(cfg.nugs.email, "");
    }

    #[test]
    fn test_load_config_with_existing_file() {
        let tmp = tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        let cache_dir = tmp.path().join("cache");

        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.toml"),
            r#"
format = "aac"
output_dir = "~/Music/Test"
postprocess_codec = "none"

[nugs]
email = "manual@example.com"
"#,
        )
        .unwrap();

        let cfg = load_config_from(&config_dir, &cache_dir);

        assert_eq!(cfg.nugs.email, "manual@example.com");
        assert_eq!(cfg.format, "aac");
        assert_eq!(cfg.output_dir, "~/Music/Test");
    }

    #[test]
    fn test_legacy_email_migration() {
        let tmp = tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        let cache_dir = tmp.path().join("cache");

        fs::create_dir_all(&config_dir).unwrap();
        // Old-style config with top-level email
        fs::write(
            config_dir.join("config.toml"),
            r#"
email = "legacy@example.com"
format = "flac"
output_dir = "~/Music/Nugs"
postprocess_codec = "none"
"#,
        )
        .unwrap();

        let cfg = load_config_from(&config_dir, &cache_dir);

        // Legacy email should be migrated to nugs section
        assert_eq!(cfg.nugs.email, "legacy@example.com");
        // Top-level email field should be cleared after migration
        assert_eq!(cfg.email, "");

        // Reload to confirm migration was persisted
        let cfg2 = load_config_from(&config_dir, &cache_dir);
        assert_eq!(cfg2.nugs.email, "legacy@example.com");
    }

    #[test]
    fn test_email_for_service() {
        let cfg = Config {
            nugs: ServiceSection {
                email: "nugs@example.com".to_string(),
            },
            livephish: ServiceSection {
                email: "lp@example.com".to_string(),
            },
            ..Config::default()
        };
        assert_eq!(cfg.email_for(Service::Nugs), "nugs@example.com");
        assert_eq!(cfg.email_for(Service::LivePhish), "lp@example.com");
    }

    #[test]
    fn test_save_config_creates_directory() {
        let tmp = tempdir().unwrap();
        let config_dir = tmp.path().join("new_dir");

        assert!(!config_dir.exists());

        let cfg = Config {
            nugs: ServiceSection {
                email: "test@example.com".to_string(),
            },
            ..Config::default()
        };
        save_config_to(&cfg, &config_dir);

        assert!(config_dir.exists());
        assert!(config_dir.join("config.toml").exists());
    }

    #[test]
    fn test_valid_formats() {
        for fmt in ["flac", "alac", "aac", "mqa", "360"] {
            let cfg = Config {
                format: fmt.to_string(),
                ..Config::default()
            };
            assert_eq!(cfg.format, fmt);
        }
    }

    #[test]
    fn test_format_persists_in_save_load() {
        let tmp = tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        let cache_dir = tmp.path().join("cache");

        for fmt in ["flac", "alac", "aac", "mqa", "360"] {
            let cfg = Config {
                format: fmt.to_string(),
                ..Config::default()
            };
            save_config_to(&cfg, &config_dir);
            let loaded = load_config_from(&config_dir, &cache_dir);
            assert_eq!(loaded.format, fmt);
        }
    }

    #[test]
    fn test_invalid_postprocess_codec_normalized() {
        let tmp = tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        let cache_dir = tmp.path().join("cache");

        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.toml"),
            r#"
format = "flac"
output_dir = "~/Music/Nugs"
postprocess_codec = "invalid_codec"
"#,
        )
        .unwrap();

        let cfg = load_config_from(&config_dir, &cache_dir);
        assert_eq!(cfg.postprocess_codec, "none");
    }

    #[test]
    fn test_invalid_flac_convert_normalized() {
        let tmp = tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        let cache_dir = tmp.path().join("cache");

        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("config.toml"),
            r#"
format = "flac"
output_dir = "~/Music/Nugs"
flac_convert = "mp3"
"#,
        )
        .unwrap();

        let cfg = load_config_from(&config_dir, &cache_dir);
        assert_eq!(cfg.flac_convert, "none");
    }

    #[test]
    fn test_flac_convert_valid_values_persist() {
        let tmp = tempdir().unwrap();
        let config_dir = tmp.path().join("config");
        let cache_dir = tmp.path().join("cache");

        for val in ["none", "alac", "aac"] {
            let cfg = Config {
                flac_convert: val.to_string(),
                ..Config::default()
            };
            save_config_to(&cfg, &config_dir);
            let loaded = load_config_from(&config_dir, &cache_dir);
            assert_eq!(loaded.flac_convert, val);
        }
    }
}
