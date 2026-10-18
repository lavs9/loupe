#!/bin/bash
# End-to-end check for EmbeddingGemma 2 and the dual-disk search.
#
# Ways this can fail, checked below:
# - a bridge photo loses to a kitchen photo on the query "golden gate bridge"
# - a kitchen photo loses to the bridge on a kitchen query
# - an external disk's index is ignored, or is still searched after unplug
# - a move re-embeds, or the old path is left behind
# - a deleted file stays in the results
# - a RAW container is indexed from its embedded JPEG
# - a HEIC of the bridge photo is missing from that search
# - an AVIF of the kitchen photo is missing from that search
# - search creates an index on a disk that does not have one
# - a mounted disk is reported as not connected
# - a mounted disk with an index is missing from the folder list
# - a mounted disk without an index is treated as searchable
# - an unplugged remembered disk is not marked not connected
# - a song longer than 41 seconds is skipped (LiteRT 1024-token cap)
# - search for that song's name returns nothing
set -euo pipefail
cd "$(dirname "$0")/.."

ROOT=$(mktemp -d /tmp/loupe-e2e.XXXXXX)
export LOUPE_STATE_DIR="$ROOT/state"
export LOUPE_VOLUMES="$ROOT/volumes"
export LOUPE_MODEL_DIR="$PWD/target/gemma-models"
export LOUPE_RUNTIME_DIR="$PWD/target/gemma-runtime"
mkdir -p "$ROOT/local/Photos" "$ROOT/volumes/BackupHDD/Photos" "$ROOT/volumes/EmptyDisk" "$ROOT/state" "$ROOT/empty" \
  "$LOUPE_MODEL_DIR" "$LOUPE_RUNTIME_DIR"

cleanup() {
  if [[ -f "$LOUPE_RUNTIME_DIR/llama-server.pid" ]]; then
    kill "$(cat "$LOUPE_RUNTIME_DIR/llama-server.pid")" 2>/dev/null || true
  fi
  if [[ -f "$ROOT/report.txt" ]]; then
    mkdir -p target
    cp "$ROOT/report.txt" target/e2e-report.txt
  fi
  rm -rf "$ROOT"
}
trap cleanup EXIT

# Own target dir. A CLI build and a window build share target/debug/loupe,
# and Cargo will skip a later window rebuild after this one replaces that file.
export CARGO_TARGET_DIR="$PWD/target/cli"
cargo build -q
BIN="$CARGO_TARGET_DIR/debug/loupe"

# Real photographs, named so the words in the query are not in the path.
# Ranking then has to come from the picture.
curl -fsSL -o "$ROOT/local/Photos/frame-a.png" \
  "https://raw.githubusercontent.com/google-gemma/cookbook/refs/heads/main/apps/sample-data/GoldenGate.png"
curl -fsSL -o "$ROOT/volumes/BackupHDD/Photos/frame-b.jpg" \
  "https://raw.githubusercontent.com/google-gemma/cookbook/refs/heads/main/apps/sample-data/kitchen_painting.jpg"
{
  printf 'RAWHEADER'
  cat "$ROOT/volumes/BackupHDD/Photos/frame-b.jpg"
  printf 'RAWTAIL'
} > "$ROOT/local/Photos/frame-c.nef"

# The same pictures, in the formats an iPhone and the web use.
sips -s format heic "$ROOT/local/Photos/frame-a.png" --out "$ROOT/local/Photos/frame-a.heic" >/dev/null \
  || { echo "FAIL: could not write a HEIC" >&2; exit 1; }
sips -s format avif "$ROOT/volumes/BackupHDD/Photos/frame-b.jpg" --out "$ROOT/volumes/BackupHDD/Photos/frame-b.avif" >/dev/null \
  || { echo "FAIL: could not write an AVIF" >&2; exit 1; }
test -s "$ROOT/local/Photos/frame-a.heic"
test -s "$ROOT/volumes/BackupHDD/Photos/frame-b.avif"

# 120 s of silence named so FTS can find it. LiteRT used to reject anything
# over ~41 s (1024 tokens). Missing python is a fail, not a skip.
mkdir -p "$ROOT/songs"
python3 - "$ROOT/songs/piano-etude.wav" <<'PY' || { echo "FAIL: could not write a 120 second wav" >&2; exit 1; }
import sys, wave
path = sys.argv[1]
n = 44100 * 120
with wave.open(path, "w") as wav:
    wav.setnchannels(1)
    wav.setsampwidth(2)
    wav.setframerate(44100)
    wav.writeframes(b"\x00\x00" * n)
PY
test -s "$ROOT/songs/piano-etude.wav" || { echo "FAIL: 120 second wav is empty" >&2; exit 1; }

"$BIN" index "$ROOT/local"
"$BIN" index "$ROOT/volumes/BackupHDD"
song_out=$("$BIN" index "$ROOT/songs")
echo "$song_out" | grep -q 'embedded 1' || { echo "FAIL: 120 second song was not embedded: $song_out" >&2; exit 1; }
echo "$song_out" | grep -q 'embedded 0' && { echo "FAIL: 120 second song was skipped: $song_out" >&2; exit 1; }

REPORT="$ROOT/report.txt"
{
  echo "=== bridge ==="
  "$BIN" search golden gate bridge san francisco
  echo "=== kitchen ==="
  "$BIN" search painting of a kitchen interior
  echo "=== piano ==="
  "$BIN" search piano-etude --source "$ROOT/songs"
  echo "=== status ==="
  "$BIN" status 2>"$ROOT/status.err"
  echo "=== status stderr ==="
  cat "$ROOT/status.err"
} | tee "$REPORT"

first_path() {
  awk -v section="$1" '
    $0 ~ "^=== " section " ===" { grab = 1; next }
    grab && $0 ~ "^===" { exit }
    grab && $0 ~ "^      /" { print; exit }
  ' "$REPORT"
}

fail() { echo "FAIL: $*" | tee -a "$REPORT"; exit 1; }

grep -q 'no such table' "$REPORT" && fail "search printed a sqlite table error"

for db in \
  "$ROOT/local/.photo_index/db/catalog.db" \
  "$ROOT/volumes/BackupHDD/.photo_index/db/catalog.db"
do
  sqlite3 "file:${db}?immutable=1" "SELECT name FROM sqlite_master WHERE name='vec_media';" \
    | grep -q vec_media \
    || fail "catalog schema was not checkpointed for search: $db"
done

bridge=$(first_path bridge)
echo "$bridge" | grep -E -q 'frame-a.png|frame-a.heic' || fail "bridge ranked [$bridge], wanted the Golden Gate photo"
kitchen=$(first_path kitchen)
echo "$kitchen" | grep -E -q 'frame-b.jpg|frame-b.avif|frame-c.nef' || fail "kitchen ranked [$kitchen], wanted the kitchen photo"
awk '
  $0 ~ /^=== kitchen ===/ { grab = 1; next }
  grab && $0 ~ /^===/ { exit }
  grab && $0 ~ /^      \// { print }
' "$REPORT" > "$ROOT/kitchen-paths.txt"
grep -q 'frame-b.jpg' "$ROOT/kitchen-paths.txt" || fail "external kitchen photo missing"
grep -q 'frame-c.nef' "$ROOT/kitchen-paths.txt" || fail "RAW preview of the kitchen was not searchable"
awk '
  $0 ~ /^=== bridge ===/ { grab = 1; next }
  grab && $0 ~ /^===/ { exit }
  grab && $0 ~ /^      \// { print }
' "$REPORT" > "$ROOT/bridge-paths.txt"
grep -q 'frame-a.heic' "$ROOT/bridge-paths.txt" || fail "HEIC of the bridge was not searchable"
grep -q 'frame-a.png' "$ROOT/bridge-paths.txt" || fail "PNG of the bridge was not searchable"
grep -q 'frame-b.avif' "$ROOT/kitchen-paths.txt" || fail "AVIF of the kitchen was not searchable"
piano=$(first_path piano)
echo "$piano" | grep -q 'piano-etude.wav' || fail "120 second song ranked [$piano], wanted piano-etude.wav"
sqlite3 "file:$ROOT/songs/.photo_index/db/catalog.db?immutable=1" \
  "SELECT mime_type FROM media_assets;" | grep -q 'audio/wav' \
  || fail "120 second song was not stored as audio"
# The bridge photo is a different picture, so it ranks after both kitchen frames.
kitchen_bridge_line=$(grep -n 'frame-a.png' "$ROOT/kitchen-paths.txt" | head -1 | cut -d: -f1)
kitchen_photo_line=$(grep -n 'frame-b.jpg' "$ROOT/kitchen-paths.txt" | head -1 | cut -d: -f1)
test -n "$kitchen_bridge_line" || fail "bridge photo missing from kitchen results"
test "$kitchen_photo_line" -lt "$kitchen_bridge_line" || fail "kitchen photo did not beat the bridge"
kitchen_raw_line=$(grep -n 'frame-c.nef' "$ROOT/kitchen-paths.txt" | head -1 | cut -d: -f1)
test "$kitchen_raw_line" -lt "$kitchen_bridge_line" || fail "RAW kitchen preview did not beat the bridge"
kitchen_avif_line=$(grep -n 'frame-b.avif' "$ROOT/kitchen-paths.txt" | head -1 | cut -d: -f1)
test -n "$kitchen_avif_line" || fail "AVIF kitchen photo missing from ranked results"
test "$kitchen_avif_line" -lt "$kitchen_bridge_line" || fail "AVIF kitchen photo did not beat the bridge"
grep -q 'not connected' "$REPORT" && fail "mounted BackupHDD was reported not connected"

{
  echo "=== folders ==="
  "$BIN" gui --self-check
} | tee -a "$REPORT"
grep -q 'images v1' "$REPORT" || fail "image scope was not reported"
grep -q 'shortcut shift+super+space' "$REPORT" || fail "default shortcut changed"
grep -q 'display ⇧⌘Space' "$REPORT" || fail "shortcut display changed"
grep -q 'reject-bare rejected' "$REPORT" || fail "a bare key was accepted as a shortcut"
grep -q 'order 0.1.0 < 0.2.0' "$REPORT" || fail "version order broke"
grep -q 'order 0.2.0 = 0.2.0' "$REPORT" || fail "equal versions did not match"
grep -q 'order 0.2.0 > 0.1.0' "$REPORT" || fail "newer local version compared wrong"
grep -q 'order 0.1 < 0.1.1' "$REPORT" || fail "a longer version compared wrong"
grep -q 'feed-empty no-feed' "$REPORT" || fail "an empty feed was not offline"
grep -q 'feed-newer available 99.0.0 https://example.test/loupe' "$REPORT" || fail "a newer feed was not offered"
grep -q 'feed-same current' "$REPORT" || fail "the same version was not current"
grep -q 'feed-older current' "$REPORT" || fail "an older feed looked newer"
grep -q 'feed-bad failed' "$REPORT" || fail "a bad feed was accepted"
grep -q 'audio-cap 30s ' "$REPORT" || fail "120 second audio was not cut to 30 seconds"
grep -q $'folder\tdisk\tready\tBackupHDD\t' "$REPORT" || fail "mounted BackupHDD was not a ready disk"
grep -q $'folder\tdisk\tno-index\tEmptyDisk\t' "$REPORT" || fail "a disk without an index was hidden"
grep -q $'folder\tmac\tready\tThis Mac · local\t' "$REPORT" || fail "the local folder was not ready"
grep -q $'folder\tdisk\tnot-connected\t' "$REPORT" && fail "a disk was not connected while it was mounted"

mv "$ROOT/volumes/BackupHDD" "$ROOT/unplugged"
unplugged_rows=$("$BIN" gui --self-check)
echo "$unplugged_rows" | grep -q $'folder\tdisk\tnot-connected\tBackupHDD\t' || fail "unplugged disk was not marked not connected"
echo "$unplugged_rows" | grep -q $'folder\tdisk\tready\tBackupHDD\t' && fail "unplugged disk was still ready"
unplugged=$("$BIN" search painting of a kitchen interior)
echo "$unplugged" | grep -q 'BackupHDD' && fail "unplugged disk still returned hits"
echo "$unplugged" | grep -q 'frame-c.nef' || fail "local RAW preview disappeared while the disk was unplugged"
mv "$ROOT/unplugged" "$ROOT/volumes/BackupHDD"

mv "$ROOT/local/Photos/frame-a.png" "$ROOT/local/Photos/frame-a-moved.png"
move_out=$("$BIN" index "$ROOT/local")
echo "$move_out" | grep -q 'moved 1' || fail "move did not relink: $move_out"
echo "$move_out" | grep -q 'embedded 0' || fail "move re-embedded: $move_out"
moved=$("$BIN" search golden gate bridge san francisco)
echo "$moved" | grep -q 'frame-a-moved.png' || fail "moved file missing from search"
echo "$moved" | grep -q 'frame-a.png' && fail "old path survived the move"

rm "$ROOT/local/Photos/frame-c.nef"
"$BIN" index "$ROOT/local" >/dev/null
gone=$("$BIN" search painting of a kitchen interior)
echo "$gone" | grep -q 'frame-c.nef' && fail "deleted file still searchable"
echo "$gone" | grep -q 'frame-b.jpg' || fail "external kitchen photo disappeared"

set +e
"$BIN" search --source "$ROOT/empty" sunset >/dev/null 2>&1
code=$?
set -e
test "$code" -ne 0 || fail "search of a disk with no index succeeded"
test ! -e "$ROOT/empty/.photo_index" || fail "search created an index"

echo "PASS" | tee -a "$REPORT"
echo "artifact target/e2e-report.txt"
