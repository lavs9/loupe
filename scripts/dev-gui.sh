#!/bin/bash
# Run a local GUI build with its own Application Support folder.
# The dmg /Applications copy always uses ~/Library/Application Support/loupe.
#
#   ./scripts/dev-gui.sh
set -euo pipefail
cd "$(dirname "$0")/.."
export LOUPE_STATE_DIR="${LOUPE_STATE_DIR:-$HOME/Library/Application Support/loupe-dev}"
mkdir -p "$LOUPE_STATE_DIR"
echo "dev state $LOUPE_STATE_DIR"
exec cargo run --features gui -- gui
