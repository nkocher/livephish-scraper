use crate::models::show::DisplayLocation;
use crate::models::CatalogShow;

// ── Constants ────────────────────────────────────────────────────────

/// "← Back" choice used in every menu.
pub const BACK: &str = "\u{2190} Back";

/// ANSI escape: dim text.
const DIM: &str = "\x1b[2m";

/// ANSI escape: reset all styling.
const RESET: &str = "\x1b[0m";

/// 256-color amber (warm gold accent).
const AMBER: &str = "\x1b[38;5;214m";

/// Bold amber for titles.
const BOLD_AMBER: &str = "\x1b[1;38;5;214m";

/// Dim amber for structural elements (headers, rules).
const DIM_AMBER: &str = "\x1b[2;38;5;214m";

/// Middle dot used as separator in labels and summary lines.
pub const MIDDOT: char = '\u{00b7}';

/// Minimum column width for dot-leader alignment.
const DOT_LEADER_MIN_WIDTH: usize = 48;

// ── ANSI helpers ─────────────────────────────────────────────────────

/// Wrap text in ANSI dim.
pub fn dim(text: &str) -> String {
    format!("{DIM}{text}{RESET}")
}

/// Build a choice label with dim secondary text.
///
/// Equivalent to Python's `_dim_label()`. The secondary text (city, state)
/// appears dimmed after the primary text (date, venue).
pub fn dim_label(primary: &str, secondary: &str) -> String {
    if secondary.is_empty() {
        primary.to_string()
    } else {
        format!("{primary}  {DIM}{secondary}{RESET}")
    }
}

// ── Show label formatting ────────────────────────────────────────────

/// Build a show choice label with primary/secondary dim split.
///
/// Mirrors Python's `_format_show_label()`:
/// Primary: "date · artist · venue"
/// Secondary (dim): "· City, State"
pub fn format_show_label(show: &CatalogShow, prefix: &str, include_artist: bool) -> String {
    let mut primary_parts: Vec<&str> = Vec::new();

    let date_str = show.display_date();
    let date_part;
    if !date_str.is_empty() {
        date_part = if prefix.is_empty() {
            date_str.to_string()
        } else {
            format!("{prefix}{date_str}")
        };
        primary_parts.push(&date_part);
    } else {
        date_part = format!("{prefix}Unknown date");
        primary_parts.push(&date_part);
    }

    if include_artist && !show.artist_name.is_empty() {
        primary_parts.push(&show.artist_name);
    }
    if !show.venue_name.is_empty() {
        primary_parts.push(&show.venue_name);
    } else if !include_artist {
        primary_parts.push("Unknown venue");
    }

    let primary = primary_parts.join(&format!(" {MIDDOT} "));

    let loc_short = show.display_location_short();
    let secondary = if loc_short.is_empty() {
        String::new()
    } else {
        format!("{MIDDOT} {loc_short}")
    };

    dim_label(&primary, &secondary)
}

// ── Section headers ──────────────────────────────────────────────────

/// Print a section header line (replaces Python's Rich Rule).
///
/// Used in non-menu contexts (show detail, queue display).
/// Leading blank line matches the banner's top margin for consistent
/// vertical positioning across all screens.
pub fn print_section(title: &str, hint: Option<&str>) {
    println!();
    let title_upper = title.to_uppercase();
    let rule_width = 40usize.saturating_sub(title_upper.len() + 1);
    let rule = "\u{2500}".repeat(rule_width);
    print!("  {BOLD_AMBER}{title_upper}{RESET} {DIM_AMBER}{rule}{RESET}");
    if let Some(h) = hint {
        print!("  {DIM}{h}{RESET}");
    }
    println!();
    println!();
}

/// Build a section separator string for use as a menu choice.
///
/// These are selectable (inquire has no Separator type) — callers must
/// filter them on selection and re-prompt. Rendered as dim amber
/// uppercase labels with a trailing rule for clear visual hierarchy.
pub fn section_header(label: &str) -> String {
    let label_upper = label.to_uppercase();
    let rule_width = 32usize.saturating_sub(label_upper.len() + 1);
    let rule = "\u{2500}".repeat(rule_width);
    format!("{DIM_AMBER}{label_upper} {rule}{RESET}")
}

/// Check if a choice string is a section header (non-actionable).
pub fn is_section_header(choice: &str) -> bool {
    choice.contains('\u{2500}')
}

/// Sort toggle marker used in show list choices.
const SORT_TOGGLE_PREFIX: &str = "\u{21c5} "; // ⇅

/// Build a sort toggle choice label.
pub fn sort_toggle_label(mode_name: &str) -> String {
    format!("{DIM}{SORT_TOGGLE_PREFIX}Sort: {mode_name}{RESET}")
}

/// Check if a choice string is a sort toggle (non-actionable show).
pub fn is_sort_toggle(choice: &str) -> bool {
    choice.contains('\u{21c5}')
}

/// Queue action marker used in show list choices.
const QUEUE_ACTION_PREFIX: &str = "\u{2234} "; // ∴

/// Build a queue action choice label.
pub fn queue_action_label(text: &str) -> String {
    format!("{DIM}{QUEUE_ACTION_PREFIX}{text}{RESET}")
}

/// Check if a choice string is a queue action.
pub fn is_queue_action(choice: &str) -> bool {
    choice.contains('\u{2234}')
}

// ── Screen control ───────────────────────────────────────────────────

/// Clear the terminal screen and move cursor to top-left.
pub fn clear_screen() {
    use std::io::Write;
    print!("\x1b[2J\x1b[H");
    std::io::stdout().flush().ok();
}

// ── Duration formatting ──────────────────────────────────────────────

/// Format seconds as "M:SS" or "H:MM:SS".
pub fn format_duration(seconds: i64) -> String {
    if seconds <= 0 {
        return String::new();
    }
    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    let s = seconds % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

// ── Dot-leader alignment ─────────────────────────────────────────────

/// Calculate column width for dot-leader alignment.
pub fn dot_leader_col_width(title_lengths: &[usize]) -> usize {
    let max_len = title_lengths.iter().copied().max().unwrap_or(20);
    (max_len + 4).max(DOT_LEADER_MIN_WIDTH)
}

/// Format a track line with dot-leader alignment.
///
/// "Song Title ····· 5:23"
pub fn dot_leader_line(title: &str, duration: &str, col_width: usize) -> String {
    if duration.is_empty() {
        return title.to_string();
    }
    let title_len = title.chars().count();
    let dur_len = duration.chars().count();
    let fill = col_width.saturating_sub(title_len + dur_len + 2);
    if fill == 0 {
        return format!("{title} {duration}");
    }
    let dots = "\u{00b7}".repeat(fill);
    format!("{title} {DIM}{dots}{RESET} {DIM}{duration}{RESET}")
}

// ── Banner ───────────────────────────────────────────────────────────

const TAGLINES: &[&str] = &[
    "live archive browser + downloader",
    "every show, every note",
    "your portable tape deck",
    "straight from the soundboard",
    "the vault is open",
];

/// Print the top-level banner with a random tagline.
pub fn print_banner(queue_count: usize) {
    // Simple pseudo-random: use current time nanos
    let idx = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize
        % TAGLINES.len();

    println!();
    println!(
        "  {BOLD_AMBER}\u{266b}  nugs{RESET}  {DIM}{}{RESET}",
        TAGLINES[idx]
    );
    println!();
    let mut status_parts = Vec::new();
    if queue_count > 0 {
        status_parts.push(format!(
            "{AMBER}{queue_count}{RESET}{DIM} show{} queued{RESET}",
            if queue_count != 1 { "s" } else { "" }
        ));
    }
    status_parts.push(format!(
        "{DIM}\u{2191}\u{2193} navigate {MIDDOT} type to filter {MIDDOT} esc back{RESET}"
    ));
    println!(
        "  {}",
        status_parts.join(&format!("  {DIM}{MIDDOT}{RESET}  "))
    );
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dim_label_empty_secondary() {
        assert_eq!(dim_label("hello", ""), "hello");
    }

    #[test]
    fn test_dim_label_with_secondary() {
        let label = dim_label("2024-06-15", "Morrison, CO");
        assert!(label.contains("2024-06-15"));
        assert!(label.contains("Morrison, CO"));
        assert!(label.contains(DIM));
    }

    #[test]
    fn test_format_duration_minutes() {
        assert_eq!(format_duration(185), "3:05");
    }

    #[test]
    fn test_format_duration_hours() {
        assert_eq!(format_duration(3661), "1:01:01");
    }

    #[test]
    fn test_format_duration_zero() {
        assert_eq!(format_duration(0), "");
    }

    #[test]
    fn test_dot_leader_col_width_default() {
        assert_eq!(dot_leader_col_width(&[10, 20, 30]), DOT_LEADER_MIN_WIDTH);
    }

    #[test]
    fn test_dot_leader_col_width_long_titles() {
        assert_eq!(dot_leader_col_width(&[50, 60]), 64);
    }

    #[test]
    fn test_section_header_roundtrip() {
        let header = section_header("Browse");
        assert!(is_section_header(&header));
        // Should contain uppercase label
        assert!(header.contains("BROWSE"));
    }

    #[test]
    fn test_not_section_header() {
        assert!(!is_section_header("Recents"));
    }

    #[test]
    fn test_sort_toggle_roundtrip() {
        let toggle = sort_toggle_label("Newest first");
        assert!(is_sort_toggle(&toggle));
        assert!(toggle.contains("Newest first"));
    }

    #[test]
    fn test_not_sort_toggle() {
        assert!(!is_sort_toggle("Some regular choice"));
        assert!(!is_sort_toggle(""));
    }

    #[test]
    fn test_queue_action_roundtrip() {
        let action = queue_action_label("Add all to queue (5)");
        assert!(is_queue_action(&action));
        assert!(action.contains("Add all to queue (5)"));
    }

    #[test]
    fn test_not_queue_action() {
        assert!(!is_queue_action("Some regular choice"));
        assert!(!is_queue_action(""));
    }
}
