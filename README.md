# Nugs Downloader

Browse and download shows from **nugs.net** and **LivePhish** from your terminal.

---

## Windows — no setup required

1. Go to [**Releases**](../../releases) and download `Nugs-Windows.zip`
2. Right-click the ZIP and choose **Extract All** — extract to a folder (e.g. your Desktop)
3. Double-click **START.bat**

> **Windows safety warnings**
>
> - *"Windows protected your PC"* — this is Windows SmartScreen. Click **More info**, then **Run anyway**.
>   This appears on all apps not submitted to Microsoft; it is not a virus warning.
> - *Windows Defender may delete or quarantine `ffmpeg.exe`* — this is a common false positive for
>   command-line audio tools. To fix it: open **Windows Security → Virus & threat protection →
>   Protection history**, find the blocked item, and click **Allow**.

4. **First run:** the app opens to a menu. Type `config` and press Enter to enter your nugs.net
   (and optionally LivePhish) credentials.

### What's in the ZIP

| File | What it does |
|---|---|
| `nugs.exe` | The downloader |
| `ffmpeg.exe` / `ffprobe.exe` | Audio conversion (FLAC→AAC/ALAC) — bundled, nothing to install |
| `START.bat` | The launcher — always use this to open the app |
| `LICENSE-ffmpeg.txt` | FFmpeg open-source license (LGPL 2.1) |

---

## macOS

```
tar -xzf Nugs-macOS.tar.gz
./nugs
```

On first run macOS may say the app is from an unidentified developer. Go to **System Settings →
Privacy & Security** and click **Open Anyway**.

---

## Linux

```
tar -xzf Nugs-Linux.tar.gz
./nugs
```

Fully static binary — works on any x86_64 Linux distribution.

---

## Features

- Interactive browser: browse by year, artist, or search
- Download individual shows or bulk-download entire artist catalogs
- nugs.net and LivePhish supported (LivePhish credentials optional)
- Format selection: FLAC, ALAC, AAC, MQA, 360 Reality Audio
- FLAC → AAC or ALAC conversion via bundled ffmpeg
- Download queue for batching multiple shows
- Resume-safe: skips already-downloaded tracks

---

## Configuration

Run the app and select `config` to set:
- nugs.net email and password
- LivePhish email and password (optional — enables higher-quality Phish streams)
- Download folder (default: `~/Music/Nugs`)
- Default format

---

## Build from source

Requires [Rust](https://rustup.rs) stable.

```
cargo build --release
./target/release/nugs
```

Run tests:
```
cargo test
```
