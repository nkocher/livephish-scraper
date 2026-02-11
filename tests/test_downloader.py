"""Tests for downloader filename utilities and escape cancellation."""

import pytest
from livephish.downloader import make_track_filename
from livephish.models import sanitize_filename


def test_make_track_filename_basic():
    """Basic track filename with single digit track number."""
    result = make_track_filename(1, "Tweezer", ".flac")
    assert result == "01. Tweezer.flac"


def test_make_track_filename_double_digit():
    """Track filename with double digit track number."""
    result = make_track_filename(12, "Sand", ".m4a")
    assert result == "12. Sand.m4a"


def test_make_track_filename_special_chars():
    """Special characters in song title should be sanitized."""
    result = make_track_filename(1, 'Say It Ain\'t So / "Hey"', ".flac")
    # The / and " should be replaced with _
    assert "/" not in result
    assert '"' not in result
    assert "_" in result
    assert result == "01. Say It Ain't So _ _Hey_.flac"


def test_sanitize_filename_replaces_unsafe():
    """Unsafe filesystem characters should be replaced with underscores."""
    result = sanitize_filename('foo:bar/baz*"qux')
    # All unsafe chars should be replaced
    assert ":" not in result
    assert "/" not in result
    assert "*" not in result
    assert '"' not in result
    assert result == "foo_bar_baz__qux"


def test_sanitize_filename_truncation():
    """Long filenames should be truncated to 200 characters."""
    long_name = "a" * 300
    result = sanitize_filename(long_name)
    assert len(result) == 200


def test_sanitize_filename_safe_chars_preserved():
    """Safe characters and normal punctuation should be preserved."""
    result = sanitize_filename("Normal Song Title (2024)")
    assert result == "Normal Song Title (2024)"


def test_make_track_filename_leading_zero():
    """Single digit track numbers should have leading zero."""
    result = make_track_filename(5, "Tube", ".flac")
    assert result == "05. Tube.flac"
    assert result.startswith("05.")


def _make_show():
    from livephish.models import Show
    return Show(
        container_id=1, artist_name="Test", container_info="Test Show",
        venue_name="Venue", venue_city="City", venue_state="ST",
        performance_date="2024-01-01", performance_date_formatted="01/01/2024",
        performance_date_year="2024",
    )


def _make_tracks_with_urls(count=3):
    from livephish.models import Track, Quality
    quality = Quality(code="flac", specs="FLAC", extension=".flac")
    titles = ["First Song", "Second Song", "Third Song", "Fourth Song", "Fifth Song"]
    return [
        (
            Track(track_id=i, song_id=i, song_title=titles[i - 1],
                  track_num=i, disc_num=1, set_num=1),
            f"http://url{i}",
            quality,
        )
        for i in range(1, count + 1)
    ]


class TestDownloadShowResilience:
    """Tests for download_show error handling."""

    def test_single_track_failure_continues(self, tmp_path):
        """If one track download fails, other tracks should still download."""
        from livephish.models import Show, Track, Quality
        from livephish.downloader import download_show
        from unittest.mock import patch, MagicMock

        show = Show(
            container_id=1, artist_name="Test", container_info="Test Show",
            venue_name="Venue", venue_city="City", venue_state="ST",
            performance_date="2024-01-01", performance_date_formatted="01/01/2024",
            performance_date_year="2024",
        )

        track1 = Track(track_id=1, song_id=1, song_title="Good Song", track_num=1, disc_num=1, set_num=1)
        track2 = Track(track_id=2, song_id=2, song_title="Bad Song", track_num=2, disc_num=1, set_num=1)
        track3 = Track(track_id=3, song_id=3, song_title="Another Good", track_num=3, disc_num=1, set_num=1)

        quality = Quality(code="flac", specs="FLAC", extension=".flac")

        call_count = 0
        def mock_download_track(url, dest, track, progress, **kwargs):
            nonlocal call_count
            call_count += 1
            if track.song_title == "Bad Song":
                # Create a .part file to test cleanup
                part = dest.with_suffix(dest.suffix + ".part")
                part.write_bytes(b"partial")
                raise ConnectionError("Network failed")
            # Create the file for successful downloads
            dest.write_bytes(b"audio data")

        tracks_with_urls = [
            (track1, "http://url1", quality),
            (track2, "http://url2", quality),
            (track3, "http://url3", quality),
        ]

        with patch("livephish.downloader.download_track", side_effect=mock_download_track):
            with patch("livephish.downloader.tag_track"):
                result = download_show(show, tracks_with_urls, tmp_path)

        # Should return True (completed, not cancelled)
        assert result is True

        # All 3 tracks attempted
        assert call_count == 3

        # Good tracks exist, bad track doesn't
        show_dir = tmp_path / show.folder_name
        assert (show_dir / "01. Good Song.flac").exists()
        assert not (show_dir / "02. Bad Song.flac").exists()
        assert (show_dir / "03. Another Good.flac").exists()

        # .part file should be cleaned up
        assert not (show_dir / "02. Bad Song.flac.part").exists()


class TestEscapeCancellation:
    """Tests for Escape-key download cancellation."""

    def test_escape_cancellation_cleans_up(self, tmp_path):
        """Escape during download: .part deleted, returns False, first track kept."""
        from livephish.downloader import download_show, _DownloadCancelled
        from unittest.mock import patch, MagicMock

        show = _make_show()
        tracks = _make_tracks_with_urls(3)

        call_count = 0
        def mock_download_track(url, dest, track, progress, **kwargs):
            nonlocal call_count
            call_count += 1
            if call_count == 1:
                # First track succeeds
                dest.write_bytes(b"audio data")
            else:
                # Second track: simulate escape by creating .part and raising
                part = dest.with_suffix(dest.suffix + ".part")
                part.write_bytes(b"partial")
                raise _DownloadCancelled()

        mock_monitor = MagicMock()
        mock_monitor.__enter__ = MagicMock(return_value=mock_monitor)
        mock_monitor.__exit__ = MagicMock(return_value=False)

        with patch("livephish.downloader.download_track", side_effect=mock_download_track):
            with patch("livephish.downloader.tag_track"):
                with patch("livephish.downloader._EscapeMonitor", return_value=mock_monitor):
                    with patch("livephish.downloader.console") as mock_console:
                        mock_console.input.return_value = "y"
                        result = download_show(show, tracks, tmp_path)

        # User confirmed cancel
        assert result is False

        # Only 2 tracks attempted (first succeeds, second raises, loop breaks)
        assert call_count == 2

        # First track file should exist
        show_dir = tmp_path / show.folder_name
        assert (show_dir / "01. First Song.flac").exists()

        # Second track's .part should be cleaned up
        assert not (show_dir / "02. Second Song.flac.part").exists()

        # Third track never attempted
        assert not (show_dir / "03. Third Song.flac").exists()

    def test_escape_continue_retries_track(self, tmp_path):
        """Escape + 'n' retries the interrupted track, then continues."""
        from livephish.downloader import download_show, _DownloadCancelled
        from unittest.mock import patch, MagicMock

        show = _make_show()
        tracks = _make_tracks_with_urls(3)

        call_count = 0
        cancelled_once = False
        def mock_download_track(url, dest, track, progress, **kwargs):
            nonlocal call_count, cancelled_once
            call_count += 1
            if track.song_title == "Second Song" and not cancelled_once:
                # Second track first attempt: escape
                cancelled_once = True
                part = dest.with_suffix(dest.suffix + ".part")
                part.write_bytes(b"partial")
                raise _DownloadCancelled()
            # All other calls (including second track retry) succeed
            dest.write_bytes(b"audio data")

        mock_monitor = MagicMock()
        mock_monitor.__enter__ = MagicMock(return_value=mock_monitor)
        mock_monitor.__exit__ = MagicMock(return_value=False)

        with patch("livephish.downloader.download_track", side_effect=mock_download_track):
            with patch("livephish.downloader.tag_track"):
                with patch("livephish.downloader._EscapeMonitor", return_value=mock_monitor):
                    with patch("livephish.downloader.console") as mock_console:
                        mock_console.input.return_value = "n"
                        result = download_show(show, tracks, tmp_path)

        # User chose to continue
        assert result is True

        # 4 calls: track1 + track2 (escape) + track2 (retry) + track3
        assert call_count == 4

        show_dir = tmp_path / show.folder_name
        # All three tracks downloaded (second was retried)
        assert (show_dir / "01. First Song.flac").exists()
        assert (show_dir / "02. Second Song.flac").exists()
        assert (show_dir / "03. Third Song.flac").exists()
        # No .part files left
        assert not (show_dir / "02. Second Song.flac.part").exists()

    def test_escape_monitor_noop_without_tty(self):
        """EscapeMonitor degrades gracefully when termios unavailable."""
        from livephish.downloader import _EscapeMonitor
        from unittest.mock import patch

        with patch.dict("sys.modules", {"termios": None, "tty": None}):
            monitor = _EscapeMonitor()
            monitor.start()
            assert monitor._active is False
            assert monitor.check() is False
            monitor.stop()  # should not raise
