# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Setup

```bash
uv sync --all-extras
```

## Commands

```bash
uv run livephish              # Interactive browser (default, invokes browse)
uv run livephish browse       # Same as above
uv run livephish search "MSG" # Search by venue/date/song
uv run livephish download <url> [<url>...]  # Direct URL download
uv run livephish config       # Configure credentials
uv run livephish refresh      # Force-refresh catalog cache
uv run pytest -v              # Run all tests
uv run pytest tests/test_api.py -v              # Single test file
uv run pytest tests/test_api.py::test_authenticate_success -v  # Single test
bash scripts/build.sh              # Build standalone macOS binary → dist/LivePhish-macOS.zip
```

## Architecture

### Data Flow

```
CLI (click) → API auth → Catalog (cached) → Browser (InquirerPy) → Downloader → Tagger
```

The tool uses a **3-phase auth** ported from the Go LivePhish-Downloader:
1. OAuth2 password grant → `access_token` (via `id.livephish.com`)
2. Legacy session token → `tokenValue` (via `secureApi.aspx`)
3. Subscriber info → `StreamParams` (subscription ID, user ID, date stamps)

Stream URLs require an **MD5 signature**: `md5(SIG_KEY + str(epoch_timestamp + epoch_compensation))`. The `epoch_compensation` value (typically 3) compensates for server clock skew — the CLI auto-retries values [3, 5, 7, 10] if the first attempt returns an empty stream link.

### Two Show Types

- **`CatalogShow`** — lightweight entries from `catalog.containersAll` (no tracks, no song list in practice). Used for browsing/filtering.
- **`Show`** — full detail from `catalog.container` (includes tracks). Fetched on-demand when user selects a show.

### API Quirks

- Base URL `www.livephish.com` redirects 301 → `streamapi.livephish.com` — httpx client has `follow_redirects=True`
- `secureApi.aspx` serves multiple methods via `?method=` query param (session.getUserToken, user.getSubscriberInfo)
- Stream URL endpoint requires User-Agent `"LivePhishAndroid"` (different from the rest of the client)
- Catalog pagination is 1-indexed with `startOffset` param
- `songList` field in `containersAll` response is unreliable (often empty)

### Config & Credentials

- Config file: `platformdirs.user_config_dir("livephish")/config.yaml` (YAML, no secrets)
  - macOS: `~/Library/Application Support/livephish/config.yaml`
  - Linux: `~/.config/livephish/config.yaml`
  - Windows: `%APPDATA%/livephish/config.yaml`
- Passwords: system keyring via `keyring` library (service name: `"livephish"`), falls back to `getpass` prompt
- Catalog cache: `platformdirs.user_cache_dir("livephish")/catalog.json` (7-day TTL)
- Downloads: `~/Music/LivePhish/` (configurable)
- One-time migration from old `~/.config/livephish/` paths on first run

### Testing

Tests use **respx** for HTTP mocking (not pytest-httpx despite it being in dev deps). API tests mock `secureApi.aspx` with `side_effect` functions that route by `?method=` param. Shared fixtures in `conftest.py` provide sample API response dicts.

**Do not run `uv run livephish browse`** — it launches an interactive terminal prompt that will hang.

### File Sanitization

Single `models.sanitize_filename()` handles both show folders (120 char limit) and track filenames (200 char limit). Strips `\/:*?"<>|`, trailing dots/spaces, and prefixes Windows reserved names (CON, PRN, COM1-9, LPT1-9) with `_`.

## Conventions

- Sync httpx only (no async) — sequential CLI has no benefit from async
- 0.5s rate limiting between API requests, 3x retry with exponential backoff
- `.part` file safety: downloads write to `.part`, rename on completion
- `mutagen` for audio tagging: FLAC uses Vorbis comments, M4A uses MP4 atoms
- Format codes from Go downloader: `flac=4, alac=2, aac=3`
