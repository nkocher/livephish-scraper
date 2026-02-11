"""Tests for InquirerPy browser — mock prompts to verify logic."""

from unittest.mock import MagicMock, patch

import pytest

from livephish.browser import (
    BACK,
    browse_by_year,
    download_queued_shows,
    edit_settings,
    main_menu,
    manage_queue,
    search_shows,
    show_detail,
)
from livephish.config import Config
from livephish.models import CatalogShow, Show, StreamParams, Track


@pytest.fixture(autouse=True)
def _mock_console():
    """Prevent console.input() from blocking and console.clear() from emitting ANSI."""
    with patch("livephish.browser.console") as mock_console:
        yield mock_console


@pytest.fixture
def stream_params():
    return StreamParams(
        subscription_id="123",
        sub_costplan_id_access_list="456",
        user_id="789",
        start_stamp="2024-01-01",
        end_stamp="2025-01-01",
    )


@pytest.fixture
def config(tmp_path):
    return Config(
        email="test@example.com",
        format="flac",
        output_dir=str(tmp_path / "output"),
    )


@pytest.fixture
def catalog_show():
    return CatalogShow(
        container_id=1001,
        artist_name="Phish",
        container_info="2024-12-31 MSG",
        venue_name="Madison Square Garden",
        venue_city="New York",
        venue_state="NY",
        performance_date="2024-12-31",
        performance_date_formatted="12/31/2024",
        performance_date_year="2024",
    )


@pytest.fixture
def full_show():
    return Show(
        container_id=1001,
        artist_name="Phish",
        container_info="2024-12-31 MSG",
        venue_name="Madison Square Garden",
        venue_city="New York",
        venue_state="NY",
        performance_date="2024-12-31",
        performance_date_formatted="12/31/2024",
        performance_date_year="2024",
        total_duration_display="3:12:45",
        tracks=[
            Track(
                track_id=1,
                song_id=100,
                song_title="Tweezer",
                track_num=1,
                disc_num=1,
                set_num=1,
                duration_seconds=960,
                duration_display="16:00",
            ),
        ],
    )


@pytest.fixture
def mock_catalog(catalog_show):
    catalog = MagicMock()
    catalog.get_years.return_value = ["2024", "2023"]
    catalog.get_shows_by_year.return_value = [catalog_show]
    catalog.search.return_value = [catalog_show]
    return catalog


@pytest.fixture
def mock_api(full_show):
    api = MagicMock()
    api.get_show_detail.return_value = full_show
    api.get_stream_url.return_value = ""
    return api


# -- Main menu -----------------------------------------------------------------


@patch("livephish.browser.inquirer")
def test_main_menu_browse(mock_inq):
    mock_inq.select.return_value.execute.return_value = "browse"
    assert main_menu(0) == "browse"


@patch("livephish.browser.inquirer")
def test_main_menu_quit(mock_inq):
    mock_inq.select.return_value.execute.return_value = "quit"
    assert main_menu(0) == "quit"


@patch("livephish.browser.inquirer")
def test_main_menu_all_actions(mock_inq):
    """Each menu option maps to correct action string."""
    for action in ("browse", "search", "queue", "settings", "quit"):
        mock_inq.select.return_value.execute.return_value = action
        assert main_menu(0) == action


# -- Browse by year ------------------------------------------------------------


@patch("livephish.browser.inquirer")
def test_browse_by_year_back(mock_inq, mock_catalog, mock_api, stream_params, config):
    """Selecting Back returns to main menu without selecting a show."""
    mock_inq.fuzzy.return_value.execute.return_value = BACK
    browse_by_year(mock_catalog, mock_api, stream_params, config, {})
    mock_api.get_show_detail.assert_not_called()


@patch("livephish.browser.inquirer")
def test_browse_by_year_calls_catalog(
    mock_inq, mock_catalog, mock_api, stream_params, config
):
    """browse_by_year uses catalog.get_years() and get_shows_by_year()."""
    # First fuzzy call: select year "2024", second: select BACK from show list
    mock_inq.fuzzy.return_value.execute.side_effect = ["2024", BACK]
    browse_by_year(mock_catalog, mock_api, stream_params, config, {})
    mock_catalog.get_years.assert_called_once()
    mock_catalog.get_shows_by_year.assert_called_with("2024")


@patch("livephish.browser.inquirer")
def test_browse_empty_catalog(mock_inq, mock_api, stream_params, config):
    """Empty catalog shows message without prompting."""
    catalog = MagicMock()
    catalog.get_years.return_value = []
    browse_by_year(catalog, mock_api, stream_params, config, {})
    mock_inq.fuzzy.assert_not_called()


# -- Search --------------------------------------------------------------------


@patch("livephish.browser.inquirer")
def test_search_uses_catalog_search(
    mock_inq, mock_catalog, mock_api, stream_params, config
):
    """search_shows calls catalog.search() with user query."""
    mock_inq.text.return_value.execute.return_value = "msg"
    mock_inq.fuzzy.return_value.execute.return_value = BACK
    search_shows(mock_catalog, mock_api, stream_params, config, {})
    mock_catalog.search.assert_called_once_with("msg")


@patch("livephish.browser.inquirer")
def test_search_empty_query(mock_inq, mock_catalog, mock_api, stream_params, config):
    """Empty search query returns immediately."""
    mock_inq.text.return_value.execute.return_value = "  "
    search_shows(mock_catalog, mock_api, stream_params, config, {})
    mock_catalog.search.assert_not_called()


@patch("livephish.browser.inquirer")
def test_search_no_results(mock_inq, mock_api, stream_params, config):
    """Empty result set prints message and returns."""
    catalog = MagicMock()
    catalog.search.return_value = []
    mock_inq.text.return_value.execute.return_value = "zzzznonexistent"
    search_shows(catalog, mock_api, stream_params, config, {})
    mock_inq.fuzzy.assert_not_called()


# -- Show detail ---------------------------------------------------------------


@patch("livephish.browser.inquirer")
def test_show_detail_fetches_from_api(
    mock_inq, mock_api, catalog_show, stream_params, config
):
    """show_detail calls api.get_show_detail for the selected show."""
    mock_inq.select.return_value.execute.return_value = "back"
    show_detail(catalog_show, mock_api, stream_params, config, {})
    mock_api.get_show_detail.assert_called_once_with(catalog_show.container_id)


@patch("livephish.browser.inquirer")
def test_show_detail_add_to_queue(
    mock_inq, mock_api, catalog_show, stream_params, config
):
    """Selecting 'add' adds the show to the queue."""
    mock_inq.select.return_value.execute.return_value = "add"
    queue: dict[int, CatalogShow] = {}
    show_detail(catalog_show, mock_api, stream_params, config, queue)
    assert catalog_show.container_id in queue


@patch("livephish.browser.inquirer")
def test_show_detail_remove_from_queue(
    mock_inq, mock_api, catalog_show, stream_params, config
):
    """Selecting 'remove' removes the show from the queue."""
    mock_inq.select.return_value.execute.return_value = "remove"
    queue = {catalog_show.container_id: catalog_show}
    show_detail(catalog_show, mock_api, stream_params, config, queue)
    assert catalog_show.container_id not in queue


@patch("livephish.browser.inquirer")
def test_show_detail_api_error(
    mock_inq, catalog_show, stream_params, config
):
    """API error in get_show_detail is handled gracefully."""
    api = MagicMock()
    api.get_show_detail.side_effect = Exception("Connection failed")
    show_detail(catalog_show, api, stream_params, config, {})
    # Should not prompt for action since detail fetch failed
    mock_inq.select.assert_not_called()


# -- Queue management ----------------------------------------------------------


@patch("livephish.browser.inquirer")
def test_queue_empty(mock_inq, mock_api, stream_params, config):
    """Empty queue shows message without prompting."""
    manage_queue({}, mock_api, stream_params, config)
    mock_inq.select.assert_not_called()


@patch("livephish.browser.inquirer")
def test_queue_back(mock_inq, mock_api, catalog_show, stream_params, config):
    """Selecting Back returns to main menu."""
    mock_inq.select.return_value.execute.return_value = "back"
    queue = {catalog_show.container_id: catalog_show}
    manage_queue(queue, mock_api, stream_params, config)
    assert len(queue) == 1  # Queue unchanged


@patch("livephish.browser.inquirer")
def test_queue_clear(mock_inq, mock_api, catalog_show, stream_params, config):
    """Clearing queue removes all shows."""
    mock_inq.select.return_value.execute.return_value = "clear"
    mock_inq.confirm.return_value.execute.return_value = True
    queue = {catalog_show.container_id: catalog_show}
    manage_queue(queue, mock_api, stream_params, config)
    assert len(queue) == 0


@patch("livephish.browser.inquirer")
def test_queue_remove_show(mock_inq, mock_api, catalog_show, stream_params, config):
    """Removing a specific show from queue works."""
    # First select call: "remove", second select call: the show id,
    # then queue is empty so exits via console.input (mocked)
    mock_inq.select.return_value.execute.side_effect = [
        "remove",
        catalog_show.container_id,
    ]
    queue = {catalog_show.container_id: catalog_show}
    manage_queue(queue, mock_api, stream_params, config)
    assert catalog_show.container_id not in queue


# -- Queue add/remove (unit) ---------------------------------------------------


def test_queue_add_and_remove():
    """Queue adds by container_id, removes correctly, dedupes."""
    queue: dict[int, CatalogShow] = {}
    show = CatalogShow(
        container_id=42,
        artist_name="Phish",
        container_info="2024-12-31",
        venue_name="MSG",
        venue_city="New York",
        venue_state="NY",
        performance_date="2024-12-31",
        performance_date_formatted="12/31/2024",
        performance_date_year="2024",
    )

    # Add
    queue[show.container_id] = show
    assert 42 in queue
    assert len(queue) == 1

    # Dedupe — adding same key again doesn't increase count
    queue[show.container_id] = show
    assert len(queue) == 1

    # Remove
    del queue[show.container_id]
    assert 42 not in queue
    assert len(queue) == 0


# -- Download ------------------------------------------------------------------


@patch("livephish.browser.download_show")
@patch("livephish.browser._resolve_stream_url")
def test_queue_download_dispatches(
    mock_resolve, mock_download, mock_api, catalog_show, stream_params, config, full_show
):
    """download_queued_shows resolves streams and calls download_show."""
    mock_resolve.return_value = "https://stream.example.com/track.flac16/file.flac"
    queue = {catalog_show.container_id: catalog_show}
    download_queued_shows(queue, mock_api, stream_params, config)
    mock_api.get_show_detail.assert_called_once_with(catalog_show.container_id)
    mock_download.assert_called_once()


@patch("livephish.browser.download_show")
@patch("livephish.browser._resolve_stream_url")
def test_queue_download_warns_on_format_fallback(
    mock_resolve,
    mock_download,
    _mock_console,
    mock_api,
    catalog_show,
    stream_params,
    config,
):
    """When API returns a different codec, download proceeds with warning."""
    mock_resolve.return_value = "https://stream.example.com/path/.alac16/track.m4a"
    config.format = "flac"
    queue = {catalog_show.container_id: catalog_show}

    download_queued_shows(queue, mock_api, stream_params, config)

    mock_download.assert_called_once()
    printed = [str(call.args[0]) for call in _mock_console.print.call_args_list if call.args]
    assert any("Requested 16-bit / 44.1 kHz FLAC, API returned 1x 16-bit / 44.1 kHz ALAC" in line for line in printed)
    assert any("Format fallbacks: requested 16-bit / 44.1 kHz FLAC" in line for line in printed)


@patch("livephish.browser.download_show")
@patch("livephish.browser._resolve_stream_url")
def test_queue_download_reports_unknown_format_skip(
    mock_resolve,
    mock_download,
    _mock_console,
    mock_api,
    catalog_show,
    stream_params,
    config,
):
    """Unknown stream formats are skipped and reported."""
    mock_resolve.return_value = "https://stream.example.com/path/unknown/track.mp3"
    queue = {catalog_show.container_id: catalog_show}

    download_queued_shows(queue, mock_api, stream_params, config)

    mock_download.assert_not_called()
    printed = [str(call.args[0]) for call in _mock_console.print.call_args_list if call.args]
    assert any("Skipped 1 track with unknown stream format." in line for line in printed)


@patch("livephish.browser.download_show")
@patch("livephish.browser._resolve_stream_url")
def test_queue_download_skips_failed_shows(
    mock_resolve, mock_download, catalog_show, stream_params, config
):
    """Shows that fail to fetch are skipped, queue not cleared."""
    api = MagicMock()
    api.get_show_detail.side_effect = Exception("timeout")
    queue = {catalog_show.container_id: catalog_show}
    download_queued_shows(queue, api, stream_params, config)
    mock_download.assert_not_called()
    assert len(queue) == 1  # Queue not cleared on failure


# -- Back navigation -----------------------------------------------------------


@patch("livephish.browser.inquirer")
def test_back_navigation_year(mock_inq, mock_catalog, mock_api, stream_params, config):
    """Selecting Back in year list returns without error."""
    mock_inq.fuzzy.return_value.execute.return_value = BACK
    browse_by_year(mock_catalog, mock_api, stream_params, config, {})
    # No exception means success


@patch("livephish.browser.inquirer")
def test_back_navigation_search(
    mock_inq, mock_catalog, mock_api, stream_params, config
):
    """Selecting Back in search results returns without error."""
    mock_inq.text.return_value.execute.return_value = "msg"
    mock_inq.fuzzy.return_value.execute.return_value = BACK
    search_shows(mock_catalog, mock_api, stream_params, config, {})


# -- Escape key (skip → None) -------------------------------------------------


@patch("livephish.browser.inquirer")
def test_escape_browse_year(mock_inq, mock_catalog, mock_api, stream_params, config):
    """Escape in year list returns to main menu (None from skip)."""
    mock_inq.fuzzy.return_value.execute.return_value = None
    browse_by_year(mock_catalog, mock_api, stream_params, config, {})
    mock_api.get_show_detail.assert_not_called()


@patch("livephish.browser.inquirer")
def test_escape_search_text(mock_inq, mock_catalog, mock_api, stream_params, config):
    """Escape in search text input returns without searching."""
    mock_inq.text.return_value.execute.return_value = None
    search_shows(mock_catalog, mock_api, stream_params, config, {})
    mock_catalog.search.assert_not_called()


@patch("livephish.browser.inquirer")
def test_escape_show_detail(mock_inq, mock_api, catalog_show, stream_params, config):
    """Escape in show detail action returns without modifying queue."""
    mock_inq.select.return_value.execute.return_value = None
    queue: dict[int, CatalogShow] = {}
    show_detail(catalog_show, mock_api, stream_params, config, queue)
    assert len(queue) == 0


@patch("livephish.browser.inquirer")
def test_escape_queue_action(mock_inq, mock_api, catalog_show, stream_params, config):
    """Escape in queue action menu returns without modifying queue."""
    mock_inq.select.return_value.execute.return_value = None
    queue = {catalog_show.container_id: catalog_show}
    manage_queue(queue, mock_api, stream_params, config)
    assert len(queue) == 1


@patch("livephish.browser.save_config")
@patch("livephish.browser.inquirer")
def test_escape_settings_cancels(mock_inq, mock_save, config):
    """Escape in settings format select cancels without saving."""
    mock_inq.select.return_value.execute.return_value = None
    edit_settings(config)
    mock_save.assert_not_called()


# -- Settings ------------------------------------------------------------------


@patch("livephish.browser.save_config")
@patch("livephish.browser.inquirer")
def test_settings_saves_config(mock_inq, mock_save, config):
    """edit_settings calls save_config with updated values."""
    mock_inq.select.return_value.execute.return_value = "alac"
    mock_inq.text.return_value.execute.return_value = "/tmp/new-output"
    edit_settings(config)
    assert config.format == "alac"
    assert config.output_dir == "/tmp/new-output"
    mock_save.assert_called_once_with(config)
