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
nugs.exe
if %ERRORLEVEL% neq 0 (
    echo.
    echo Something went wrong. Please take a screenshot of this window and report the issue.
    pause
)
