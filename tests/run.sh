#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

status=0
for test in "$TEST_DIR"/test-*.sh; do
  if [[ ! -f "$test" ]]; then
    echo "No tests found" >&2
    exit 1
  fi
  echo "Running $(basename "$test")"
  if ! "$test"; then
    status=1
  fi
done

exit "$status"
