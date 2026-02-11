"""Download engine with Rich progress bars and .part file safety."""

import logging
import os
import select
import sys
from collections.abc import Callable
from pathlib import Path

import httpx
from rich.console import Console
from rich.progress import (
    BarColumn,
    DownloadColumn,
    Progress,
    SpinnerColumn,
    TaskProgressColumn,
    TextColumn,
    TransferSpeedColumn,
)

from livephish.models import Quality, Show, Track, sanitize_filename
from livephish.tagger import tag_track

logger = logging.getLogger(__name__)

USER_AGENT = "LivePhish/3.4.5.357 (Android; 7.1.2; Asus; ASUS_Z01QD)"
REFERER = "https://plus.livephish.com/"

console = Console()


class _DownloadCancelled(Exception):
    """Raised when user presses Escape during a download."""


class _EscapeMonitor:
    """Detect Escape keypresses during downloads via cbreak mode.

    Uses cbreak mode (not raw) so Ctrl+C still generates SIGINT.
    Uses select.select() to poll stdin — avoids os.set_blocking() which
    would also make stdout non-blocking (they share a TTY file description),
    breaking Rich's progress bar writes.
    Falls back to no-op on Windows or non-tty environments.
    """

    def __init__(self):
        self._active = False
        self._old_settings = None
        self._fd = None

    def __enter__(self):
        self.start()
        return self

    def __exit__(self, *exc):
        self.stop()
        return False

    def start(self):
        """Enter cbreak mode. No-op if not a tty."""
        try:
            import termios
            import tty

            self._fd = sys.stdin.fileno()
            self._old_settings = termios.tcgetattr(self._fd)
            tty.setcbreak(self._fd)
            self._active = True
        except (ImportError, OSError):
            self._active = False

    def stop(self):
        """Restore original terminal settings."""
        if self._active and self._old_settings is not None:
            import termios

            termios.tcsetattr(self._fd, termios.TCSADRAIN, self._old_settings)
            self._active = False

    def drain(self):
        """Discard any buffered stdin bytes (call before prompts)."""
        if self._fd is None:
            return
        while select.select([self._fd], [], [], 0)[0]:
            try:
                os.read(self._fd, 1024)
            except OSError:
                break

    def check(self) -> bool:
        """Non-blocking check: was Escape pressed?

        Returns True only for standalone Escape, not arrow/function key
        escape sequences (which also start with \\x1b).
        """
        if not self._active:
            return False
        if not select.select([self._fd], [], [], 0)[0]:
            return False
        try:
            b = os.read(self._fd, 1)
        except OSError:
            return False
        if b != b"\x1b":
            return False
        # Disambiguate: escape sequences arrive within microseconds,
        # so a short wait distinguishes standalone Escape from arrow keys
        if select.select([self._fd], [], [], 0.01)[0]:
            try:
                os.read(self._fd, 8)  # drain sequence bytes (e.g. [A, [B)
            except OSError:
                pass
            return False  # was an escape sequence
        return True  # standalone Escape


def _part_path(dest: Path) -> Path:
    """Return the .part path for a destination file."""
    return dest.with_suffix(dest.suffix + ".part")


def make_track_filename(track_num: int, title: str, extension: str) -> str:
    """Format a track filename with leading number.

    Args:
        track_num: Track number (1-indexed)
        title: Song title
        extension: File extension (e.g., ".flac" or ".m4a")

    Returns:
        Formatted filename like "01. Song Title.flac"
    """
    return f"{track_num:02d}. {sanitize_filename(title)}{extension}"


def download_show(
    show: Show,
    tracks_with_urls: list[tuple[Track, str, Quality]],
    output_dir: Path,
    on_complete: Callable | None = None,
) -> bool:
    """Download all tracks in a show with progress bars.

    Args:
        show: Show containing metadata
        tracks_with_urls: List of (Track, download_url, Quality) tuples
        output_dir: Base output directory
        on_complete: Optional callback invoked after each track completes

    Returns:
        True if all tracks completed (or failed normally), False if user cancelled.
    """
    show_dir = output_dir / show.folder_name
    show_dir.mkdir(parents=True, exist_ok=True)

    failed = []
    completed = 0
    total = len(tracks_with_urls)
    cancelled = False

    with _EscapeMonitor() as monitor:
        with Progress(
            SpinnerColumn(),
            TextColumn("{task.description}", justify="right"),
            BarColumn(),
            TaskProgressColumn(),
            DownloadColumn(),
            TransferSpeedColumn(),
        ) as progress:
            for track, url, quality in tracks_with_urls:
                filename = make_track_filename(
                    track.track_num, track.song_title, quality.extension
                )
                dest = show_dir / filename

                if dest.exists():
                    print(f"⏭  Skipping (already exists): {filename}")
                    completed += 1
                    continue

                # Retry loop: on Escape + "n", re-download this same track
                while True:
                    try:
                        download_track(
                            url, dest, track, progress,
                            escape_check=monitor.check,
                        )
                        tag_track(dest, show, track)
                        completed += 1
                        if on_complete is not None:
                            on_complete()
                        break  # track done, move to next

                    except _DownloadCancelled:
                        part = _part_path(dest)
                        if part.exists():
                            part.unlink()

                        # Pause terminal mode + progress for clean prompt
                        monitor.stop()
                        monitor.drain()
                        progress.stop()

                        console.print(
                            f"\n[yellow]Escape pressed — {completed}/{total} "
                            f"tracks saved.[/yellow]"
                        )
                        try:
                            answer = console.input(
                                "[yellow]Cancel remaining downloads? [Y/n] [/yellow]"
                            )
                            if answer.strip().lower() not in ("n", "no"):
                                cancelled = True
                                break
                            # Retry this track from scratch
                            console.print(
                                "[dim]Resuming downloads...[/dim]"
                            )
                            progress.start()
                            monitor.start()
                            continue  # retry the while loop
                        except KeyboardInterrupt:
                            cancelled = True
                            break

                    except KeyboardInterrupt:
                        part = _part_path(dest)
                        if part.exists():
                            part.unlink()
                        raise

                    except Exception as e:
                        logger.error(
                            f"Failed to download {track.song_title}: {e}"
                        )
                        failed.append(track.song_title)
                        part = _part_path(dest)
                        if part.exists():
                            part.unlink()
                        break  # move to next track

                if cancelled:
                    break

    if failed:
        console.print(
            f"[yellow]Could not download {len(failed)} track(s): {', '.join(failed)}[/yellow]"
        )

    return not cancelled


def download_track(
    url: str,
    dest: Path,
    track: Track,
    progress: Progress,
    *,
    escape_check: Callable[[], bool] | None = None,
) -> Path:
    """Download a single track with progress tracking.

    Args:
        url: Download URL
        dest: Final destination path
        track: Track containing metadata for display
        progress: Rich Progress instance
        escape_check: Optional callable that returns True if Escape was pressed

    Returns:
        Path to the downloaded file
    """
    part_path = _part_path(dest)

    # Delete stale partial file from previous failed attempt
    if part_path.exists():
        part_path.unlink()

    # Truncate display name for progress bar
    display_name = track.song_title
    truncated_name = (
        display_name[:27] + "..." if len(display_name) > 30 else display_name
    )
    task_id = progress.add_task(truncated_name, total=None)

    try:
        with httpx.stream(
            "GET",
            url,
            headers={
                "Referer": REFERER,
                "User-Agent": USER_AGENT,
                "Range": "bytes=0-",
            },
            follow_redirects=True,
            timeout=60.0,
        ) as response:
            response.raise_for_status()

            # Get total size from Content-Length header
            total = int(response.headers.get("content-length", 0))
            progress.update(task_id, total=total)

            # Stream download to .part file
            chunk_count = 0
            with part_path.open("wb") as f:
                for chunk in response.iter_bytes(chunk_size=8192):
                    f.write(chunk)
                    progress.update(task_id, advance=len(chunk))
                    chunk_count += 1
                    if (
                        escape_check is not None
                        and chunk_count % 64 == 0
                        and escape_check()
                    ):
                        raise _DownloadCancelled()

        # Atomic rename on successful completion
        part_path.rename(dest)
        logger.info(f"Downloaded: {dest.name}")
        return dest

    except _DownloadCancelled:
        raise

    except Exception as e:
        logger.error(f"Download failed for {display_name}: {e}")
        # Leave .part file for debugging
        raise
