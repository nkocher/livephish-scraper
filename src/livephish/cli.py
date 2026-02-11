"""Click CLI for LivePhish interactive scraper."""

from __future__ import annotations

import re
import sys
from pathlib import Path

import click
from rich.console import Console

from livephish import __version__
from livephish.api import APIError, AuthError, LivePhishAPI, SubscriptionError
from livephish.config import Config, first_run_wizard, get_credentials, load_config
from livephish.models import FORMAT_CODES, Quality

console = Console()

# URL patterns matching the Go downloader
URL_PATTERNS = [
    re.compile(r"https://plus\.livephish\.com/(?:index\.html|)#/catalog/recording/(\d+)"),
    re.compile(r"https://www\.livephish\.com/browse/music/0,(\d+)/[\w-]+"),
]


def _extract_container_id(url: str) -> int | None:
    """Extract container ID from a LivePhish URL."""
    for pattern in URL_PATTERNS:
        match = pattern.search(url)
        if match:
            return int(match.group(1))
    return None


def _login(api: LivePhishAPI, config: Config) -> tuple:
    """Authenticate and return stream params."""
    email, password = get_credentials(config)
    stream_params, plan_name = api.login(email, password)
    console.print(f"[green]Signed in - {plan_name}[/green]\n")
    return stream_params, plan_name


def _download_shows(api, shows, stream_params, config):
    """Download a list of shows."""
    from livephish.downloader import download_show

    format_code = FORMAT_CODES[config.format]
    output_dir = Path(config.output_dir).expanduser()
    output_dir.mkdir(parents=True, exist_ok=True)

    total_tracks = 0
    for show in shows:
        console.print(f"\n[bold]{show.artist_name} - {show.display_date} {show.venue_name}[/bold]")

        # Get stream URLs for all tracks
        tracks_with_urls = []
        for track in show.tracks:
            try:
                stream_url = api.get_stream_url(
                    track.track_id, format_code, stream_params, config.epoch_compensation
                )
                if not stream_url:
                    # Try different epoch compensation values
                    for comp in [3, 5, 7, 10]:
                        stream_url = api.get_stream_url(
                            track.track_id, format_code, stream_params, comp
                        )
                        if stream_url:
                            break
                if not stream_url:
                    console.print(f"[yellow]No stream URL for: {track.song_title}[/yellow]")
                    continue
                quality = Quality.from_stream_url(stream_url)
                if not quality:
                    console.print(f"[yellow]Unsupported format in URL for: {track.song_title}[/yellow]")
                    continue
                tracks_with_urls.append((track, stream_url, quality))
            except APIError as e:
                console.print(f"[red]Error getting stream for {track.song_title}: {e}[/red]")

        if tracks_with_urls:
            download_show(show, tracks_with_urls, output_dir)
            total_tracks += len(tracks_with_urls)
            console.print(f"[green]Done: {show.folder_name}[/green]")

    # Download summary
    console.print(f"\n[bold green]Download complete![/bold green]")
    console.print(f"[cyan]Downloaded {total_tracks} tracks from {len(shows)} show(s)[/cyan]")
    console.print(f"[dim]Saved to: {output_dir}[/dim]")


@click.group(invoke_without_command=True)
@click.version_option(version=__version__, prog_name="livephish")
@click.pass_context
def cli(ctx):
    """LivePhish interactive show browser and downloader."""
    if ctx.invoked_subcommand is None:
        ctx.invoke(browse)


@cli.command()
def browse():
    """Interactive show browser."""
    from livephish.browser import browse_interactive, confirm_download
    from livephish.catalog import Catalog
    from rich.text import Text

    try:
        # Welcome banner
        banner = Text()
        banner.append("LivePhish", style="bold magenta")
        banner.append(f" v{__version__}", style="dim")
        banner.append(" — browse and download live recordings\n", style="")
        console.print(banner)

        config = load_config()
        if not config.email:
            console.print("[yellow]No configuration found. Running setup wizard...[/yellow]\n")
            config = first_run_wizard()

        api = LivePhishAPI()
        try:
            stream_params, plan_name = _login(api, config)

            catalog = Catalog(api)
            catalog.load()

            # Browse/download loop
            while True:
                selected_shows = browse_interactive(api, catalog, config)
                if not selected_shows:
                    break

                if confirm_download(selected_shows, config.format):
                    try:
                        _download_shows(api, selected_shows, stream_params, config)
                    except AuthError:
                        console.print("[dim]Session expired, re-authenticating...[/dim]")
                        stream_params, _ = _login(api, config)
                        _download_shows(api, selected_shows, stream_params, config)
                    except KeyboardInterrupt:
                        console.print("\n[yellow]Download interrupted. Returning to menu.[/yellow]")
                        continue
                console.print()
        finally:
            api.close()

    except KeyboardInterrupt:
        console.print("\n[dim]Interrupted.[/dim]")
    except AuthError as e:
        console.print(f"[red]Authentication failed: {e}[/red]")
        console.print("[dim]Run 'livephish config' to update credentials.[/dim]")
        sys.exit(1)
    except SubscriptionError as e:
        console.print(f"[red]Subscription error: {e}[/red]")
        sys.exit(1)
    except APIError as e:
        console.print(f"[red]Connection error: {e}[/red]")
        console.print("[dim]Check your internet connection and try again.[/dim]")
        sys.exit(1)
    except ValueError as e:
        console.print(f"[red]{e}[/red]")
        console.print("[dim]Run 'livephish config' to set up credentials.[/dim]")
        sys.exit(1)


@cli.command()
@click.argument("query")
def search(query):
    """Search shows by date, venue, or song."""
    from livephish.browser import confirm_download, display_show_panel
    from livephish.catalog import Catalog
    from InquirerPy import inquirer

    try:
        config = load_config()
        if not config.email:
            config = first_run_wizard()

        api = LivePhishAPI()
        try:
            stream_params, _ = _login(api, config)

            catalog = Catalog(api)
            catalog.load()

            results = catalog.search(query)
            if not results:
                console.print(f"[yellow]No shows found for '{query}'[/yellow]")
                return

            console.print(f"[green]Found {len(results)} shows matching '{query}'[/green]\n")

            choices = [
                {"name": f"{s.display_date}  {s.display_location}", "value": s}
                for s in results
            ]
            selected = inquirer.fuzzy(
                message="Select shows to download (tab to select, enter to confirm):",
                choices=choices,
                multiselect=True,
                instruction="(type to filter, tab to select)",
                mandatory=False,
                raise_keyboard_interrupt=False,
            ).execute()

            if not selected:
                return

            # Fetch full show details
            shows = []
            for catalog_show in selected:
                show = api.get_show_detail(catalog_show.container_id)
                display_show_panel(show)
                shows.append(show)

            if confirm_download(shows, config.format):
                _download_shows(api, shows, stream_params, config)
        finally:
            api.close()

    except KeyboardInterrupt:
        console.print("\n[dim]Interrupted.[/dim]")
    except AuthError as e:
        console.print(f"[red]Authentication failed: {e}[/red]")
        console.print("[dim]Run 'livephish config' to update credentials.[/dim]")
        sys.exit(1)
    except SubscriptionError as e:
        console.print(f"[red]Subscription error: {e}[/red]")
        sys.exit(1)
    except APIError as e:
        console.print(f"[red]Connection error: {e}[/red]")
        console.print("[dim]Check your internet connection and try again.[/dim]")
        sys.exit(1)
    except ValueError as e:
        console.print(f"[red]{e}[/red]")
        sys.exit(1)


@cli.command()
@click.argument("urls", nargs=-1, required=True)
def download(urls):
    """Download shows by URL (non-interactive)."""
    try:
        config = load_config()
        if not config.email:
            config = first_run_wizard()

        api = LivePhishAPI()
        try:
            stream_params, _ = _login(api, config)

            for url in urls:
                container_id = _extract_container_id(url)
                if not container_id:
                    console.print(f"[red]Invalid URL: {url}[/red]")
                    continue

                show = api.get_show_detail(container_id)
                _download_shows(api, [show], stream_params, config)
        finally:
            api.close()

    except KeyboardInterrupt:
        console.print("\n[dim]Interrupted.[/dim]")
    except AuthError as e:
        console.print(f"[red]Authentication failed: {e}[/red]")
        console.print("[dim]Run 'livephish config' to update credentials.[/dim]")
        sys.exit(1)
    except SubscriptionError as e:
        console.print(f"[red]Subscription error: {e}[/red]")
        sys.exit(1)
    except APIError as e:
        console.print(f"[red]Connection error: {e}[/red]")
        console.print("[dim]Check your internet connection and try again.[/dim]")
        sys.exit(1)
    except ValueError as e:
        console.print(f"[red]{e}[/red]")
        sys.exit(1)


@cli.command()
def config():
    """Configure credentials and preferences."""
    try:
        first_run_wizard()
    except KeyboardInterrupt:
        console.print("\n[dim]Cancelled.[/dim]")


@cli.command()
def refresh():
    """Force-refresh the catalog cache."""
    try:
        config = load_config()
        if not config.email:
            config = first_run_wizard()

        from livephish.catalog import Catalog

        api = LivePhishAPI()
        try:
            _login(api, config)
            catalog = Catalog(api)
            catalog.fetch_all()
        finally:
            api.close()

    except KeyboardInterrupt:
        console.print("\n[dim]Interrupted.[/dim]")
    except AuthError as e:
        console.print(f"[red]Authentication failed: {e}[/red]")
        console.print("[dim]Run 'livephish config' to update credentials.[/dim]")
        sys.exit(1)
    except APIError as e:
        console.print(f"[red]Connection error: {e}[/red]")
        console.print("[dim]Check your internet connection and try again.[/dim]")
        sys.exit(1)
    except ValueError as e:
        console.print(f"[red]{e}[/red]")
        sys.exit(1)
