"""Click CLI for LivePhish — launches InquirerPy browser by default."""

from __future__ import annotations

import sys

import click

from livephish import __version__


def _launch_browser(force_login: bool = False) -> None:
    """Auth, load catalog, launch InquirerPy browser."""
    from livephish.api import LivePhishAPI
    from livephish.config import Config, first_run_wizard, get_credentials, load_config

    config = load_config()
    if not config.email:
        print("No configuration found. Running setup wizard...\n")
        config = first_run_wizard()

    api = LivePhishAPI()
    try:
        email, password = get_credentials(config)

        if force_login:
            from livephish.config import clear_session_cache
            clear_session_cache()

        stream_params, status = api.login_cached(email, password)
        print(f"Signed in — {status}")

        from livephish.catalog import Catalog

        catalog = Catalog(api)
        catalog.load()

        from livephish.browser import run_browser

        run_browser(catalog, api, stream_params, config)
    finally:
        api.close()


@click.group(invoke_without_command=True)
@click.version_option(version=__version__, prog_name="livephish")
@click.option("--force-login", "-f", is_flag=True, help="Force fresh authentication")
@click.pass_context
def cli(ctx, force_login):
    """LivePhish — browse and download live concert recordings."""
    if ctx.invoked_subcommand is None:
        try:
            _launch_browser(force_login=force_login)
        except KeyboardInterrupt:
            print("\nInterrupted.")
        except Exception as e:
            from livephish.api import AuthError, SubscriptionError, APIError

            if isinstance(e, AuthError):
                print(f"Authentication failed: {e}", file=sys.stderr)
                print("Run 'livephish config' to update credentials.", file=sys.stderr)
                sys.exit(1)
            elif isinstance(e, SubscriptionError):
                print(f"Subscription error: {e}", file=sys.stderr)
                sys.exit(1)
            elif isinstance(e, APIError):
                print(f"Connection error: {e}", file=sys.stderr)
                print("Check your internet connection and try again.", file=sys.stderr)
                sys.exit(1)
            elif isinstance(e, ValueError):
                print(f"{e}", file=sys.stderr)
                print("Run 'livephish config' to set up credentials.", file=sys.stderr)
                sys.exit(1)
            else:
                raise


@cli.command()
def config():
    """Configure credentials and preferences."""
    from livephish.config import first_run_wizard

    try:
        first_run_wizard()
    except KeyboardInterrupt:
        print("\nCancelled.")


@cli.command()
def refresh():
    """Force-refresh the catalog cache."""
    from livephish.api import APIError, AuthError, LivePhishAPI
    from livephish.config import get_credentials, load_config, first_run_wizard

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
        print("\nInterrupted.")
    except AuthError as e:
        print(f"Authentication failed: {e}", file=sys.stderr)
        print("Run 'livephish config' to update credentials.", file=sys.stderr)
        sys.exit(1)
    except APIError as e:
        print(f"Connection error: {e}", file=sys.stderr)
        print("Check your internet connection and try again.", file=sys.stderr)
        sys.exit(1)
    except ValueError as e:
        print(f"{e}", file=sys.stderr)
        sys.exit(1)
