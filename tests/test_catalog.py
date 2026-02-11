"""Tests for Catalog management with caching and search."""

import json
import time
import pytest
from pathlib import Path
from livephish.catalog import Catalog, CACHE_TTL_DAYS
from livephish.models import CatalogShow


@pytest.fixture
def tmp_cache(tmp_path, monkeypatch):
    """Fixture to use temporary cache directory."""
    cache_dir = tmp_path / "cache"
    cache_dir.mkdir()
    cache_file = cache_dir / "catalog.json"
    monkeypatch.setattr("livephish.catalog.CACHE_DIR", cache_dir)
    monkeypatch.setattr("livephish.catalog.CACHE_FILE", cache_file)
    return cache_dir


class MockAPI:
    """Mock API for testing Catalog without real HTTP calls."""

    def __init__(self):
        self._catalog_data = []
        self._returned = False

    def get_catalog_page(self, offset=1, limit=100):
        """Return mock catalog data once, then empty (simulates pagination end)."""
        if self._returned:
            return []
        self._returned = True
        return self._catalog_data

    def add_show(self, **kwargs):
        """Add a show to the mock catalog."""
        show_data = {
            "containerID": kwargs.get("container_id", 1),
            "artistName": kwargs.get("artist_name", "Phish"),
            "containerInfo": kwargs.get("container_info", "Test Venue"),
            "venueName": kwargs.get("venue_name", "Test Venue"),
            "venueCity": kwargs.get("venue_city", "Test City"),
            "venueState": kwargs.get("venue_state", "TS"),
            "performanceDate": kwargs.get("performance_date", "2024-01-01"),
            "performanceDateFormatted": kwargs.get(
                "performance_date_formatted", "January 1, 2024"
            ),
            "performanceDateYear": kwargs.get("performance_date_year", "2024"),
            "img": {"url": kwargs.get("image_url", "")},
            "songList": kwargs.get("song_list", ""),
        }
        self._catalog_data.append(show_data)


def test_cache_save_load_round_trip(tmp_cache, monkeypatch):
    """Test saving and loading catalog cache."""
    monkeypatch.setattr("livephish.catalog.MIN_CATALOG_SIZE", 0)
    mock_api = MockAPI()
    mock_api.add_show(
        container_id=1,
        venue_name="Madison Square Garden",
        performance_date="2024-08-31",
        performance_date_year="2024",
    )
    mock_api.add_show(
        container_id=2,
        venue_name="Sphere",
        performance_date="2024-04-18",
        performance_date_year="2024",
    )

    # Create catalog and fetch shows
    catalog1 = Catalog(mock_api)
    catalog1.shows = catalog1.fetch_all()

    assert len(catalog1.shows) == 2

    # Create new catalog instance and load from cache
    catalog2 = Catalog(mock_api)
    catalog2.load()

    assert len(catalog2.shows) == 2
    assert catalog2.shows[0].container_id == 1
    assert catalog2.shows[0].venue_name == "Madison Square Garden"
    assert catalog2.shows[1].container_id == 2
    assert catalog2.shows[1].venue_name == "Sphere"


def test_cache_expiry(tmp_cache, monkeypatch):
    """Test cache expiry after TTL days."""
    mock_api = MockAPI()
    mock_api.add_show(container_id=1, venue_name="Old Venue")

    # Create catalog and save cache
    catalog = Catalog(mock_api)
    catalog.shows = catalog.fetch_all()

    cache_file = tmp_cache / "catalog.json"
    assert cache_file.exists()

    # Set mtime to 8 days ago (exceeds 7-day TTL)
    old_time = time.time() - (8 * 86400)
    import os

    os.utime(cache_file, (old_time, old_time))

    # Create new catalog and try to load - should return None
    catalog2 = Catalog(mock_api)
    cached_data = catalog2._load_cache()

    assert cached_data is None


def test_search_by_venue(tmp_cache):
    """Test searching shows by venue name."""
    mock_api = MockAPI()
    mock_api.add_show(container_id=1, venue_name="Madison Square Garden", venue_city="New York")
    mock_api.add_show(container_id=2, venue_name="Sphere", venue_city="Las Vegas")
    mock_api.add_show(container_id=3, venue_name="The Garden", venue_city="Boston")

    catalog = Catalog(mock_api)
    catalog.load()

    results = catalog.search("Madison Square Garden")
    assert len(results) >= 1
    assert results[0].venue_name == "Madison Square Garden"


def test_search_fuzzy_abbreviation(tmp_cache):
    """Test fuzzy search handles abbreviations like MSG."""
    mock_api = MockAPI()
    mock_api.add_show(container_id=1, venue_name="Madison Square Garden", venue_city="New York", venue_state="NY")
    mock_api.add_show(container_id=2, venue_name="Sphere", venue_city="Las Vegas", venue_state="NV")

    catalog = Catalog(mock_api)
    catalog.load()

    results = catalog.search("msg")
    assert len(results) >= 1
    assert results[0].venue_name == "Madison Square Garden"


def test_search_state_name_expansion(tmp_cache):
    """Test that searching by full state name matches state abbreviations."""
    mock_api = MockAPI()
    mock_api.add_show(container_id=1, venue_name="Cuthbert Amphitheater", venue_city="Eugene", venue_state="OR")
    mock_api.add_show(container_id=2, venue_name="Red Rocks", venue_city="Morrison", venue_state="CO")

    catalog = Catalog(mock_api)
    catalog.load()

    results = catalog.search("oregon")
    assert len(results) >= 1
    assert results[0].venue_state == "OR"


def test_search_empty_query(tmp_cache):
    """Test that empty query returns no results."""
    mock_api = MockAPI()
    mock_api.add_show(container_id=1, venue_name="Test Venue")

    catalog = Catalog(mock_api)
    catalog.load()

    assert catalog.search("") == []
    assert catalog.search("   ") == []


def test_search_by_date(tmp_cache):
    """Test searching shows by performance date."""
    mock_api = MockAPI()
    mock_api.add_show(
        container_id=1,
        venue_name="Venue A",
        performance_date="2024-08-31",
        performance_date_year="2024",
    )
    mock_api.add_show(
        container_id=2,
        venue_name="Venue B",
        performance_date="2024-04-18",
        performance_date_year="2024",
    )

    catalog = Catalog(mock_api)
    catalog.load()

    # Search for specific date — fuzzy search may return partial matches
    # but the exact date match should be first
    results = catalog.search("2024-08-31")

    assert len(results) >= 1
    assert results[0].performance_date == "2024-08-31"


def test_search_case_insensitive(tmp_cache):
    """Test case-insensitive search."""
    mock_api = MockAPI()
    mock_api.add_show(container_id=1, venue_name="Madison Square Garden")
    mock_api.add_show(container_id=2, venue_name="The Sphere")

    catalog = Catalog(mock_api)
    catalog.load()

    results = catalog.search("madison")
    assert len(results) >= 1
    assert results[0].venue_name == "Madison Square Garden"


def test_get_years_sorted_descending(tmp_cache):
    """Test getting years sorted in descending order."""
    mock_api = MockAPI()
    mock_api.add_show(container_id=1, performance_date_year="2020")
    mock_api.add_show(container_id=2, performance_date_year="2024")
    mock_api.add_show(container_id=3, performance_date_year="2022")

    catalog = Catalog(mock_api)
    catalog.load()

    years = catalog.get_years()

    assert years == ["2024", "2022", "2020"]


def test_get_shows_by_year_sorted(tmp_cache):
    """Test getting shows by year sorted by date descending."""
    mock_api = MockAPI()
    mock_api.add_show(
        container_id=1,
        performance_date="2024-08-31",
        performance_date_year="2024",
    )
    mock_api.add_show(
        container_id=2,
        performance_date="2024-04-18",
        performance_date_year="2024",
    )
    mock_api.add_show(
        container_id=3,
        performance_date="2024-12-31",
        performance_date_year="2024",
    )
    mock_api.add_show(
        container_id=4,
        performance_date="2023-01-01",
        performance_date_year="2023",
    )

    catalog = Catalog(mock_api)
    catalog.load()

    shows_2024 = catalog.get_shows_by_year("2024")

    assert len(shows_2024) == 3
    # Should be sorted descending by date
    assert shows_2024[0].performance_date == "2024-12-31"
    assert shows_2024[1].performance_date == "2024-08-31"
    assert shows_2024[2].performance_date == "2024-04-18"


def test_search_word_based(tmp_cache):
    """Test search with multiple terms — top result has best match."""
    mock_api = MockAPI()
    mock_api.add_show(
        container_id=1,
        venue_name="Dick's Sporting Goods Park",
        venue_city="Commerce City",
        venue_state="CO",
        performance_date="2024-08-31",
        performance_date_formatted="August 31, 2024",
        performance_date_year="2024",
    )
    mock_api.add_show(
        container_id=2,
        venue_name="Red Rocks Amphitheatre",
        venue_city="Morrison",
        venue_state="CO",
        performance_date="2024-07-15",
        performance_date_formatted="July 15, 2024",
        performance_date_year="2024",
    )
    mock_api.add_show(
        container_id=3,
        venue_name="Dick's Sporting Goods Park",
        venue_city="Commerce City",
        venue_state="CO",
        performance_date="2023-09-01",
        performance_date_formatted="September 1, 2023",
        performance_date_year="2023",
    )

    catalog = Catalog(mock_api)
    catalog.load()

    results = catalog.search("dick 2024")
    assert len(results) >= 1
    # Top result should be a Dick's 2024 show
    assert results[0].container_id == 1


def test_search_multi_word(tmp_cache):
    """Test multi-word search returns relevant results."""
    mock_api = MockAPI()
    mock_api.add_show(
        container_id=1,
        venue_name="Dick's Sporting Goods Park",
        venue_city="Commerce City",
        venue_state="CO",
        performance_date="2024-08-31",
        performance_date_year="2024",
    )
    mock_api.add_show(
        container_id=2,
        venue_name="Red Rocks Amphitheatre",
        venue_city="Morrison",
        venue_state="CO",
        performance_date="2024-07-15",
        performance_date_year="2024",
    )

    catalog = Catalog(mock_api)
    catalog.load()

    results = catalog.search("commerce city")
    assert len(results) >= 1
    assert results[0].venue_city == "Commerce City"


def test_search_no_match(tmp_cache):
    """Test search with no matching results."""
    mock_api = MockAPI()
    mock_api.add_show(
        container_id=1,
        venue_name="Madison Square Garden",
        venue_city="New York",
        venue_state="NY",
        performance_date="2024-08-31",
        performance_date_year="2024",
    )

    catalog = Catalog(mock_api)
    catalog.load()

    # Search for something with zero overlap with any corpus content
    results = catalog.search("xyzzyqwkjhplm")

    assert len(results) == 0
