#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

PROJECT_PATH="${1:-$PWD}"

exec cargo watch --clear \
    --watch src --watch assets \
    --shell "PROFILE=debug SIGN=0 ./scripts/bundle-mac.sh && { \
               pkill -f 'Helix.app/Contents/MacOS/helix' || true; \
               open target/Helix.app --args '${PROJECT_PATH}'; \
             }"
