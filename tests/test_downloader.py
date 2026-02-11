"""Tests for downloader filename utilities."""

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


class TestDownloadShowResilience:
    """Tests for download_show error handling."""

    def test_single_track_failure_continues(self, tmp_path):
        """If one track download fails, other tracks should still download."""
        # This tests the error handling logic in download_show
        # We need to verify that failed tracks are reported but don't stop the process
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

        quality = Quality(specs="FLAC", extension=".flac")

        call_count = 0
        def mock_download_track(url, dest, track, progress):
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
                download_show(show, tracks_with_urls, tmp_path)

        # All 3 tracks attempted
        assert call_count == 3

        # Good tracks exist, bad track doesn't
        show_dir = tmp_path / show.folder_name
        assert (show_dir / "01. Good Song.flac").exists()
        assert not (show_dir / "02. Bad Song.flac").exists()
        assert (show_dir / "03. Another Good.flac").exists()

        # .part file should be cleaned up
        assert not (show_dir / "02. Bad Song.flac.part").exists()
