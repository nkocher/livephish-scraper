"""Catalog management: full fetch, JSON cache, and search indexes."""

from __future__ import annotations

import json
import logging
import time
from pathlib import Path

from rich.console import Console

from livephish.api import LivePhishAPI
from livephish.config import CACHE_DIR
from livephish.models import CatalogShow

logger = logging.getLogger(__name__)
console = Console()

CACHE_FILE = CACHE_DIR / "catalog.json"
CACHE_TTL_DAYS = 7


class Catalog:
    def __init__(self, api: LivePhishAPI) -> None:
        self.api = api
        self.shows: list[CatalogShow] = []
        self._by_year: dict[str, list[CatalogShow]] = {}
        self._by_venue: dict[str, list[CatalogShow]] = {}

    def load(self) -> None:
        """Load from cache or fetch from API."""
        cached = self._load_cache()
        if cached is not None:
            self.shows = cached
        else:
            self.shows = self.fetch_all()
        self._build_indexes()

    def fetch_all(self) -> list[CatalogShow]:
        """Paginate through catalog.containersAll, cache results."""
        all_shows: list[CatalogShow] = []
        offset = 1
        with console.status("[bold green]Fetching catalog...") as status:
            while True:
                containers = self.api.get_catalog_page(offset=offset, limit=100)
                if not containers:
                    break
                for c in containers:
                    all_shows.append(CatalogShow.from_dict(c))
                offset += len(containers)
                status.update(f"[bold green]Fetching catalog... {len(all_shows)} shows")
        self._save_cache(all_shows)
        console.print(f"[green]Catalog loaded: {len(all_shows)} shows[/green]")
        return all_shows

    def _save_cache(self, shows: list[CatalogShow]) -> None:
        """Save catalog to JSON cache file."""
        CACHE_DIR.mkdir(parents=True, exist_ok=True)
        data = []
        for show in shows:
            data.append({
                "containerID": show.container_id,
                "artistName": show.artist_name,
                "containerInfo": show.container_info,
                "venueName": show.venue_name,
                "venueCity": show.venue_city,
                "venueState": show.venue_state,
                "performanceDate": show.performance_date,
                "performanceDateFormatted": show.performance_date_formatted,
                "performanceDateYear": show.performance_date_year,
                "img": {"url": show.image_url},
                "songList": show.song_list,
            })
        CACHE_FILE.write_text(json.dumps(data, indent=2))

    def _load_cache(self) -> list[CatalogShow] | None:
        """Load from cache if valid (< 7 days old)."""
        if not CACHE_FILE.exists():
            return None
        age_days = (time.time() - CACHE_FILE.stat().st_mtime) / 86400
        if age_days > CACHE_TTL_DAYS:
            return None
        try:
            data = json.loads(CACHE_FILE.read_text())
            shows = [CatalogShow.from_dict(d) for d in data]
            console.print(f"[dim]Loaded {len(shows)} shows from cache[/dim]")
            return shows
        except (json.JSONDecodeError, KeyError):
            return None

    def _build_indexes(self) -> None:
        """Build year and venue indexes."""
        self._by_year = {}
        self._by_venue = {}
        for show in self.shows:
            year = show.performance_date_year or "Unknown"
            self._by_year.setdefault(year, []).append(show)
            venue_key = show.venue_name.lower()
            if venue_key:
                self._by_venue.setdefault(venue_key, []).append(show)

    def get_years(self) -> list[str]:
        """Get available years sorted descending."""
        return sorted(self._by_year.keys(), reverse=True)

    def get_shows_by_year(self, year: str) -> list[CatalogShow]:
        """Get shows for a year sorted by date descending."""
        shows = self._by_year.get(year, [])
        return sorted(shows, key=lambda s: s.performance_date or "", reverse=True)

    def search(self, query: str) -> list[CatalogShow]:
        """Search shows by venue, city, state, date, or songs. Word-based matching."""
        query_words = query.lower().split()
        results = []
        for show in self.shows:
            searchable = " ".join([
                show.venue_name,
                show.venue_city,
                show.venue_state,
                show.performance_date,
                show.performance_date_formatted,
                show.container_info,
                show.song_list,
            ]).lower()
            if all(word in searchable for word in query_words):
                results.append(show)
        return sorted(results, key=lambda s: s.performance_date or "", reverse=True)
