# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Setup

```bash
uv sync --all-extras
```

## Commands

```bash
uv run livephish              # Launch InquirerPy browser (default)
uv run livephish config       # Configure credentials (plain input wizard)
uv run livephish refresh      # Force-refresh catalog cache
uv run pytest -v              # Run all tests
uv run pytest tests/test_api.py -v              # Single test file
uv run pytest tests/test_api.py::test_authenticate_success -v  # Single test
bash scripts/build.sh              # Build standalone macOS binary → dist/LivePhish-macOS.zip
```

## Architecture

### Data Flow

```
CLI (click) → API auth (cached) → Catalog (cached) → InquirerPy Browser → Downloader → Tagger
```

The tool uses a **3-phase auth** ported from the Go LivePhish-Downloader:
1. OAuth2 password grant → `access_token` (via `id.livephish.com`)
2. Legacy session token → `tokenValue` (via `secureApi.aspx`)
3. Subscriber info → `StreamParams` (subscription ID, user ID, date stamps)

Auth tokens are cached in `session.json` (24h TTL) — `api.login_cached()` validates with an auth-required API call before reusing, falls back to full 3-step auth on failure.

Stream URLs require an **MD5 signature**: `md5(SIG_KEY + str(epoch_timestamp + epoch_compensation))`. The `epoch_compensation` value (typically 3) compensates for server clock skew — the CLI auto-retries values [3, 5, 7, 10] if the first attempt returns an empty stream link.

### Browser (browser.py)

Step-by-step InquirerPy prompt flow with Rich display:
- **Main menu**: Browse by year, Search shows, Download queue, Settings, Refresh catalog, Quit
- **Browse**: `inquirer.fuzzy` year list → show list → Rich Panel detail → action (add to queue / download / back)
- **Search**: `inquirer.text` → `catalog.search()` fuzzy results → same show list/detail flow
- **Queue**: `dict[int, CatalogShow]` keyed by container_id (natural deduplication). Download all, remove individual, clear.
- **Navigation**: `"← Back"` as last Choice in every list. `console.clear()` at each navigation level prevents prompt pile-up.

Downloads use `download_show()` from `downloader.py` (Rich progress bars). Stream URLs resolved via `_resolve_stream_url()` which retries epoch compensation values [3, 5, 7, 10].

**Do not run `uv run livephish`** in a headless/agent context — it launches an interactive prompt that will hang.

### Fuzzy Search (catalog.py)

Precomputed search index built once during `_build_indexes()`:
- Each show's corpus includes: venue name, venue abbreviation (e.g. "MSG"), city, state code, full state name, dates, song list
- All lowercased for case-insensitive matching
- Uses `rapidfuzz.fuzz.WRatio` scorer with `score_cutoff=40`
- `_abbreviate()` generates acronyms from multi-word venue names

### Two Show Types

- **`CatalogShow`** — lightweight entries from `catalog.containersAll` (no tracks, no song list in practice). Used for browsing/filtering.
- **`Show`** — full detail from `catalog.container` (includes tracks). Fetched on-demand when user selects a show.

### API Quirks

- Base URL `www.livephish.com` redirects 301 → `streamapi.livephish.com` — httpx client has `follow_redirects=True`
- `secureApi.aspx` serves multiple methods via `?method=` query param (session.getUserToken, user.getSubscriberInfo)
- Stream URL endpoint requires User-Agent `"LivePhishAndroid"` (different from the rest of the client)
- Catalog pagination is 1-indexed with `startOffset` param
- `songList` field in `containersAll` response is unreliable (often empty)
- Format API ignores `platformID` — returns whatever format is available. `Quality.from_stream_url()` detects actual format.

### Config & Credentials

- Config file: `platformdirs.user_config_dir("livephish")/config.yaml` (YAML, no secrets)
  - macOS: `~/Library/Application Support/livephish/config.yaml`
  - Linux: `~/.config/livephish/config.yaml`
  - Windows: `%APPDATA%/livephish/config.yaml`
- Passwords: system keyring via `keyring` library (service name: `"livephish"`), falls back to `getpass` prompt
- Session cache: `platformdirs.user_cache_dir("livephish")/session.json` (24h TTL)
- Catalog cache: `platformdirs.user_cache_dir("livephish")/catalog.json` (7-day TTL, auto-refreshes if < 50 shows)
- Downloads: `~/Music/LivePhish/` (configurable)
- One-time migration from old `~/.config/livephish/` paths on first run

### Testing

Tests use **respx** for HTTP mocking (not pytest-httpx despite it being in dev deps). API tests mock `secureApi.aspx` with `side_effect` functions that route by `?method=` param. Shared fixtures in `conftest.py` provide sample API response dicts. Catalog tests use a `MockAPI` class for self-contained testing without HTTP.

### File Sanitization

Single `models.sanitize_filename()` handles both show folders (120 char limit) and track filenames (200 char limit). Strips `\/:*?"<>|`, trailing dots/spaces, and prefixes Windows reserved names (CON, PRN, COM1-9, LPT1-9) with `_`.

## Conventions

- Sync httpx only (no async) — sequential CLI has no benefit from async
- 0.5s rate limiting between API requests, 3x retry with exponential backoff
- `.part` file safety: downloads write to `.part`, rename on completion
- `mutagen` for audio tagging: FLAC uses Vorbis comments, M4A uses MP4 atoms
- Format codes from Go downloader: `flac=4, alac=2, aac=3`
- Browser tests mock InquirerPy via `@patch("livephish.browser.inquirer")` with `autouse` console fixture to prevent blocking
- `download_show()` is the Rich-based download function; `download_track_raw()` is the callback-driven variant (available for custom integrations)
- Catalog auto-heals: cache with < `MIN_CATALOG_SIZE` (50) shows is treated as incomplete and triggers automatic re-fetch
