#!/bin/bash
set -e

cd "$(dirname "$0")/.."

echo "Building LivePhish standalone binary..."

uv run pyinstaller --onefile --name livephish src/livephish/__main__.py \
    --hidden-import=keyring.backends.macOS \
    --hidden-import=keyring.backends.Windows \
    --hidden-import=InquirerPy.prompts.fuzzy \
    --hidden-import=InquirerPy.prompts.secret \
    --hidden-import=InquirerPy.prompts.confirm \
    --hidden-import=InquirerPy.prompts.text \
    --hidden-import=InquirerPy.prompts.list \
    --hidden-import=platformdirs \
    --collect-submodules=rich._unicode_data

# Copy launcher into dist/
cp scripts/LivePhish.command dist/
chmod +x dist/LivePhish.command dist/livephish

# Create zip
cd dist
zip -r LivePhish-macOS.zip livephish LivePhish.command
cd ..

echo ""
echo "Built: dist/LivePhish-macOS.zip"
echo "To distribute: unzip and double-click LivePhish.command"
