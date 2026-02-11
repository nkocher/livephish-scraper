"""Audio metadata tagging with mutagen."""

import logging
from pathlib import Path

from livephish.models import Show, Track

logger = logging.getLogger(__name__)


def tag_track(path: Path, show: Show, track: Track) -> None:
    """Tag an audio file with show and track metadata.

    Args:
        path: Path to the audio file (.flac or .m4a)
        show: Show containing album/venue information
        track: Track containing song title and track number
    """
    try:
        if path.suffix.lower() == ".flac":
            import mutagen.flac

            audio = mutagen.flac.FLAC(path)
            audio["title"] = track.song_title
            audio["artist"] = show.artist_name
            audio["album"] = show.container_info
            audio["date"] = show.performance_date
            audio["tracknumber"] = str(track.track_num)
            audio["discnumber"] = str(track.disc_num)
            audio["venue"] = show.display_location
            audio.save()
            logger.debug(f"Tagged FLAC: {path.name}")

        elif path.suffix.lower() == ".m4a":
            import mutagen.mp4

            audio = mutagen.mp4.MP4(path)
            audio["\xa9nam"] = [track.song_title]
            audio["\xa9ART"] = [show.artist_name]
            audio["\xa9alb"] = [show.container_info]
            audio["\xa9day"] = [show.performance_date]
            audio["trkn"] = [(track.track_num, len(show.tracks))]
            audio["disk"] = [(track.disc_num, 1)]
            audio.save()
            logger.debug(f"Tagged M4A: {path.name}")

    except Exception as e:
        logger.warning(f"Failed to tag {path.name}: {e}")
