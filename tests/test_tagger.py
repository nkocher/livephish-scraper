"""Tests for audio file tagging."""

from pathlib import Path

import pytest
from mutagen.flac import FLAC

from livephish.models import Show, Track
from livephish.tagger import tag_track


def test_tag_flac(tmp_path):
    """Tag a FLAC file and verify metadata is written correctly."""
    # Create a minimal valid FLAC file using mutagen
    path = tmp_path / "test.flac"

    # Create an empty FLAC file - mutagen can initialize it
    # We'll use the FLAC constructor with an empty file, but need valid structure
    # Simplest approach: create minimal FLAC with valid STREAMINFO
    from mutagen.flac import StreamInfo

    # Build a valid 34-byte STREAMINFO
    # Format: min_block(2) max_block(2) min_frame(3) max_frame(3)
    #         sample_rate(20b) channels(3b) bps(5b) samples(36b) md5(16)
    streaminfo = bytearray()
    streaminfo.extend((4096).to_bytes(2, "big"))  # min_blocksize
    streaminfo.extend((4096).to_bytes(2, "big"))  # max_blocksize
    streaminfo.extend(b"\x00\x00\x00")  # min_framesize
    streaminfo.extend(b"\x00\x00\x00")  # max_framesize

    # Sample rate: 44100 Hz (20 bits) = 0xAC44
    # Channels: 2 (stored as channels-1 = 1, 3 bits)
    # Bits per sample: 16 (stored as bps-1 = 15, 5 bits)
    # Total samples: 0 (36 bits)
    # Packed into 8 bytes total

    # Byte layout for the tricky part:
    # Byte 10-11: sample_rate[19:4] (16 bits)
    # Byte 12: sample_rate[3:0] (4 bits) | channels-1 (3 bits) | bps-1[4] (1 bit)
    # Byte 13: bps-1[3:0] (4 bits) | total_samples[35:32] (4 bits)
    # Byte 14-17: total_samples[31:0] (32 bits)

    sample_rate = 44100
    channels_minus_1 = 1  # 2 channels
    bps_minus_1 = 15  # 16 bits per sample
    total_samples = 0

    streaminfo.extend((sample_rate >> 4).to_bytes(2, "big"))
    streaminfo.append(((sample_rate & 0xF) << 4) | (channels_minus_1 << 1) | (bps_minus_1 >> 4))
    streaminfo.append(((bps_minus_1 & 0xF) << 4) | (total_samples >> 32))
    streaminfo.extend((total_samples & 0xFFFFFFFF).to_bytes(4, "big"))
    streaminfo.extend(b"\x00" * 16)  # MD5

    # Build FLAC file: "fLaC" + metadata block header + STREAMINFO
    # Block header: [last_block(1bit) | type(7bit)] [length(24bit)]
    # last_block=1, type=0 (STREAMINFO)
    block_header = bytes([0x80, 0x00, 0x00, 0x22])  # 0x80 = last block, type 0; 0x22 = 34 bytes
    flac_data = b"fLaC" + block_header + bytes(streaminfo)
    path.write_bytes(flac_data)

    # Create test fixtures
    show = Show(
        container_id=1,
        artist_name="Phish",
        container_info="2024-08-31 Dick's",
        venue_name="Dick's Sporting Goods Park",
        venue_city="Commerce City",
        venue_state="CO",
        performance_date="2024-08-31",
        performance_date_formatted="08/31/2024",
        performance_date_year="2024",
        tracks=[],
    )
    track = Track(
        track_id=1,
        song_id=1,
        song_title="Tweezer",
        track_num=1,
        disc_num=1,
        set_num=1,
    )

    # Tag the file
    tag_track(path, show, track)

    # Read back and verify tags
    audio = FLAC(path)
    assert audio["title"] == ["Tweezer"]
    assert audio["artist"] == ["Phish"]
    assert audio["album"] == ["2024-08-31 Dick's"]
    assert audio["tracknumber"] == ["1"]
    assert audio["discnumber"] == ["1"]
    assert audio["date"] == ["2024-08-31"]
    assert audio["venue"] == ["Dick's Sporting Goods Park, Commerce City, CO"]


def test_tag_graceful_failure(tmp_path):
    """Tagging a non-audio file should not raise an exception."""
    path = tmp_path / "not_audio.flac"
    path.write_text("not a real flac file")

    show = Show(
        container_id=1,
        artist_name="Phish",
        container_info="2024-08-31 Dick's",
        venue_name="Dick's",
        venue_city="Commerce City",
        venue_state="CO",
        performance_date="2024-08-31",
        performance_date_formatted="08/31/2024",
        performance_date_year="2024",
        tracks=[],
    )
    track = Track(
        track_id=1,
        song_id=1,
        song_title="Tweezer",
        track_num=1,
        disc_num=1,
        set_num=1,
    )

    # Should not raise - failures are logged as warnings
    tag_track(path, show, track)


def test_tag_nonexistent_file(tmp_path):
    """Tagging a nonexistent file should not raise an exception."""
    path = tmp_path / "does_not_exist.flac"

    show = Show(
        container_id=1,
        artist_name="Phish",
        container_info="2024-08-31 Dick's",
        venue_name="Dick's",
        venue_city="Commerce City",
        venue_state="CO",
        performance_date="2024-08-31",
        performance_date_formatted="08/31/2024",
        performance_date_year="2024",
        tracks=[],
    )
    track = Track(
        track_id=1,
        song_id=1,
        song_title="Tweezer",
        track_num=1,
        disc_num=1,
        set_num=1,
    )

    # Should not raise - failures are logged as warnings
    tag_track(path, show, track)
