#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$TEST_DIR/.." && pwd)"

status=0
echo "Running validate_prd"
if ! cargo run --quiet --manifest-path "$REPO_ROOT/Cargo.toml" --bin validate_prd -- \
  --tasks "$REPO_ROOT/prd.json" \
  --schema "$REPO_ROOT/prd.schema.json"; then
  status=1
fi

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
