#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

require_cmd cargo
require_cmd git

repo_dir="$(make_temp_dir)"
trap 'rm -rf "$repo_dir"' EXIT
true_bin="$(command -v true)"

cat > "$repo_dir/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "ALPHA",
      "title": "Completed placeholder task",
      "status": "completed",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Capture completion metadata"
      ],
      "recommended": {
        "approach": "Nurse this stub"
      }
    },
    {
      "task_id": "BETA",
      "title": "Next runnable task",
      "status": "blocked",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Run the next available task"
      ],
      "recommended": {
        "approach": "Keep this quick"
      }
    }
  ]
}
JSON

init_git_repo "$repo_dir"

output="$(
  cargo run --quiet --manifest-path "$TEST_DIR/../Cargo.toml" \
    -- --tasks "$repo_dir/prd.json" --command-path "$true_bin"
)"

if ! grep -q "selected task BETA" <<<"$output"; then
  echo "Expected lever to select the first non-completed task" >&2
  exit 1
fi

output="$(
  cargo run --quiet --manifest-path "$TEST_DIR/../Cargo.toml" \
    -- --tasks "$repo_dir/prd.json" --task-id ALPHA --command-path "$true_bin"
)"

if ! grep -q "selected task ALPHA" <<<"$output"; then
  echo "Expected lever to honor explicit --task-id" >&2
  exit 1
fi

set +e
missing_output="$(
  cargo run --quiet --manifest-path "$TEST_DIR/../Cargo.toml" \
    -- --tasks "$repo_dir/prd.json" --task-id MISSING --command-path "$true_bin" 2>&1
)"
missing_exit=$?
set -e

if [[ $missing_exit -eq 0 ]]; then
  echo "Expected lever to fail when the requested task does not exist" >&2
  exit 1
fi

if ! grep -q "Task ID 'MISSING' was not found" <<<"$missing_output"; then
  echo "Expected lever to report the missing task ID" >&2
  exit 1
fi
