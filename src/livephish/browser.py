"""Interactive browser using InquirerPy prompts and Rich display."""

from __future__ import annotations

import random
from collections import defaultdict
from pathlib import Path

from InquirerPy import inquirer
from InquirerPy.base.control import Choice
from rich.console import Console
from rich.panel import Panel
from rich.rule import Rule

from livephish.api import APIError, LivePhishAPI
from livephish.catalog import Catalog
from livephish.config import Config, save_config
from livephish.downloader import download_show
from livephish.models import (
    FORMAT_CODES,
    FORMAT_LABELS,
    CatalogShow,
    Quality,
    Show,
    StreamParams,
    Track,
)

console = Console()

BACK = "\u2190 Back"
_ESC_BACK = {"skip": [{"key": "escape"}]}
ResolvedTracks = list[tuple[Track, str, Quality]]

_BANNER = """[bold cyan]       ><(((º>[/bold cyan]
 [bold]L I V E P H I S H[/bold]"""

_LOADING_MESSAGES = [
    "Loading show details...",
    "Fetching the setlist...",
    "Consulting the Helping Friendly Book...",
    "Checking the stash...",
    "Reading the Gamehendge map...",
]

_ESCAPE_TIMEOUT = 0.1


def _fast_execute(prompt):
    """Execute prompt with reduced escape delay."""
    try:
        prompt.application.ttimeoutlen = _ESCAPE_TIMEOUT
    except AttributeError:
        pass  # prompt_toolkit version doesn't support this
    return prompt.execute()


def _print_banner():
    console.print(_BANNER)
    console.print()


def run_browser(
    catalog: Catalog,
    api: LivePhishAPI,
    stream_params: StreamParams,
    config: Config,
) -> None:
    """Main browser loop."""
    queue: dict[int, CatalogShow] = {}
    while True:
        console.clear()
        action = main_menu(len(queue))
        if action == "browse":
            browse_by_year(catalog, api, stream_params, config, queue)
        elif action == "search":
            search_shows(catalog, api, stream_params, config, queue)
        elif action == "queue":
            manage_queue(queue, api, stream_params, config)
        elif action == "settings":
            edit_settings(config)
        elif action == "refresh":
            console.clear()
            catalog.refresh()
        elif action == "quit":
            break


def main_menu(queue_count: int) -> str:
    """Main menu with InquirerPy select."""
    _print_banner()

    if queue_count:
        queue_label = f"Download queue ({queue_count} show{'s' if queue_count != 1 else ''})"
    else:
        queue_label = "Download queue"

    return _fast_execute(inquirer.select(
        message="LivePhish",
        choices=[
            Choice("browse", name="Browse by year"),
            Choice("search", name="Search shows"),
            Choice("queue", name=queue_label),
            Choice("settings", name="Settings"),
            Choice("refresh", name="Refresh catalog"),
            Choice("quit", name="Quit"),
        ],
        instruction="(arrows to navigate, enter to select)",
    ))


def browse_by_year(
    catalog: Catalog,
    api: LivePhishAPI,
    stream_params: StreamParams,
    config: Config,
    queue: dict[int, CatalogShow],
) -> None:
    """Year list -> show list -> show detail."""
    years = catalog.get_years()
    if not years:
        console.print("[yellow]No shows in catalog.[/yellow]")
        return

    choices = [
        Choice(year, name=f"{year} ({len(catalog.get_shows_by_year(year))} shows)")
        for year in years
    ]
    choices.append(Choice(BACK, name=BACK))

    year = _fast_execute(inquirer.fuzzy(
        message="Select year",
        choices=choices,
        instruction="(type to filter, esc to go back)",
        keybindings=_ESC_BACK,
        mandatory=False,
    ))

    if year is None or year == BACK:
        return

    shows = catalog.get_shows_by_year(year)
    _show_list(shows, api, stream_params, config, queue, title=year)


def search_shows(
    catalog: Catalog,
    api: LivePhishAPI,
    stream_params: StreamParams,
    config: Config,
    queue: dict[int, CatalogShow],
) -> None:
    """Text input -> fuzzy results -> show detail."""
    query = _fast_execute(inquirer.text(
        message="Search",
        instruction="(venue, city, state, date, song... esc to go back)",
        keybindings=_ESC_BACK,
        mandatory=False,
    ))

    if not query or not query.strip():
        return

    results = catalog.search(query)
    if not results:
        console.print("[yellow]No results found.[/yellow]")
        return

    _show_list(results, api, stream_params, config, queue, title=f'Search: "{query}"')


def _show_list(
    shows: list[CatalogShow],
    api: LivePhishAPI,
    stream_params: StreamParams,
    config: Config,
    queue: dict[int, CatalogShow],
    title: str = "",
) -> None:
    """Display a list of shows, let user select one for detail view."""
    while True:
        console.clear()
        header = title or "Shows"
        console.print(Rule(f" {header}  [dim]({len(shows)} shows)[/dim] ", style="dim"))
        console.print()

        choices = []
        for show in shows:
            prefix = "\u2713 " if show.container_id in queue else ""
            label = f"{prefix}{show.display_date} \u00b7 {show.display_location}"
            choices.append(Choice(show.container_id, name=label))
        choices.append(Choice(BACK, name=BACK))

        selected = _fast_execute(inquirer.fuzzy(
            message="Select show",
            choices=choices,
            instruction="(type to filter, esc to go back)",
            keybindings=_ESC_BACK,
            mandatory=False,
        ))

        if selected is None or selected == BACK:
            return

        catalog_show = next(s for s in shows if s.container_id == selected)
        show_detail(catalog_show, api, stream_params, config, queue)


def show_detail(
    catalog_show: CatalogShow,
    api: LivePhishAPI,
    stream_params: StreamParams,
    config: Config,
    queue: dict[int, CatalogShow],
) -> None:
    """Fetch full Show, display Rich panel, offer actions."""
    console.clear()
    console.print(f"[dim]{random.choice(_LOADING_MESSAGES)}[/dim]")

    try:
        show = api.get_show_detail(catalog_show.container_id)
    except Exception as e:
        console.print(f"[red]Error loading show: {e}[/red]")
        console.input("\n[dim]Press Enter to continue...[/dim]")
        return

    console.clear()
    _print_show_panel(show)

    in_queue = catalog_show.container_id in queue
    queue_action = "remove" if in_queue else "add"
    queue_label = "Remove from queue" if in_queue else "Add to queue"

    action = _fast_execute(inquirer.select(
        message="Action",
        choices=[
            Choice(queue_action, name=queue_label),
            Choice("download", name="Download now"),
            Choice("back", name=BACK),
        ],
        instruction="(esc to go back)",
        keybindings=_ESC_BACK,
        mandatory=False,
    ))

    if action == "add":
        queue[catalog_show.container_id] = catalog_show
        console.print(
            f"[green]Added to queue ({len(queue)} show{'s' if len(queue) != 1 else ''})[/green]"
        )
    elif action == "remove":
        del queue[catalog_show.container_id]
        console.print(
            f"[yellow]Removed from queue ({len(queue)} show{'s' if len(queue) != 1 else ''})[/yellow]"
        )
    elif action == "download":
        _download_single(show, api, stream_params, config)


def _print_show_panel(show: Show) -> None:
    """Display show details in a Rich panel."""
    lines: list[str] = []

    location = f"{show.venue_city}, {show.venue_state}" if show.venue_city else ""
    duration = f" \u00b7 {show.total_duration_display}" if show.total_duration_display else ""
    lines.append(f"{show.venue_name} \u00b7 {location}{duration}")
    lines.append("")

    sets_grouped = show.sets_grouped()
    if sets_grouped:
        max_normal_set = max((sn for sn in sets_grouped if sn > 0), default=0)
    else:
        max_normal_set = 0

    for set_num in sorted(sets_grouped):
        tracks = sets_grouped[set_num]
        if set_num == 0 or (max_normal_set > 0 and set_num > max_normal_set):
            set_label = "Encore"
        else:
            set_label = f"Set {set_num}"

        lines.append(f"[bold cyan]{set_label}[/bold cyan]")
        for track in tracks:
            dur = f"  [dim]{track.duration_display}[/dim]" if track.duration_display else ""
            lines.append(f"  {track.track_num:2d}. {track.song_title}{dur}")
        lines.append("")

    title = f"{show.artist_name} \u00b7 {show.display_date}"
    console.print(Panel("\n".join(lines), title=title, expand=False, padding=(1, 2)))


def manage_queue(
    queue: dict[int, CatalogShow],
    api: LivePhishAPI,
    stream_params: StreamParams,
    config: Config,
) -> None:
    """View queue, remove items, download all."""
    if not queue:
        console.print("[yellow]Queue is empty.[/yellow]")
        console.input("[dim]Press Enter to continue...[/dim]")
        return

    while True:
        console.clear()

        if not queue:
            console.print("[yellow]Queue is now empty.[/yellow]")
            console.input("[dim]Press Enter to continue...[/dim]")
            return

        console.print(Rule(f" {len(queue)} show{'s' if len(queue) != 1 else ''} queued ", style="bold green"))
        for show in queue.values():
            console.print(f"  \u2713 {show.display_date} \u00b7 {show.display_location}")
        console.print()

        action = _fast_execute(inquirer.select(
            message="Queue",
            choices=[
                Choice("download", name="Download all"),
                Choice("remove", name="Remove a show"),
                Choice("clear", name="Clear queue"),
                Choice("back", name=BACK),
            ],
            instruction="(esc to go back)",
            keybindings=_ESC_BACK,
            mandatory=False,
        ))

        if action is None or action == "back":
            return
        elif action == "download":
            download_queued_shows(queue, api, stream_params, config)
            return
        elif action == "remove":
            remove_choices = [
                Choice(
                    show.container_id,
                    name=f"{show.display_date} \u00b7 {show.display_location}",
                )
                for show in queue.values()
            ]
            remove_choices.append(Choice(BACK, name=BACK))

            to_remove = _fast_execute(inquirer.select(
                message="Remove which show?",
                choices=remove_choices,
                instruction="(esc to cancel)",
                keybindings=_ESC_BACK,
                mandatory=False,
            ))

            if to_remove is not None and to_remove != BACK:
                del queue[to_remove]
                console.print("[yellow]Removed.[/yellow]")
        elif action == "clear":
            if _fast_execute(inquirer.confirm(message="Clear entire queue?", default=False, instruction="(esc to cancel)", keybindings=_ESC_BACK, mandatory=False)):
                queue.clear()
                console.print("[yellow]Queue cleared.[/yellow]")
                return


def download_queued_shows(
    queue: dict[int, CatalogShow],
    api: LivePhishAPI,
    stream_params: StreamParams,
    config: Config,
) -> None:
    """Resolve streams and download all queued shows."""
    output_dir = Path(config.output_dir).expanduser()
    output_dir.mkdir(parents=True, exist_ok=True)
    format_code = FORMAT_CODES[config.format]

    total_shows = len(queue)
    downloaded_shows = 0
    skipped_shows = 0
    fallback_counts: dict[str, int] = defaultdict(int)
    unknown_format_skips = 0

    for i, catalog_show in enumerate(list(queue.values()), 1):
        console.print(
            f"\n[bold][{i}/{total_shows}] {catalog_show.display_date}"
            f" \u00b7 {catalog_show.display_location}[/bold]"
        )

        try:
            show = api.get_show_detail(catalog_show.container_id)
        except Exception as e:
            console.print(f"[red]  Error fetching show: {e}[/red]")
            skipped_shows += 1
            continue

        tracks_with_urls, mismatches, unknown = _resolve_tracks(
            show, api, stream_params, config.format, format_code
        )
        _print_resolution_warnings(config.format, mismatches, unknown, indent="  ")
        for code, count in mismatches.items():
            fallback_counts[code] += count
        unknown_format_skips += unknown

        if not tracks_with_urls:
            console.print("[yellow]  No downloadable tracks found.[/yellow]")
            skipped_shows += 1
            continue

        download_show(show, tracks_with_urls, output_dir)
        downloaded_shows += 1

    # Summary
    parts = [f"[bold green]Done![/bold green] {downloaded_shows}/{total_shows} shows downloaded"]
    if skipped_shows:
        parts.append(f"[yellow]{skipped_shows} skipped[/yellow]")
    console.print(", ".join(parts))
    if fallback_counts:
        details = ", ".join(
            f"{count}x {_format_label(code)}"
            for code, count in sorted(fallback_counts.items())
        )
        total_fallback_tracks = sum(fallback_counts.values())
        console.print(
            "[yellow]Format fallbacks: requested "
            f"{_format_label(config.format)}, downloaded {details} across "
            f"{total_fallback_tracks} track{'s' if total_fallback_tracks != 1 else ''}."
            "[/yellow]"
        )
    if unknown_format_skips:
        console.print(
            "[yellow]Skipped "
            f"{unknown_format_skips} track{'s' if unknown_format_skips != 1 else ''} "
            "with unknown stream format.[/yellow]"
        )

    # Clear queue only if all shows completed
    if skipped_shows == 0:
        queue.clear()

    console.input("\n[dim]Press Enter to continue...[/dim]")


def _download_single(
    show: Show,
    api: LivePhishAPI,
    stream_params: StreamParams,
    config: Config,
) -> None:
    """Download a single show immediately."""
    output_dir = Path(config.output_dir).expanduser()
    output_dir.mkdir(parents=True, exist_ok=True)
    format_code = FORMAT_CODES[config.format]

    console.print("[dim]Resolving stream URLs...[/dim]")
    tracks_with_urls, mismatches, unknown = _resolve_tracks(
        show, api, stream_params, config.format, format_code
    )
    _print_resolution_warnings(config.format, mismatches, unknown)

    if not tracks_with_urls:
        console.print("[red]No downloadable tracks found.[/red]")
        return

    download_show(show, tracks_with_urls, output_dir)
    console.print("[bold green]Download complete![/bold green]")


def _resolve_tracks(
    show: Show,
    api: LivePhishAPI,
    stream_params: StreamParams,
    requested_format: str,
    format_code: int,
) -> tuple[ResolvedTracks, dict[str, int], int]:
    """Resolve stream URLs for all tracks in a show."""
    results: ResolvedTracks = []
    mismatch_counts: dict[str, int] = defaultdict(int)
    unknown_formats = 0

    for track in show.tracks:
        url = _resolve_stream_url(api, track.track_id, format_code, stream_params)
        if not url:
            continue
        quality = Quality.from_stream_url(url)
        if not quality:
            unknown_formats += 1
            continue
        if quality.code != requested_format:
            mismatch_counts[quality.code] += 1
        results.append((track, url, quality))
    return results, dict(mismatch_counts), unknown_formats


def _format_label(format_code: str) -> str:
    """Human-readable label for configured audio format."""
    return FORMAT_LABELS.get(format_code, format_code.upper())


def _print_resolution_warnings(
    requested_format: str,
    mismatches: dict[str, int],
    unknown_count: int,
    indent: str = "",
) -> None:
    """Emit warnings when API returns a different or unknown format."""
    total_mismatches = sum(mismatches.values())
    if total_mismatches:
        details = ", ".join(
            f"{count}x {_format_label(code)}"
            for code, count in sorted(mismatches.items())
        )
        console.print(
            f"[yellow]{indent}Requested {_format_label(requested_format)}, API returned "
            f"{details}. Downloading returned format.[/yellow]"
        )
    if unknown_count:
        console.print(
            f"[yellow]{indent}Skipped {unknown_count} track"
            f"{'s' if unknown_count != 1 else ''} with unknown stream format.[/yellow]"
        )


def _resolve_stream_url(
    api: LivePhishAPI,
    track_id: int,
    format_code: int,
    stream_params: StreamParams,
) -> str:
    """Try multiple epoch compensation values to get a stream URL."""
    for comp in [3, 5, 7, 10]:
        try:
            url = api.get_stream_url(track_id, format_code, stream_params, comp)
            if url:
                return url
        except APIError:
            continue
    return ""


def edit_settings(config: Config) -> None:
    """Edit format and output directory settings."""
    console.clear()
    console.print(Rule(" Settings ", style="dim"))
    console.print()

    format_choices = [
        Choice(code, name=f"{label} ({code})")
        for code, label in FORMAT_LABELS.items()
    ]

    new_format = _fast_execute(inquirer.select(
        message="Audio format",
        choices=format_choices,
        default=config.format,
        instruction="(esc to cancel)",
        keybindings=_ESC_BACK,
        mandatory=False,
    ))

    if new_format is None:
        return

    new_dir = _fast_execute(inquirer.text(
        message="Output directory",
        default=config.output_dir,
        instruction="(esc to cancel)",
        keybindings=_ESC_BACK,
        mandatory=False,
    ))

    if new_dir is None:
        return

    config.format = new_format
    config.output_dir = new_dir
    save_config(config)
    console.print("[green]Settings saved.[/green]")
