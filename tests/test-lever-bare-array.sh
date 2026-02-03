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
[
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
    "title": "Human decision task",
    "status": "unstarted",
    "model": "human",
    "definition_of_done": [
      "Confirm human ownership"
    ],
    "recommended": {
      "approach": "Wait for operator"
    }
  },
  {
    "task_id": "GAMMA",
    "title": "First runnable task",
    "status": "blocked",
    "model": "gpt-5.1-codex-mini",
    "definition_of_done": [
      "Run the first runnable task"
    ],
    "recommended": {
      "approach": "Keep this quick"
    }
  },
  {
    "task_id": "DELTA",
    "title": "Later runnable task",
    "status": "unstarted",
    "model": "gpt-5.1-codex-mini",
    "definition_of_done": [
      "Run later task if needed"
    ],
    "recommended": {
      "approach": "Only after GAMMA"
    }
  }
]
JSON

init_git_repo "$repo_dir"

output="$(
  cargo run --quiet --manifest-path "$TEST_DIR/../Cargo.toml" \
    -- --workspace "$repo_dir" --tasks "$repo_dir/prd.json" --command-path "$true_bin"
)"

if ! grep -q "selected task GAMMA" <<<"$output"; then
  echo "Expected lever to select the first runnable task (GAMMA) from a bare array" >&2
  exit 1
fi
