"""Interactive show browsing with InquirerPy and Rich display."""

from __future__ import annotations

from typing import TYPE_CHECKING

from InquirerPy import inquirer
from rich.console import Console
from rich.panel import Panel
from rich.text import Text

from livephish.models import CatalogShow, Show, FORMAT_LABELS

if TYPE_CHECKING:
    from livephish.api import LivePhishAPI
    from livephish.catalog import Catalog
    from livephish.config import Config

console = Console()


def browse_interactive(
    api: LivePhishAPI, catalog: Catalog, config: Config
) -> list[Show] | None:
    """
    Main browse loop for selecting shows.

    Args:
        api: LivePhish API client
        catalog: Show catalog with search/filter capabilities
        config: User configuration

    Returns:
        List of Show objects with tracks loaded, or None if cancelled
    """
    while True:
        # Main menu
        action = inquirer.select(
            message="Select action:",
            choices=[
                "Browse shows by year",
                "Search for shows",
                "Settings",
                "Exit",
            ],
            mandatory=False,
            raise_keyboard_interrupt=False,
        ).execute()

        if action is None or action == "Exit":
            return None

        if action == "Settings":
            _settings_menu(config)
            continue

        selected_catalog_shows: list[CatalogShow] = []

        if action == "Browse shows by year":
            # Get years and show year picker
            years = catalog.get_years()
            if not years:
                console.print("[yellow]No shows available in catalog[/yellow]")
                continue

            # Build year choices with show counts
            year_choices = []
            for year in years:
                shows = catalog.get_shows_by_year(year)
                year_choices.append({
                    "name": f"{year}  ({len(shows)} shows)",
                    "value": year,
                })

            selected_year = inquirer.fuzzy(
                message="Select year:",
                choices=year_choices,
                instruction="(type to filter, esc to go back)",
                mandatory=False,
                raise_keyboard_interrupt=False,
            ).execute()

            if selected_year is None:
                continue

            # Get shows for selected year
            shows = catalog.get_shows_by_year(selected_year)
            if not shows:
                console.print(f"[yellow]No shows found for {selected_year}[/yellow]")
                continue

            # Show fuzzy picker with multiselect
            show_choices = [
                {
                    "name": f"{s.display_date}  {s.display_location}",
                    "value": s,
                }
                for s in shows
            ]

            selected_catalog_shows = inquirer.fuzzy(
                message="Select shows (tab to select, enter to confirm):",
                choices=show_choices,
                multiselect=True,
                instruction="(type to filter, tab to select, esc to go back)",
                mandatory=False,
                raise_keyboard_interrupt=False,
            ).execute()

            if selected_catalog_shows is None:
                continue

        elif action == "Search for shows":
            # Text search
            query = inquirer.text(
                message="Search:",
                instruction="(esc to go back)",
                mandatory=False,
                raise_keyboard_interrupt=False,
            ).execute()

            if not query or not query.strip():
                continue

            # Search catalog
            results = catalog.search(query)
            if not results:
                console.print(f"[yellow]No shows found matching '{query}'[/yellow]")
                continue

            # Show fuzzy picker with multiselect
            show_choices = [
                {
                    "name": f"{s.display_date}  {s.display_location}",
                    "value": s,
                }
                for s in results
            ]

            selected_catalog_shows = inquirer.fuzzy(
                message="Select shows (tab to select, enter to confirm):",
                choices=show_choices,
                multiselect=True,
                instruction="(type to filter, tab to select, esc to go back)",
                mandatory=False,
                raise_keyboard_interrupt=False,
            ).execute()

            if selected_catalog_shows is None:
                continue

        # If user selected shows, fetch details and display
        if selected_catalog_shows:
            full_shows: list[Show] = []

            for catalog_show in selected_catalog_shows:
                console.print(
                    f"[cyan]Fetching details for {catalog_show.display_date}...[/cyan]"
                )
                full_show = api.get_show_detail(catalog_show.container_id)
                full_shows.append(full_show)
                display_show_panel(full_show)

            return full_shows


def _settings_menu(config: Config) -> None:
    """
    Settings menu for changing format and output directory inline.

    Args:
        config: User configuration (will be updated in place)
    """
    from livephish.config import save_config

    while True:
        action = inquirer.select(
            message="Settings:",
            choices=[
                f"Audio format: {FORMAT_LABELS.get(config.format, config.format)}",
                f"Output directory: {config.output_dir}",
                "Back",
            ],
            mandatory=False,
            raise_keyboard_interrupt=False,
        ).execute()

        if action is None or action == "Back":
            return

        if action.startswith("Audio format:"):
            format_choices = [
                {"name": f"FLAC ({FORMAT_LABELS['flac']}) - Recommended", "value": "flac"},
                {"name": f"ALAC ({FORMAT_LABELS['alac']})", "value": "alac"},
                {"name": f"AAC ({FORMAT_LABELS['aac']})", "value": "aac"},
            ]

            new_format = inquirer.select(
                message="Select audio format:",
                choices=format_choices,
                default=config.format,
                mandatory=False,
                raise_keyboard_interrupt=False,
            ).execute()

            if new_format:
                config.format = new_format
                save_config(config)
                console.print(f"[green]Audio format updated to {FORMAT_LABELS[new_format]}[/green]")

        elif action.startswith("Output directory:"):
            new_dir = inquirer.text(
                message="Output directory:",
                default=config.output_dir,
                mandatory=False,
                raise_keyboard_interrupt=False,
            ).execute()

            if new_dir:
                config.output_dir = new_dir
                save_config(config)
                console.print(f"[green]Output directory updated to {new_dir}[/green]")


def display_show_panel(show: Show) -> None:
    """
    Display a Rich panel with show details.

    Args:
        show: Show object with tracks loaded
    """
    # Build content lines
    lines = []

    # Header line
    header = f"{show.venue_city}, {show.venue_state} · {show.total_duration_display}"
    lines.append(header)
    lines.append("")

    # Group tracks by set
    sets_grouped = show.sets_grouped()

    # Determine max normal set number (for encore detection)
    if sets_grouped:
        max_normal_set = max(
            (set_num for set_num in sets_grouped.keys() if set_num > 0),
            default=0,
        )
    else:
        max_normal_set = 0

    # Display each set
    for set_num in sorted(sets_grouped.keys()):
        tracks = sets_grouped[set_num]

        # Determine set label
        if set_num == 0 or (max_normal_set > 0 and set_num > max_normal_set):
            set_label = "Encore"
        else:
            set_label = f"Set {set_num}"

        lines.append(f"[bold cyan]{set_label}[/bold cyan]")

        # Display tracks with track number and duration
        for track in tracks:
            duration = track.duration_display or ""
            track_line = f"  {track.track_num:2d}. {track.song_title}"
            if duration:
                track_line += f" [{duration}]"
            lines.append(track_line)

        lines.append("")

    # Create panel
    content = "\n".join(lines)
    panel_title = f"{show.artist_name} · {show.display_date} · {show.venue_name}"

    panel = Panel(
        content,
        title=panel_title,
        border_style="blue",
    )

    console.print(panel)


def confirm_download(shows: list[Show], format_name: str) -> bool:
    """
    Confirm download action with user.

    Args:
        shows: List of shows to download
        format_name: Audio format code (e.g., "flac", "alac", "aac")

    Returns:
        True if user confirms, False otherwise
    """
    show_count = len(shows)
    show_word = "show" if show_count == 1 else "shows"
    format_label = FORMAT_LABELS.get(format_name, format_name)

    result = inquirer.confirm(
        message=f"Download {show_count} {show_word} in {format_label}?",
        default=True,
        mandatory=False,
        raise_keyboard_interrupt=False,
    ).execute()

    return result if result is not None else False
