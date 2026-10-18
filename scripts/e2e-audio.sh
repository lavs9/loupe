#!/bin/bash
# End-to-end check: a song longer than LiteRT's 1024-token cap still indexes.
#
# Ways this can fail, checked below:
# - python cannot write a 120 second wav (skip is a fail)
# - ffmpeg/afconvert cannot decode it (skip is a fail)
# - LiteRT rejects the wav (sequence length > 1024)
# - index reports embedded 0
# - catalog has no audio/wav row
# - search for the file name returns nothing
set -euo pipefail
cd "$(dirname "$0")/.."

ROOT=$(mktemp -d /tmp/loupe-e2e-audio.XXXXXX)
export LOUPE_STATE_DIR="$ROOT/state"
export LOUPE_VOLUMES="$ROOT/volumes"
export LOUPE_MODEL_DIR="${LOUPE_MODEL_DIR:-$PWD/target/gemma-models}"
export LOUPE_RUNTIME_DIR="${LOUPE_RUNTIME_DIR:-$PWD/target/gemma-runtime}"
mkdir -p "$ROOT/songs" "$ROOT/state" "$LOUPE_MODEL_DIR" "$LOUPE_RUNTIME_DIR"

cleanup() {
  if [[ -f "$ROOT/report.txt" ]]; then
    mkdir -p target
    cp "$ROOT/report.txt" target/e2e-audio-report.txt
  fi
  rm -rf "$ROOT"
}
trap cleanup EXIT

if [[ -z "${BIN:-}" ]]; then
  export CARGO_TARGET_DIR="$PWD/target/cli"
  cargo build -q
  BIN="$CARGO_TARGET_DIR/debug/loupe"
fi

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

REPORT="$ROOT/report.txt"
{
  echo "=== self-check ==="
  "$BIN" gui --self-check
  echo "=== index ==="
  "$BIN" index "$ROOT/songs"
  echo "=== catalog ==="
  sqlite3 "$ROOT/songs/.photo_index/db/catalog.db" "SELECT mime_type, COUNT(*) FROM media_assets GROUP BY 1;"
  echo "=== search ==="
  "$BIN" search piano-etude --source "$ROOT/songs"
} | tee "$REPORT"

fail() { echo "FAIL: $*" | tee -a "$REPORT"; exit 1; }

grep -q 'audio-cap 30s ' "$REPORT" || fail "120 second audio was not cut to 30 seconds"
grep -q 'embedded 1' "$REPORT" || fail "120 second song was not embedded"
grep -q $'audio/wav|1' "$REPORT" || grep -q 'audio/wav' "$REPORT" || fail "catalog has no audio/wav row"
grep -q 'piano-etude.wav' "$REPORT" || fail "search missed piano-etude.wav"
grep -q 'exceeds maximum' "$REPORT" && fail "LiteRT still rejected the wav as too long"

echo "PASS" | tee -a "$REPORT"
echo "artifact target/e2e-audio-report.txt"
