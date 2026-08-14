#!/usr/bin/env bash
# Watch + rebuild + relaunch loop. There is no hot reload: gpui is a native app,
# so every change rebuilds and restarts the process. Window position and any
# in-memory UI state are lost on each cycle; the data dir survives.
#
#   scripts/dev.sh                 # seeded demo data, mock harness, offline
#   scripts/dev.sh --claude        # real claude-code harness instead of mock
#   scripts/dev.sh --slow          # pace mock streams to watch streaming
#   scripts/dev.sh --data DIR      # use another data dir
#
# Needs cargo-watch (`cargo install cargo-watch`).
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=./target/debug/helix
DATA_DIR=/tmp/helix-dev-data
HARNESS=mock
DELAY=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --claude) HARNESS=claude-code; shift ;;
    --slow) DELAY=350; shift ;;
    --data) DATA_DIR="$2"; shift 2 ;;
    # Internal: the cargo-watch hook after a successful build.
    --restart) MODE=restart; shift ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done

# The app holds an exclusive lock on its data dir, so the old process must be
# gone — not merely signalled — before the new one starts.
stop_app() {
  pkill -x -f "$BIN" 2>/dev/null || true
  for _ in $(seq 1 40); do
    pgrep -x -f "$BIN" >/dev/null 2>&1 || return 0
    sleep 0.1
  done
  echo "warn: previous instance still running; the new one may fail to take the data-dir lock" >&2
}

launch_app() {
  mkdir -p "$DATA_DIR"
  env HELIX_DATA_DIR="$DATA_DIR" HELIX_EDGE_URL= HELIX_HARNESS="$HARNESS" \
    ${DELAY:+HELIX_MOCK_DELAY_MS=$DELAY} RUST_LOG="${RUST_LOG:-info}" \
    "$BIN" >>"$DATA_DIR/dev.out" 2>&1 &
  echo "▸ helix pid $! — logs: $DATA_DIR/dev.out"
}

if [[ "${MODE:-}" == "restart" ]]; then
  stop_app
  launch_app
  exit 0
fi

command -v cargo-watch >/dev/null 2>&1 || {
  echo "cargo-watch missing: cargo install cargo-watch" >&2
  exit 1
}

mkdir -p "$DATA_DIR"
echo "▸ building (first run takes a few minutes)…"
cargo build -p helix -q
cargo build -p helix-engine --example demo_seed -q

if [[ ! -f "$DATA_DIR/.demo-seeded" ]]; then
  echo "▸ seeding dev data"
  env HELIX_DATA_DIR="$DATA_DIR" HELIX_HARNESS=mock RUST_LOG=warn \
    ./target/debug/examples/demo_seed "$DATA_DIR"
  touch "$DATA_DIR/.demo-seeded"
fi

# Kill the app when the watcher itself dies, so Ctrl-C leaves nothing behind.
trap stop_app EXIT INT TERM

RESTART_ARGS=(--restart --data "$DATA_DIR")
[[ "$HARNESS" != mock ]] && RESTART_ARGS+=(--claude)
[[ -n "$DELAY" ]] && RESTART_ARGS+=(--slow)

echo "▸ watching crates/ apps/ assets/ — save to rebuild and restart"
exec cargo watch --clear \
  --watch crates --watch apps --watch assets --watch Cargo.toml \
  --exec "build -p helix" \
  --shell "scripts/dev.sh $(printf '%q ' "${RESTART_ARGS[@]}")"
