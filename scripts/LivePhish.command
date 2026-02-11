#!/bin/bash
# Double-click this file to launch LivePhish in Terminal.
# If macOS blocks it: right-click -> Open, then allow in System Settings -> Privacy & Security.
cd "$(dirname "$0")"
./livephish
echo ""
echo "Press any key to close this window..."
read -n 1
