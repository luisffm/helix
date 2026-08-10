#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

PROJECT_PATH="${1:-$PWD}"

exec cargo watch --clear \
    --watch app --watch ui --watch terminal --watch git --watch worktree \
    --watch filesystem --watch events --watch models --watch state \
    --watch agents --watch github --watch commands \
    --exec "run -- ${PROJECT_PATH}"
