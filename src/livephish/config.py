"""Configuration management for LivePhish scraper."""

from __future__ import annotations

from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

import keyring
import platformdirs
import yaml
from InquirerPy import inquirer

from livephish.models import FORMAT_LABELS


@dataclass
class Config:
    """User configuration for LivePhish scraper."""

    email: str = ""
    format: str = "flac"  # One of: "flac", "alac", "aac"
    output_dir: str = "~/Music/LivePhish"
    epoch_compensation: int = 0


# Configuration paths
CONFIG_DIR = Path(platformdirs.user_config_dir("livephish"))
CACHE_DIR = Path(platformdirs.user_cache_dir("livephish"))
CONFIG_FILE = CONFIG_DIR / "config.yaml"

# Keyring service name
KEYRING_SERVICE = "livephish"


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


def first_run_wizard() -> Config:
    """
    Interactive setup wizard for first-time configuration.

    Prompts user for:
    - Email address
    - Password (stored in keyring)
    - Audio format preference
    - Output directory

    Returns:
        Config: Newly created configuration
    """
    print("\n🎵 LivePhish Scraper - First Run Setup\n")

    # Ask for email
    email = inquirer.text(
        message="Email address:",
        validate=lambda x: "@" in x and len(x) > 3,
        invalid_message="Please enter a valid email address",
    ).execute()

    # Ask for password
    password = inquirer.secret(
        message="Password:",
        validate=lambda x: len(x) > 0,
        invalid_message="Password cannot be empty",
    ).execute()

    # Ask for format
    format_choices = [
        {"name": f"FLAC ({FORMAT_LABELS['flac']}) - Recommended", "value": "flac"},
        {"name": f"ALAC ({FORMAT_LABELS['alac']})", "value": "alac"},
        {"name": f"AAC ({FORMAT_LABELS['aac']})", "value": "aac"},
    ]

    audio_format = inquirer.select(
        message="Select audio format:",
        choices=format_choices,
        default="flac",
    ).execute()

    # Ask for output directory
    output_dir = inquirer.text(
        message="Output directory:",
        default="~/Music/LivePhish",
    ).execute()

    # Create config
    config = Config(
        email=email,
        format=audio_format,
        output_dir=output_dir,
        epoch_compensation=0,
    )

    # Save config and credentials
    save_config(config)
    save_credentials(email, password)

    print(f"\n✓ Configuration saved to {CONFIG_FILE}")
    print(f"✓ Password stored securely in system keyring")
    print(f"✓ Downloads will be saved to {Path(output_dir).expanduser()}\n")

    return config
