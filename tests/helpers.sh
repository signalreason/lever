#!/usr/bin/env bash
set -euo pipefail

make_temp_dir() {
  mktemp -d 2>/dev/null || mktemp -d -t lever
}

require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Missing required command: $cmd" >&2
    exit 1
  fi
}
