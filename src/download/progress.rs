use indicatif::{ProgressBar, ProgressStyle};

/// Create the overall progress bar for tracking completed tracks.
///
/// Shows: dim "tracks" label, progress bar, completed/total count.
/// Use `pb.println()` to print per-track messages above this bar.
pub fn make_overall_bar(total: usize) -> ProgressBar {
    let pb = ProgressBar::new(total as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "  \x1b[2m{msg}\x1b[0m [{bar:30.yellow/dim}] \x1b[38;5;214m{pos}\x1b[0m/{len}",
        )
        .unwrap()
        .progress_chars("━╸─"),
    );
    pb.set_message("tracks");
    pb
}
