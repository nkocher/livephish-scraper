"""Download engine with Rich progress bars and .part file safety."""

import logging
from collections.abc import Callable
from pathlib import Path

import httpx
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
) -> None:
    """Download all tracks in a show with progress bars.

    Args:
        show: Show containing metadata
        tracks_with_urls: List of (Track, download_url, Quality) tuples
        output_dir: Base output directory
        on_complete: Optional callback invoked after each track completes
    """
    show_dir = output_dir / show.folder_name
    show_dir.mkdir(parents=True, exist_ok=True)

    failed = []
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
                continue

            try:
                download_track(url, dest, track, progress)
                tag_track(dest, show, track)
                if on_complete is not None:
                    on_complete()
            except KeyboardInterrupt:
                part_path = dest.with_suffix(dest.suffix + ".part")
                if part_path.exists():
                    part_path.unlink()
                raise
            except Exception as e:
                logger.error(f"Failed to download {track.song_title}: {e}")
                failed.append(track.song_title)
                part_path = dest.with_suffix(dest.suffix + ".part")
                if part_path.exists():
                    part_path.unlink()
                continue

    if failed:
        from rich.console import Console
        Console().print(
            f"[yellow]Could not download {len(failed)} track(s): {', '.join(failed)}[/yellow]"
        )


def download_track(
    url: str, dest: Path, track: Track, progress: Progress
) -> Path:
    """Download a single track with progress tracking.

    Args:
        url: Download URL
        dest: Final destination path
        track: Track containing metadata for display
        progress: Rich Progress instance

    Returns:
        Path to the downloaded file
    """
    part_path = dest.with_suffix(dest.suffix + ".part")

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
            with part_path.open("wb") as f:
                for chunk in response.iter_bytes(chunk_size=8192):
                    f.write(chunk)
                    progress.update(task_id, advance=len(chunk))

        # Atomic rename on successful completion
        part_path.rename(dest)
        logger.info(f"Downloaded: {dest.name}")
        return dest

    except Exception as e:
        logger.error(f"Download failed for {display_name}: {e}")
        # Leave .part file for debugging
        raise
