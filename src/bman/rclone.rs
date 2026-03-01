/// Auto-import Google OAuth credentials from rclone config.
///
/// Parses `~/.config/rclone/rclone.conf` (INI format), finds a section
/// with `type = drive`, and extracts `client_id`, `client_secret`, and
/// `refresh_token` (from the JSON `token` field). Returns None if rclone
/// isn't installed, no Drive remote exists, or required fields are missing.
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub struct RcloneCredentials {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
}

/// Try to load Google Drive OAuth credentials from rclone config.
pub fn load_rclone_credentials() -> Option<RcloneCredentials> {
    let config_path = rclone_config_path()?;
    let contents = fs::read_to_string(&config_path).ok()?;
    parse_rclone_config(&contents)
}

fn rclone_config_path() -> Option<PathBuf> {
    // RCLONE_CONFIG env var takes priority
    if let Ok(p) = std::env::var("RCLONE_CONFIG") {
        let path = PathBuf::from(p);
        if path.exists() {
            return Some(path);
        }
    }
    // XDG_CONFIG_HOME/rclone/rclone.conf or ~/.config/rclone/rclone.conf
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let p = PathBuf::from(xdg).join("rclone/rclone.conf");
        if p.exists() {
            return Some(p);
        }
    }
    let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
    let p = home.join(".config/rclone/rclone.conf");
    if p.exists() { Some(p) } else { None }
}

fn parse_rclone_config(contents: &str) -> Option<RcloneCredentials> {
    // Simple INI parser: find the first section with type = drive
    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current_section = String::new();

    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].to_string();
            sections.entry(current_section.clone()).or_default();
        } else if let Some((key, val)) = line.split_once('=') {
            if !current_section.is_empty() {
                sections
                    .entry(current_section.clone())
                    .or_default()
                    .insert(key.trim().to_string(), val.trim().to_string());
            }
        }
    }

    // Find the first section with type = drive
    for props in sections.values() {
        if props.get("type").map(String::as_str) != Some("drive") {
            continue;
        }
        let client_id = props.get("client_id").filter(|s| !s.is_empty())?;
        let client_secret = props.get("client_secret").filter(|s| !s.is_empty())?;

        // The token field is JSON: {"access_token":"...","refresh_token":"...","expiry":"..."}
        let token_json = props.get("token").filter(|s| !s.is_empty())?;
        let token_obj: serde_json::Value = serde_json::from_str(token_json).ok()?;
        let refresh_token = token_obj
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())?;

        return Some(RcloneCredentials {
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
            refresh_token: refresh_token.to_string(),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rclone_config_gdrive() {
        let config = r#"
[gdrive]
type = drive
client_id = 123456.apps.googleusercontent.com
client_secret = GOCSPX-secret123
scope = drive
token = {"access_token":"ya29.old","token_type":"Bearer","refresh_token":"1//0fresh","expiry":"2025-01-01T00:00:00Z"}
team_drive =
"#;
        let creds = parse_rclone_config(config).unwrap();
        assert_eq!(creds.client_id, "123456.apps.googleusercontent.com");
        assert_eq!(creds.client_secret, "GOCSPX-secret123");
        assert_eq!(creds.refresh_token, "1//0fresh");
    }

    #[test]
    fn test_parse_rclone_config_no_drive_section() {
        let config = r#"
[s3]
type = s3
provider = AWS
"#;
        assert!(parse_rclone_config(config).is_none());
    }

    #[test]
    fn test_parse_rclone_config_missing_token() {
        let config = r#"
[gdrive]
type = drive
client_id = cid
client_secret = csec
"#;
        assert!(parse_rclone_config(config).is_none());
    }

    #[test]
    fn test_parse_rclone_config_empty_refresh_token() {
        let config = r#"
[gdrive]
type = drive
client_id = cid
client_secret = csec
token = {"access_token":"ya29","refresh_token":"","expiry":"2025-01-01T00:00:00Z"}
"#;
        assert!(parse_rclone_config(config).is_none());
    }

    #[test]
    fn test_parse_rclone_config_multiple_remotes() {
        let config = r#"
[s3]
type = s3
provider = AWS

[mygdrive]
type = drive
client_id = my_id
client_secret = my_secret
token = {"access_token":"ya29","refresh_token":"1//refresh","expiry":"2025-01-01T00:00:00Z"}
"#;
        let creds = parse_rclone_config(config).unwrap();
        assert_eq!(creds.client_id, "my_id");
        assert_eq!(creds.refresh_token, "1//refresh");
    }
}
