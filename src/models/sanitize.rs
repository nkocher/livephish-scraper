use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;

static UNSAFE_CHARS: Lazy<Regex> = Lazy::new(|| Regex::new(r#"[\\/:*?"<>|]"#).unwrap());

static WINDOWS_RESERVED: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let mut set = HashSet::new();
    set.insert("con");
    set.insert("prn");
    set.insert("nul");
    set.insert("aux");
    for i in 1..=9 {
        // Use leaked strings for static lifetime — these are small and live for the program's duration
        set.insert(Box::leak(format!("com{i}").into_boxed_str()));
        set.insert(Box::leak(format!("lpt{i}").into_boxed_str()));
    }
    set
});

/// Sanitize a string for use as a cross-platform filename.
///
/// Handles: unsafe characters, Windows reserved names, trailing dots/spaces,
/// double dots, and length truncation.
pub fn sanitize_filename(name: &str, max_length: usize) -> String {
    let mut sanitized = UNSAFE_CHARS.replace_all(name, "_").to_string();
    sanitized = sanitized.replace("..", "_");
    sanitized = sanitized.trim_end_matches(['.', ' ']).to_string();

    // Check Windows reserved names (compare base before first dot)
    let base = sanitized.split('.').next().unwrap_or("");
    if WINDOWS_RESERVED.contains(base.to_lowercase().as_str()) {
        sanitized = format!("_{sanitized}");
    }

    if sanitized.len() > max_length {
        sanitized.truncate(max_length);
    }
    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_windows_reserved_con() {
        assert_eq!(sanitize_filename("CON", 200), "_CON");
    }

    #[test]
    fn test_windows_reserved_prn() {
        assert_eq!(sanitize_filename("PRN", 200), "_PRN");
    }

    #[test]
    fn test_windows_reserved_com1() {
        assert_eq!(sanitize_filename("COM1", 200), "_COM1");
    }

    #[test]
    fn test_trailing_dot() {
        let result = sanitize_filename("file.", 200);
        assert!(!result.ends_with('.'));
    }

    #[test]
    fn test_trailing_space() {
        let result = sanitize_filename("file ", 200);
        assert!(!result.ends_with(' '));
    }

    #[test]
    fn test_unsafe_chars_replaced() {
        assert_eq!(sanitize_filename("a:b*c?d", 200), "a_b_c_d");
    }

    #[test]
    fn test_max_length_default() {
        let long_name = "a".repeat(300);
        assert_eq!(sanitize_filename(&long_name, 200).len(), 200);
    }

    #[test]
    fn test_custom_max_length() {
        let long_name = "a".repeat(300);
        assert_eq!(sanitize_filename(&long_name, 120).len(), 120);
    }

    #[test]
    fn test_normal_name_unchanged() {
        assert_eq!(
            sanitize_filename("Phish - 2024-08-31 Dicks", 200),
            "Phish - 2024-08-31 Dicks"
        );
    }

    #[test]
    fn test_reserved_with_extension() {
        assert_eq!(sanitize_filename("CON.txt", 200), "_CON.txt");
    }
}
