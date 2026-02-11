"""Tests for LivePhish data models."""

import pytest

from livephish.models import (
    FORMAT_CODES,
    CatalogShow,
    Quality,
    Show,
    Track,
    sanitize_filename,
)


class TestTrack:
    """Tests for Track model."""

    def test_from_dict_with_all_fields(self, sample_track_dict):
        """Test Track.from_dict with complete data."""
        track = Track.from_dict(sample_track_dict)

        assert track.track_id == 12345
        assert track.song_id == 678
        assert track.song_title == "Tweezer"
        assert track.track_num == 1
        assert track.disc_num == 1
        assert track.set_num == 1
        assert track.duration_seconds == 960
        assert track.duration_display == "16:00"

    def test_from_dict_with_minimal_fields(self):
        """Test Track.from_dict handles missing optional fields with defaults."""
        minimal_dict = {"trackID": 99999}
        track = Track.from_dict(minimal_dict)

        assert track.track_id == 99999
        assert track.song_id == 0
        assert track.song_title == ""
        assert track.track_num == 0
        assert track.disc_num == 1  # Default for discNum
        assert track.set_num == 0
        assert track.duration_seconds == 0
        assert track.duration_display == ""


class TestShow:
    """Tests for Show model."""

    def test_from_dict_basic(self, sample_show_dict):
        """Test Show.from_dict with basic show data."""
        show = Show.from_dict(sample_show_dict)

        assert show.container_id == 99999
        assert show.artist_name == "Phish"
        assert show.container_info == "2024-08-31 Dick's Sporting Goods Park"
        assert show.venue_name == "Dick's Sporting Goods Park"
        assert show.venue_city == "Commerce City"
        assert show.venue_state == "CO"
        assert show.performance_date == "2024-08-31"
        assert show.performance_date_formatted == "08/31/2024"
        assert show.performance_date_year == "2024"
        assert show.total_duration_seconds == 11565
        assert show.total_duration_display == "3:12:45"
        assert show.image_url == "https://example.com/img.jpg"
        assert show.tracks == []
        assert show.songs == []

    def test_from_dict_with_tracks(self, sample_show_dict, sample_track_dict):
        """Test Show.from_dict parses embedded tracks."""
        sample_show_dict["tracks"] = [sample_track_dict]
        show = Show.from_dict(sample_show_dict)

        assert len(show.tracks) == 1
        track = show.tracks[0]
        assert track.track_id == 12345
        assert track.song_title == "Tweezer"

    def test_folder_name_sanitization(self):
        """Test Show.folder_name replaces unsafe filesystem characters."""
        show_data = {
            "containerID": 1,
            "artistName": "Test:Artist",
            "containerInfo": 'Show/With*Unsafe?Chars"<>|',
            "venueName": "Venue",
            "venueCity": "City",
            "venueState": "ST",
            "performanceDate": "2024-01-01",
            "performanceDateFormatted": "01/01/2024",
            "performanceDateYear": "2024",
        }
        show = Show.from_dict(show_data)
        folder = show.folder_name

        # All unsafe chars should be replaced with underscores
        assert "/" not in folder
        assert ":" not in folder
        assert "*" not in folder
        assert "?" not in folder
        assert '"' not in folder
        assert "<" not in folder
        assert ">" not in folder
        assert "|" not in folder
        assert "\\" not in folder

    def test_folder_name_truncation(self):
        """Test Show.folder_name truncates long names."""
        long_info = "A" * 200
        show_data = {
            "containerID": 1,
            "artistName": "Artist",
            "containerInfo": long_info,
            "venueName": "Venue",
            "venueCity": "City",
            "venueState": "ST",
            "performanceDate": "2024-01-01",
            "performanceDateFormatted": "01/01/2024",
            "performanceDateYear": "2024",
        }
        show = Show.from_dict(show_data)

        # Should be truncated to 120 chars max
        assert len(show.folder_name) <= 120

    def test_display_location(self, sample_show_dict):
        """Test Show.display_location formats location string."""
        show = Show.from_dict(sample_show_dict)
        assert show.display_location == "Dick's Sporting Goods Park, Commerce City, CO"

    def test_display_location_partial(self):
        """Test Show.display_location with missing city/state."""
        show_data = {
            "containerID": 1,
            "artistName": "Artist",
            "containerInfo": "Info",
            "venueName": "Venue Only",
            "venueCity": "",
            "venueState": "",
            "performanceDate": "2024-01-01",
            "performanceDateFormatted": "01/01/2024",
            "performanceDateYear": "2024",
        }
        show = Show.from_dict(show_data)
        assert show.display_location == "Venue Only"

    def test_sets_grouped(self, sample_track_dict):
        """Test Show.sets_grouped organizes tracks by set number."""
        track1 = sample_track_dict.copy()
        track1["setNum"] = 1
        track1["trackID"] = 1

        track2 = sample_track_dict.copy()
        track2["setNum"] = 1
        track2["trackID"] = 2

        track3 = sample_track_dict.copy()
        track3["setNum"] = 2
        track3["trackID"] = 3

        show_data = {
            "containerID": 1,
            "artistName": "Artist",
            "containerInfo": "Info",
            "venueName": "Venue",
            "venueCity": "City",
            "venueState": "ST",
            "performanceDate": "2024-01-01",
            "performanceDateFormatted": "01/01/2024",
            "performanceDateYear": "2024",
            "tracks": [track1, track2, track3],
        }
        show = Show.from_dict(show_data)
        sets = show.sets_grouped()

        assert len(sets) == 2
        assert len(sets[1]) == 2
        assert len(sets[2]) == 1
        assert sets[1][0].track_id == 1
        assert sets[1][1].track_id == 2
        assert sets[2][0].track_id == 3


class TestCatalogShow:
    """Tests for CatalogShow model."""

    def test_from_dict(self, sample_catalog_show_dict):
        """Test CatalogShow.from_dict parses catalog entry."""
        catalog_show = CatalogShow.from_dict(sample_catalog_show_dict)

        assert catalog_show.container_id == 99999
        assert catalog_show.artist_name == "Phish"
        assert catalog_show.container_info == "2024-08-31 Dick's Sporting Goods Park"
        assert catalog_show.venue_name == "Dick's Sporting Goods Park"
        assert catalog_show.venue_city == "Commerce City"
        assert catalog_show.venue_state == "CO"
        assert catalog_show.performance_date == "2024-08-31"
        assert catalog_show.performance_date_formatted == "08/31/2024"
        assert catalog_show.performance_date_year == "2024"
        assert catalog_show.image_url == "https://example.com/img.jpg"
        assert catalog_show.song_list == "Tweezer, Sand, Piper"


class TestQuality:
    """Tests for Quality model."""

    def test_from_stream_url_alac(self):
        """Test Quality.from_stream_url detects ALAC format."""
        url = "https://example.com/path/.alac16/track.m4a"
        quality = Quality.from_stream_url(url)

        assert quality is not None
        assert quality.code == "alac"
        assert quality.specs == "16-bit / 44.1 kHz ALAC"
        assert quality.extension == ".m4a"

    def test_from_stream_url_flac(self):
        """Test Quality.from_stream_url detects FLAC format."""
        url = "https://example.com/path/.flac16/track.flac"
        quality = Quality.from_stream_url(url)

        assert quality is not None
        assert quality.code == "flac"
        assert quality.specs == "16-bit / 44.1 kHz FLAC"
        assert quality.extension == ".flac"

    def test_from_stream_url_aac(self):
        """Test Quality.from_stream_url detects AAC format."""
        url = "https://example.com/path/.aac150/track.m4a"
        quality = Quality.from_stream_url(url)

        assert quality is not None
        assert quality.code == "aac"
        assert quality.specs == "AAC 150"
        assert quality.extension == ".m4a"

    def test_from_stream_url_unknown_returns_none(self):
        """Test Quality.from_stream_url returns None for unknown format."""
        url = "https://example.com/path/unknown/track.mp3"
        quality = Quality.from_stream_url(url)

        assert quality is None


class TestFormatCodes:
    """Tests for FORMAT_CODES dict."""

    def test_format_codes_values(self):
        """Test FORMAT_CODES has expected keys and values."""
        assert FORMAT_CODES["flac"] == 4
        assert FORMAT_CODES["alac"] == 2
        assert FORMAT_CODES["aac"] == 3


class TestSanitizeFilename:
    """Tests for sanitize_filename function."""

    def test_windows_reserved_con(self):
        assert sanitize_filename("CON") == "_CON"

    def test_windows_reserved_prn(self):
        assert sanitize_filename("PRN") == "_PRN"

    def test_windows_reserved_com1(self):
        assert sanitize_filename("COM1") == "_COM1"

    def test_trailing_dot(self):
        result = sanitize_filename("file.")
        assert not result.endswith(".")

    def test_trailing_space(self):
        result = sanitize_filename("file ")
        assert not result.endswith(" ")

    def test_unsafe_chars_replaced(self):
        assert sanitize_filename('a:b*c?d') == "a_b_c_d"

    def test_max_length(self):
        long_name = "a" * 300
        assert len(sanitize_filename(long_name)) == 200

    def test_custom_max_length(self):
        long_name = "a" * 300
        assert len(sanitize_filename(long_name, max_length=120)) == 120

    def test_normal_name_unchanged(self):
        assert sanitize_filename("Phish - 2024-08-31 Dicks") == "Phish - 2024-08-31 Dicks"

    def test_reserved_with_extension(self):
        """CON.txt should be escaped to _CON.txt"""
        assert sanitize_filename("CON.txt") == "_CON.txt"
