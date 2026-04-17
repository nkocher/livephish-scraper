@echo off
title Nugs Downloader
chcp 65001 >nul
cd /d "%~dp0"

if not exist "%~dp0nugs.exe" (
    echo ERROR: nugs.exe not found in this folder.
    echo Make sure ALL files from the ZIP were extracted to the same folder.
    pause
    exit /b 1
)

REM First-run setup: if no config file exists, launch the setup wizard
if not exist "%APPDATA%\nugs\config.toml" (
    echo.
    echo =============================================================
    echo   Welcome! First-time setup -- enter your nugs.net login.
    echo =============================================================
    echo.
    nugs.exe config
    if errorlevel 1 (
        echo.
        echo Setup was cancelled or did not complete. Please try again.
        pause
        exit /b 1
    )
    echo.
    echo Setup complete. Starting the app...
    echo.
)

nugs.exe
if errorlevel 1 (
    echo.
    echo Something went wrong. Please take a screenshot of this window
    echo and send it along so the issue can be fixed.
    pause
)
