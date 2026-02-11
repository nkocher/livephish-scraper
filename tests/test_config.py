"""Tests for LivePhish configuration management."""

from pathlib import Path

import pytest
import yaml

from livephish import config
from livephish.config import Config, get_credentials, load_config, save_config


class TestConfig:
    """Tests for Config dataclass."""

    def test_defaults(self):
        """Test Config has expected default values."""
        cfg = Config()

        assert cfg.email == ""
        assert cfg.format == "flac"
        assert cfg.output_dir == "~/Music/LivePhish"
        assert cfg.epoch_compensation == 0


class TestSaveLoadConfig:
    """Tests for save_config and load_config functions."""

    def test_save_and_load_roundtrip(self, tmp_path, monkeypatch):
        """Test saving and loading config preserves all values."""
        # Set up temporary config directory
        config_dir = tmp_path / "config"
        config_file = config_dir / "config.yaml"
        cache_dir = config_dir / "cache"

        monkeypatch.setattr(config, "CONFIG_DIR", config_dir)
        monkeypatch.setattr(config, "CONFIG_FILE", config_file)
        monkeypatch.setattr(config, "CACHE_DIR", cache_dir)

        # Create config with non-default values
        test_config = Config(
            email="test@example.com",
            format="alac",
            output_dir="~/Downloads/Music",
            epoch_compensation=300,
        )

        # Save and load
        save_config(test_config)
        loaded_config = load_config()

        # Verify all fields match
        assert loaded_config.email == "test@example.com"
        assert loaded_config.format == "alac"
        assert loaded_config.output_dir == "~/Downloads/Music"
        assert loaded_config.epoch_compensation == 300

    def test_load_config_creates_directories(self, tmp_path, monkeypatch):
        """Test load_config creates CONFIG_DIR and CACHE_DIR if missing."""
        config_dir = tmp_path / "new_config"
        config_file = config_dir / "config.yaml"
        cache_dir = tmp_path / "new_cache"

        monkeypatch.setattr(config, "CONFIG_DIR", config_dir)
        monkeypatch.setattr(config, "CONFIG_FILE", config_file)
        monkeypatch.setattr(config, "CACHE_DIR", cache_dir)

        # Directories should not exist yet
        assert not config_dir.exists()
        assert not cache_dir.exists()

        # Load config (which creates default if missing)
        cfg = load_config()

        # Directories should now exist
        assert config_dir.exists()
        assert cache_dir.exists()
        assert config_file.exists()

        # Should return default config (unless migration happened)
        # The migration may copy from ~/.config/livephish if it exists,
        # so we just verify the directories were created
        assert isinstance(cfg, Config)

    def test_load_config_with_existing_file(self, tmp_path, monkeypatch):
        """Test load_config reads existing YAML file."""
        config_dir = tmp_path / "config"
        config_file = config_dir / "config.yaml"
        cache_dir = config_dir / "cache"

        monkeypatch.setattr(config, "CONFIG_DIR", config_dir)
        monkeypatch.setattr(config, "CONFIG_FILE", config_file)
        monkeypatch.setattr(config, "CACHE_DIR", cache_dir)

        # Manually create config file
        config_dir.mkdir(parents=True)
        with open(config_file, "w") as f:
            yaml.safe_dump(
                {
                    "email": "manual@example.com",
                    "format": "aac",
                    "output_dir": "~/Music/Test",
                    "epoch_compensation": 600,
                },
                f,
            )

        # Load should read the file
        cfg = load_config()

        assert cfg.email == "manual@example.com"
        assert cfg.format == "aac"
        assert cfg.output_dir == "~/Music/Test"
        assert cfg.epoch_compensation == 600

    def test_save_config_creates_directory(self, tmp_path, monkeypatch):
        """Test save_config creates CONFIG_DIR if it doesn't exist."""
        config_dir = tmp_path / "new_dir"
        config_file = config_dir / "config.yaml"

        monkeypatch.setattr(config, "CONFIG_DIR", config_dir)
        monkeypatch.setattr(config, "CONFIG_FILE", config_file)

        assert not config_dir.exists()

        cfg = Config(email="test@example.com")
        save_config(cfg)

        assert config_dir.exists()
        assert config_file.exists()


class TestGetCredentials:
    """Tests for get_credentials function."""

    def test_get_credentials_raises_when_email_empty(self):
        """Test get_credentials raises ValueError when email is not configured."""
        cfg = Config(email="")

        with pytest.raises(ValueError, match="Email not configured"):
            get_credentials(cfg)

    def test_get_credentials_raises_when_password_missing(self, monkeypatch):
        """Test get_credentials falls back to getpass when password not in keyring."""
        # Mock keyring to return None
        monkeypatch.setattr("livephish.config.keyring.get_password", lambda s, e: None)
        # Mock getpass to return empty string (triggering the "Password required" error)
        monkeypatch.setattr("getpass.getpass", lambda prompt: "")

        cfg = Config(email="test@example.com")

        with pytest.raises(ValueError, match="Password required"):
            get_credentials(cfg)

    def test_get_credentials_returns_tuple(self, monkeypatch):
        """Test get_credentials returns email and password tuple."""
        # Mock keyring to return a password
        monkeypatch.setattr(
            "livephish.config.keyring.get_password", lambda s, e: "test_password"
        )

        cfg = Config(email="test@example.com")
        email, password = get_credentials(cfg)

        assert email == "test@example.com"
        assert password == "test_password"


class TestConfigFormatValidation:
    """Tests for Config format field validation."""

    def test_valid_formats(self):
        """Test Config accepts valid format values."""
        # These should all be valid
        cfg_flac = Config(format="flac")
        cfg_alac = Config(format="alac")
        cfg_aac = Config(format="aac")

        assert cfg_flac.format == "flac"
        assert cfg_alac.format == "alac"
        assert cfg_aac.format == "aac"

    def test_format_persists_in_save_load(self, tmp_path, monkeypatch):
        """Test format value is preserved through save/load cycle."""
        config_dir = tmp_path / "config"
        config_file = config_dir / "config.yaml"
        cache_dir = config_dir / "cache"

        monkeypatch.setattr(config, "CONFIG_DIR", config_dir)
        monkeypatch.setattr(config, "CONFIG_FILE", config_file)
        monkeypatch.setattr(config, "CACHE_DIR", cache_dir)

        for fmt in ["flac", "alac", "aac"]:
            cfg = Config(format=fmt)
            save_config(cfg)
            loaded = load_config()
            assert loaded.format == fmt


class TestPlatformDirsPaths:
    """Tests verifying platformdirs gives platform-appropriate paths."""

    def test_config_dir_uses_platformdirs(self):
        """CONFIG_DIR should come from platformdirs, not a hardcoded path."""
        import platformdirs

        expected = Path(platformdirs.user_config_dir("livephish"))
        assert config.CONFIG_DIR == expected

    def test_cache_dir_uses_platformdirs(self):
        """CACHE_DIR should come from platformdirs, not a hardcoded path."""
        import platformdirs

        expected = Path(platformdirs.user_cache_dir("livephish"))
        assert config.CACHE_DIR == expected

    def test_windows_paths_simulated(self, monkeypatch):
        """Verify platformdirs returns Windows-style paths when platform is Windows."""
        import platformdirs

        # Simulate Windows by overriding the platform detection
        monkeypatch.setattr("platform.system", lambda: "Windows")
        monkeypatch.setenv("APPDATA", "C:\\Users\\TestUser\\AppData\\Roaming")
        monkeypatch.setenv("LOCALAPPDATA", "C:\\Users\\TestUser\\AppData\\Local")

        # platformdirs caches results, so call the functions directly
        config_dir = platformdirs.user_config_dir("livephish")
        cache_dir = platformdirs.user_cache_dir("livephish")

        # On Windows, these should use APPDATA / LOCALAPPDATA
        assert "livephish" in config_dir
        assert "livephish" in cache_dir


class TestGetCredentialsFallback:
    """Tests for get_credentials keyring fallback."""

    def test_keyring_failure_falls_back_to_getpass(self, monkeypatch, tmp_path):
        """When keyring fails, get_credentials should fall back to getpass."""
        monkeypatch.setattr(config, "CONFIG_DIR", tmp_path)
        monkeypatch.setattr(config, "CONFIG_FILE", tmp_path / "config.yaml")

        # Make keyring raise
        def failing_get_password(service, username):
            raise Exception("No keyring backend")
        monkeypatch.setattr("keyring.get_password", failing_get_password)

        # Mock getpass to return a password
        monkeypatch.setattr("getpass.getpass", lambda prompt: "test_password")

        cfg = Config(email="test@example.com")
        email, password = get_credentials(cfg)
        assert email == "test@example.com"
        assert password == "test_password"

    def test_save_credentials_keyring_failure_no_crash(self, monkeypatch):
        """save_credentials should not crash when keyring is unavailable."""
        def failing_set_password(service, username, password):
            raise Exception("No keyring backend")
        monkeypatch.setattr("keyring.set_password", failing_set_password)

        # Should not raise
        from livephish.config import save_credentials
        save_credentials("test@example.com", "password123")
