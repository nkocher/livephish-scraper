use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

/// Shared cancellation flag for parallel downloads.
///
/// The escape watcher sets this to true on Escape key press.
/// Download tasks check it periodically to abort early.
pub type CancelFlag = Arc<AtomicBool>;

/// Create a new cancel flag (initially false).
pub fn new_cancel_flag() -> CancelFlag {
    Arc::new(AtomicBool::new(false))
}

/// Enable cbreak mode: character-at-a-time input without disabling
/// output processing or signal handling.
///
/// Uses crossterm's raw mode as a base, then re-enables OPOST and ISIG
/// so that `println!` works normally and Ctrl+C still terminates.
/// Returns false if the terminal couldn't be configured (non-TTY).
pub fn enable_cbreak() -> bool {
    if crossterm::terminal::enable_raw_mode().is_err() {
        return false;
    }
    #[cfg(unix)]
    unsafe {
        let mut termios: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(libc::STDIN_FILENO, &mut termios) == 0 {
            termios.c_oflag |= libc::OPOST;
            termios.c_lflag |= libc::ISIG;
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &termios);
        }
    }
    true
}

/// Restore terminal to normal mode after cbreak.
pub fn disable_cbreak() {
    let _ = crossterm::terminal::disable_raw_mode();
}

/// Spawn a background thread that watches for Escape key.
///
/// Sets `cancel` to true when Escape is pressed, then exits.
/// Also exits when `cancel` is already true (downloads finished).
/// Returns a JoinHandle that can be awaited for clean shutdown.
///
/// Requires cbreak mode to be enabled (via `enable_cbreak`) for
/// crossterm to receive individual key events without Enter.
pub fn spawn_escape_watcher(cancel: CancelFlag) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        while !cancel.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
            if let Ok(true) = event::poll(Duration::from_millis(0)) {
                if matches!(
                    event::read(),
                    Ok(Event::Key(KeyEvent {
                        code: KeyCode::Esc,
                        modifiers: KeyModifiers::NONE,
                        ..
                    }))
                ) {
                    cancel.store(true, Ordering::Relaxed);
                    break;
                }
            }
        }
        // Drain buffered events to leave terminal clean for next prompt
        while event::poll(Duration::from_millis(0)).unwrap_or(false) {
            let _ = event::read();
        }
    })
}
