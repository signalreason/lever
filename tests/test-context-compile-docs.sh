#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

repo_root="$(cd "$TEST_DIR/.." && pwd)"
readme="$repo_root/README.md"
repo_map="$repo_root/REPO_MAP.md"

assert_contains() {
  local file="$1"
  local needle="$2"
  if ! grep -Fq "$needle" "$file"; then
    echo "Expected $file to mention: $needle" >&2
    exit 1
  fi
}

assert_contains "$readme" "context-compile.json"
assert_contains "$readme" "policy_outcome"
assert_contains "$readme" "best-effort continues without compiled context"
assert_contains "$readme" "required blocks the run"
assert_contains "$readme" "assembly-summary.json"

assert_contains "$repo_map" "context-compile.json"
assert_contains "$repo_map" "pack/context.md"
assert_contains "$repo_map" "pack/lint.json"
assert_contains "$repo_map" "policy_outcome"
