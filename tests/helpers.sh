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

init_git_repo() {
  local dir="$1"
  (
    cd "$dir"
    git init -b main >/dev/null
    if [[ -z "$(ls -A | grep -v '^\.git$' || true)" ]]; then
      printf '%s\n' "Test repo" > README.md
    fi
    git add -A
    GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com \
      GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com \
      git commit -m "init" >/dev/null
  )
}
