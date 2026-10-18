#!/bin/bash
# Wipe the packaged-app profile so the next launch is a first-time user.
# Photos stay. Dev state (`loupe-dev`) stays.
#
#   ./scripts/reset-user.sh
set -euo pipefail

STATE="$HOME/Library/Application Support/loupe"
LAUNCH="$HOME/Library/LaunchAgents/app.loupe.gui.plist"

echo "stopping Loupe and the embedder for the user profile"
if [[ -f "$LAUNCH" ]]; then
  launchctl unload "$LAUNCH" 2>/dev/null || true
  rm -f "$LAUNCH"
fi
pkill -f '/Applications/Loupe.app/Contents/MacOS/loupe' 2>/dev/null || true
if [[ -f "$STATE/runtime/llama-server.pid" ]]; then
  kill "$(cat "$STATE/runtime/llama-server.pid")" 2>/dev/null || true
fi
pkill -f 'Application Support/loupe/runtime/.*/llama-server' 2>/dev/null || true
pkill -f 'embeddinggemma-2-Q8_0.gguf' 2>/dev/null || true
sleep 0.4
pkill -9 -f 'Application Support/loupe/runtime/.*/llama-server' 2>/dev/null || true

if [[ -f "$STATE/sources.txt" ]]; then
  while IFS= read -r line; do
    [[ "$line" =~ ^# ]] && continue
    [[ -z "$line" ]] && continue
    catalog="$line/.photo_index"
    if [[ -d "$catalog" ]]; then
      echo "removing catalog $catalog"
      rm -rf "$catalog"
    fi
  done < "$STATE/sources.txt"
fi

# Known folders that may have been indexed then dropped from sources.txt
for catalog in \
  "$HOME/Downloads/.photo_index" \
  "$HOME/Pictures/.photo_index"
do
  if [[ -d "$catalog" ]]; then
    echo "removing catalog $catalog"
    rm -rf "$catalog"
  fi
done

if [[ -d "$STATE" ]]; then
  echo "removing $STATE"
  rm -rf "$STATE"
fi

echo "user profile is empty. Download the dmg, drag to Applications, right-click Open."
