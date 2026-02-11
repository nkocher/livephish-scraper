"""Click CLI for LivePhish — launches InquirerPy browser by default."""

from __future__ import annotations

import sys

import click
from rich.console import Console

from livephish import __version__
from livephish.api import APIError, AuthError, LivePhishAPI, SubscriptionError
from livephish.config import Config, first_run_wizard, get_credentials, load_config

console = Console()


def _launch_browser() -> None:
    """Auth, load catalog, launch InquirerPy browser."""
    config = load_config()
    if not config.email:
        console.print("[yellow]No configuration found. Running setup wizard...[/yellow]\n")
        config = first_run_wizard()

    api = LivePhishAPI()
    try:
        email, password = get_credentials(config)
        stream_params, status = api.login_cached(email, password)
        console.print(f"[green]Signed in — {status}[/green]")

        from livephish.catalog import Catalog

        catalog = Catalog(api)
        catalog.load()

        from livephish.browser import run_browser

        run_browser(catalog, api, stream_params, config)
    finally:
        api.close()


@click.group(invoke_without_command=True)
@click.version_option(version=__version__, prog_name="livephish")
@click.pass_context
def cli(ctx):
    """LivePhish — browse and download live concert recordings."""
    if ctx.invoked_subcommand is None:
        try:
            _launch_browser()
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
        cfg = load_config()
        if not cfg.email:
            cfg = first_run_wizard()

        from livephish.catalog import Catalog

        api = LivePhishAPI()
        try:
            email, password = get_credentials(cfg)
            api.login_cached(email, password)
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
