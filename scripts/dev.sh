#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

PROJECT_PATH="${1:-$PWD}"

exec cargo watch --clear \
    --watch src \
    --exec "run -- ${PROJECT_PATH}"
