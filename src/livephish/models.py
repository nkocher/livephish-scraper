"""Data models for LivePhish API responses."""

from __future__ import annotations

import re
from dataclasses import dataclass, field


_WINDOWS_RESERVED = frozenset({
    "con", "prn", "nul", "aux",
    *(f"com{i}" for i in range(1, 10)),
    *(f"lpt{i}" for i in range(1, 10)),
})


def sanitize_filename(name: str, max_length: int = 200) -> str:
    """Sanitize a string for use as a cross-platform filename.

    Handles: unsafe characters, Windows reserved names, trailing dots/spaces,
    and length truncation.
    """
    sanitized = re.sub(r'[\\/:*?"<>|]', "_", name)
    sanitized = sanitized.rstrip(". ")
    base = sanitized.split(".")[0]
    if base.lower() in _WINDOWS_RESERVED:
        sanitized = f"_{sanitized}"
    return sanitized[:max_length]


@dataclass
class StreamParams:
    """Parameters needed to construct stream URL requests."""

    subscription_id: str
    sub_costplan_id_access_list: str
    user_id: str
    start_stamp: str
    end_stamp: str


@dataclass
class Quality:
    """Audio quality/format info derived from stream URL."""

    code: str
    specs: str
    extension: str

    @staticmethod
    def from_stream_url(url: str) -> Quality | None:
        quality_map = {
            ".alac16/": Quality(
                code="alac",
                specs="16-bit / 44.1 kHz ALAC",
                extension=".m4a",
            ),
            ".flac16/": Quality(
                code="flac",
                specs="16-bit / 44.1 kHz FLAC",
                extension=".flac",
            ),
            ".aac150/": Quality(
                code="aac",
                specs="AAC 150",
                extension=".m4a",
            ),
        }
        for pattern, quality in quality_map.items():
            if pattern in url:
                return quality
        return None


@dataclass
class Track:
    """A single track within a show."""

    track_id: int
    song_id: int
    song_title: str
    track_num: int
    disc_num: int
    set_num: int
    duration_seconds: int = 0
    duration_display: str = ""

    @classmethod
    def from_dict(cls, data: dict) -> Track:
        return cls(
            track_id=data["trackID"],
            song_id=data.get("songID", 0),
            song_title=data.get("songTitle", ""),
            track_num=data.get("trackNum", 0),
            disc_num=data.get("discNum", 1),
            set_num=data.get("setNum", 0),
            duration_seconds=data.get("totalRunningTime", 0),
            duration_display=data.get("hhmmssTotalRunningTime", ""),
        )


@dataclass
class Show:
    """A show/container from the catalog."""

    container_id: int
    artist_name: str
    container_info: str
    venue_name: str
    venue_city: str
    venue_state: str
    performance_date: str
    performance_date_formatted: str
    performance_date_year: str
    total_duration_seconds: int = 0
    total_duration_display: str = ""
    tracks: list[Track] = field(default_factory=list)
    songs: list[dict] = field(default_factory=list)
    image_url: str = ""

    @classmethod
    def from_dict(cls, data: dict) -> Show:
        """Parse from API response dict (works for both catalog list and detail)."""
        tracks = [Track.from_dict(t) for t in data.get("tracks", data.get("Tracks", []))]
        songs_raw = data.get("songs", [])
        songs = songs_raw if isinstance(songs_raw, list) else []

        img = data.get("img", {})
        image_url = img.get("url", "") if isinstance(img, dict) else ""

        return cls(
            container_id=data.get("containerID", 0),
            artist_name=data.get("artistName", ""),
            container_info=data.get("containerInfo", "").strip(),
            venue_name=data.get("venueName", ""),
            venue_city=data.get("venueCity", ""),
            venue_state=data.get("venueState", ""),
            performance_date=data.get("performanceDate", ""),
            performance_date_formatted=data.get("performanceDateFormatted", ""),
            performance_date_year=data.get("performanceDateYear", ""),
            total_duration_seconds=data.get("totalContainerRunningTime", 0),
            total_duration_display=data.get("hhmmssTotalRunningTime", ""),
            tracks=tracks,
            songs=songs,
            image_url=image_url,
        )

    @property
    def display_date(self) -> str:
        """Short date like '2024-08-31'."""
        return self.performance_date_formatted or self.performance_date

    @property
    def display_location(self) -> str:
        parts = [self.venue_name]
        if self.venue_city:
            parts.append(self.venue_city)
        if self.venue_state:
            parts.append(self.venue_state)
        return ", ".join(parts)

    @property
    def folder_name(self) -> str:
        """Sanitized folder name for downloads."""
        raw = f"{self.artist_name} - {self.container_info}"
        return sanitize_filename(raw, max_length=120)

    def sets_grouped(self) -> dict[int, list[Track]]:
        """Group tracks by set number."""
        sets: dict[int, list[Track]] = {}
        for track in self.tracks:
            sets.setdefault(track.set_num, []).append(track)
        return sets


@dataclass
class CatalogShow:
    """Lightweight show entry from containersAll (no tracks)."""

    container_id: int
    artist_name: str
    container_info: str
    venue_name: str
    venue_city: str
    venue_state: str
    performance_date: str
    performance_date_formatted: str
    performance_date_year: str
    image_url: str = ""
    song_list: str = ""

    @classmethod
    def from_dict(cls, data: dict) -> CatalogShow:
        img = data.get("img", {})
        image_url = img.get("url", "") if isinstance(img, dict) else ""
        song_list = data.get("songList", "") or ""

        return cls(
            container_id=data.get("containerID", 0),
            artist_name=data.get("artistName", ""),
            container_info=data.get("containerInfo", "").strip(),
            venue_name=data.get("venueName", ""),
            venue_city=data.get("venueCity", ""),
            venue_state=data.get("venueState", ""),
            performance_date=data.get("performanceDate", ""),
            performance_date_formatted=data.get("performanceDateFormatted", ""),
            performance_date_year=data.get("performanceDateYear", ""),
            image_url=image_url,
            song_list=song_list if isinstance(song_list, str) else "",
        )

    @property
    def display_date(self) -> str:
        return self.performance_date_formatted or self.performance_date

    @property
    def display_location(self) -> str:
        parts = [self.venue_name]
        if self.venue_city:
            parts.append(self.venue_city)
        if self.venue_state:
            parts.append(self.venue_state)
        return ", ".join(parts)


# Format codes matching the Go downloader's resolveFormat map
FORMAT_CODES = {
    "flac": 4,    # 16-bit / 44.1 kHz FLAC
    "alac": 2,    # 16-bit / 44.1 kHz ALAC
    "aac": 3,     # AAC 150
}

FORMAT_LABELS = {
    "flac": "16-bit / 44.1 kHz FLAC",
    "alac": "16-bit / 44.1 kHz ALAC",
    "aac": "AAC 150",
}
