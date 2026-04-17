@echo off
title Nugs Downloader - Change Credentials
chcp 65001 >nul
cd /d "%~dp0"

if not exist "%~dp0nugs.exe" (
    echo ERROR: nugs.exe not found in this folder.
    pause
    exit /b 1
)

echo.
echo =============================================================
echo   Re-enter your nugs.net (and optional LivePhish) credentials.
echo =============================================================
echo.
nugs.exe config
echo.
pause
