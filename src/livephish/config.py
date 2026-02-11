"""Configuration management for LivePhish scraper."""

from __future__ import annotations

import json
import time
from dataclasses import asdict, dataclass
from pathlib import Path
import keyring
import platformdirs
import yaml


@dataclass
class Config:
    """User configuration for LivePhish scraper."""

    email: str = ""
    format: str = "flac"  # One of: "flac", "alac", "aac"
    output_dir: str = "~/Music/LivePhish"


# Configuration paths
CONFIG_DIR = Path(platformdirs.user_config_dir("livephish"))
CACHE_DIR = Path(platformdirs.user_cache_dir("livephish"))
CONFIG_FILE = CONFIG_DIR / "config.yaml"

# Keyring service name
KEYRING_SERVICE = "livephish"

# Session cache
SESSION_CACHE_FILE = CACHE_DIR / "session.json"


def load_config() -> Config:
    """
    Load configuration from YAML file.

    Creates default config and necessary directories if they don't exist.

    Returns:
        Config: The loaded or default configuration
    """
    # Ensure directories exist
    CONFIG_DIR.mkdir(parents=True, exist_ok=True)
    CACHE_DIR.mkdir(parents=True, exist_ok=True)

    # One-time migration from old config path
    if not CONFIG_FILE.exists():
        old_config = Path("~/.config/livephish/config.yaml").expanduser()
        if old_config.exists():
            import shutil
            shutil.copy2(old_config, CONFIG_FILE)
            # Also migrate cache
            old_cache = Path("~/.config/livephish/cache/catalog.json").expanduser()
            if old_cache.exists():
                cache_file = CACHE_DIR / "catalog.json"
                shutil.copy2(old_cache, cache_file)

    # Load or create config
    if CONFIG_FILE.exists():
        with open(CONFIG_FILE, "r") as f:
            data = yaml.safe_load(f) or {}
            return Config(**{k: v for k, v in data.items() if k in Config.__annotations__})
    else:
        # Create default config
        config = Config()
        save_config(config)
        return config


def save_config(config: Config) -> None:
    """
    Save configuration to YAML file.

    Note: Password is never written to the config file. Use save_credentials() instead.

    Args:
        config: Configuration to save
    """
    CONFIG_DIR.mkdir(parents=True, exist_ok=True)

    # Convert config to dict and remove any sensitive data
    config_dict = asdict(config)

    with open(CONFIG_FILE, "w") as f:
        yaml.safe_dump(config_dict, f, default_flow_style=False, sort_keys=False)


def get_credentials(config: Config) -> tuple[str, str]:
    """
    Get email and password from config and keyring.

    Args:
        config: Configuration containing email

    Returns:
        tuple[str, str]: (email, password) from config and keyring

    Raises:
        ValueError: If email or password is not configured
    """
    email = config.email
    if not email:
        raise ValueError("Email not configured. Run 'livephish config' to set up.")

    # Try keyring first
    try:
        password = keyring.get_password(KEYRING_SERVICE, email)
        if password:
            return email, password
    except Exception:
        pass  # Keyring unavailable (headless Linux, etc.)

    # Fallback: prompt for password
    from getpass import getpass
    password = getpass(f"Password for {email}: ")
    if not password:
        raise ValueError("Password required.")
    return email, password


def save_credentials(email: str, password: str) -> None:
    """
    Save credentials to keyring.

    Args:
        email: User email (used as keyring username)
        password: User password (stored securely in system keyring)
    """
    try:
        keyring.set_password(KEYRING_SERVICE, email, password)
    except Exception:
        import logging
        logging.getLogger(__name__).warning(
            "Could not save password to system keyring. "
            "You may need to enter your password each time."
        )


def load_session_cache() -> dict | None:
    """Load cached session tokens if still valid (24h TTL)."""
    if not SESSION_CACHE_FILE.exists():
        return None
    try:
        data = json.loads(SESSION_CACHE_FILE.read_text())
        if time.time() - data.get("cached_at", 0) > 86400:
            return None
        return data
    except (json.JSONDecodeError, KeyError):
        return None


def save_session_cache(
    access_token: str,
    session_token: str,
    stream_params_dict: dict,
) -> None:
    """Save session tokens to cache file."""
    data = {
        "access_token": access_token,
        "session_token": session_token,
        "stream_params": stream_params_dict,
        "cached_at": time.time(),
    }
    CACHE_DIR.mkdir(parents=True, exist_ok=True)
    SESSION_CACHE_FILE.write_text(json.dumps(data))


def clear_session_cache() -> None:
    """Remove cached session."""
    if SESSION_CACHE_FILE.exists():
        SESSION_CACHE_FILE.unlink()


def first_run_wizard() -> Config:
    """Interactive setup wizard using plain input/getpass (no InquirerPy)."""
    from getpass import getpass

    print("\n  LivePhish — First Run Setup\n")

    email = input("  Email: ").strip()
    if not email or "@" not in email:
        raise ValueError("Invalid email address")

    password = getpass("  Password: ")
    if not password:
        raise ValueError("Password required")

    print("\n  Audio format:")
    print("    1) FLAC (lossless, recommended)")
    print("    2) ALAC (lossless, Apple)")
    print("    3) AAC (lossy, smaller files)")
    fmt_choice = input("  Choice [1]: ").strip() or "1"
    format_map = {"1": "flac", "2": "alac", "3": "aac"}
    audio_format = format_map.get(fmt_choice, "flac")

    output_dir = input("  Output directory [~/Music/LivePhish]: ").strip()
    output_dir = output_dir or "~/Music/LivePhish"

    config = Config(email=email, format=audio_format, output_dir=output_dir)
    save_config(config)
    save_credentials(email, password)

    print(f"\n  Config saved to {CONFIG_FILE}")
    print(f"  Password stored in system keyring")
    print(f"  Downloads will go to {Path(output_dir).expanduser()}\n")
    return config
