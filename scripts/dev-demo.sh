#!/usr/bin/env bash
# One-command demo: seeds a data dir, then opens the app on it, offline.
# Made for judging look & feel with real input — no edge, no auth, no daemon.
#
#   scripts/dev-demo.sh            # build, seed demo data, open the app
#   scripts/dev-demo.sh --slow     # pace mock streams (~10s) to watch streaming
#
# Everything lives under /tmp/helix-demo-data; re-runs reuse it. The seeder is
# a separate process because the app takes an exclusive lock on the data dir:
# seed first, exit, then launch.
set -euo pipefail
cd "$(dirname "$0")/.."

DATA_DIR=/tmp/helix-demo-data
DELAY=""
[[ "${1:-}" == "--slow" ]] && DELAY=350

echo "▸ building (first run takes a few minutes)…"
cargo build -p helix -q
cargo build -p helix-engine --example demo_seed -q

if [[ ! -f "$DATA_DIR/.demo-seeded" ]]; then
  echo "▸ seeding demo chats"
  env HELIX_DATA_DIR="$DATA_DIR" HELIX_HARNESS=mock RUST_LOG=warn \
    ./target/debug/examples/demo_seed "$DATA_DIR"
  touch "$DATA_DIR/.demo-seeded"
fi

echo "▸ opening helix (composer is live — type into it; --slow shows streaming)"
env HELIX_DATA_DIR="$DATA_DIR" HELIX_EDGE_URL= HELIX_HARNESS=mock RUST_LOG=warn \
  ${DELAY:+HELIX_MOCK_DELAY_MS=$DELAY} \
  ./target/debug/helix
