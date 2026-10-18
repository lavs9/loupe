#!/bin/bash
# Build the GPUI window and open it.
#
# Ways this can fail, checked below:
# - the window feature does not compile
# - the process exits before the window exists
# - the global shortcut is not registered
# - the launch starts a second download of the model
set -euo pipefail
cd "$(dirname "$0")/.."

# Own target dir, so this check does not replace the window binary in target/debug.
export CARGO_TARGET_DIR="$PWD/target/window-check"
cargo build -q --features gui
BIN="$CARGO_TARGET_DIR/debug/loupe"

ROOT=$(mktemp -d /tmp/loupe-gui.XXXXXX)
export LOUPE_STATE_DIR="$ROOT/state"
export LOUPE_VOLUMES="$ROOT/volumes"
export LOUPE_MODEL_DIR="$PWD/target/gemma-models"
export LOUPE_RUNTIME_DIR="$PWD/target/gemma-runtime"
# A dead embedder URL. Opening the window must not download weights.
export LOUPE_GEMMA_URL="http://127.0.0.1:9"
mkdir -p "$ROOT/state" "$ROOT/volumes/EmptyDisk" "$LOUPE_MODEL_DIR" "$LOUPE_RUNTIME_DIR"

# Not the default shortcut, so this check does not take ⇧⌘Space.
cat > "$ROOT/state/gui.toml" <<'EOF'
shortcut=ctrl+alt+super+f19
check_updates=false
update_feed=
last_check_unix=0
EOF

before_pid=""
cleanup() {
  if [[ -n "${gui_pid:-}" ]]; then
    kill "$gui_pid" 2>/dev/null || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do
      kill -0 "$gui_pid" 2>/dev/null || break
      sleep 0.2
    done
    kill -9 "$gui_pid" 2>/dev/null || true
    wait "$gui_pid" 2>/dev/null || true
  fi
  if [[ -f "${pidfile:-}" ]]; then
    now_pid=$(cat "$pidfile" 2>/dev/null || true)
    if [[ -n "$now_pid" && "$now_pid" != "$before_pid" ]]; then
      kill "$now_pid" 2>/dev/null || true
    fi
  fi
  if [[ -f "$ROOT/gui.out" ]]; then
    mkdir -p target
    cp "$ROOT/gui.out" target/gui-report.txt
  fi
  rm -rf "$ROOT"
}
trap cleanup EXIT

"$BIN" gui --self-check >"$ROOT/self-check.txt"
grep -q $'folder\tdisk\tno-index\tEmptyDisk\t' "$ROOT/self-check.txt"

pidfile="$LOUPE_RUNTIME_DIR/llama-server.pid"
if [[ -f "$pidfile" ]]; then
  before_pid=$(cat "$pidfile")
fi
before=$(find "$LOUPE_MODEL_DIR" -type f -exec stat -f '%i %z' {} \; | sort)
"$BIN" gui >"$ROOT/gui.out" 2>"$ROOT/gui.err" &
gui_pid=$!

opened=""
for _ in $(seq 1 60); do
  if ! kill -0 "$gui_pid" 2>/dev/null; then
    echo "the window exited" >&2
    cat "$ROOT/gui.err" >&2
    exit 1
  fi
  if grep -q '^window open$' "$ROOT/gui.out" && grep -q '^shortcut registered ctrl+alt+super+f19$' "$ROOT/gui.out" && grep -q '^menu bar ready$' "$ROOT/gui.out" && grep -q '^idle$' "$ROOT/gui.out"; then
    opened=1
    break
  fi
  sleep 0.5
done
if [[ -z "$opened" ]]; then
  echo "the window did not park in the menu bar, or the shortcut was not registered" >&2
  cat "$ROOT/gui.out" >&2
  cat "$ROOT/gui.err" >&2
  exit 1
fi

sleep 1
kill -0 "$gui_pid" 2>/dev/null || {
  echo "the window closed itself" >&2
  exit 1
}
after=$(find "$LOUPE_MODEL_DIR" -type f -exec stat -f '%i %z' {} \; | sort)
test "$before" = "$after" || {
  echo "opening the window changed the model files" >&2
  exit 1
}
if [[ -f "$pidfile" ]]; then
  now_pid=$(cat "$pidfile")
  if [[ "$now_pid" != "$before_pid" ]] && kill -0 "$now_pid" 2>/dev/null; then
    echo "opening the window started the embedder sidecar" >&2
    exit 1
  fi
fi

echo "PASS" | tee -a "$ROOT/gui.out"
echo "artifact target/gui-report.txt"
