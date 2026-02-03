#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

require_cmd cargo
require_cmd git

repo_root="$(cd "$TEST_DIR/.." && pwd)"
workspace="$(make_temp_dir)"
trap 'rm -rf "$workspace"' EXIT

cat > "$workspace/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "DELAY-TEST",
      "title": "Delay validation test",
      "model": "gpt-5.1-codex-mini",
      "status": "unstarted",
      "definition_of_done": [
        "This task should not run because validation fails first."
      ],
      "recommended": {
        "approach": "Validation error should be returned before any task processing."
      }
    }
  ]
}
JSON

init_git_repo "$workspace"

output=""
if output="$(
  cd "$repo_root"
  cargo run --quiet -- \
    --workspace "$workspace" \
    --tasks "$workspace/prd.json" \
    --delay 1 \
    2>&1
)"; then
  echo "expected non-zero exit when --delay is used without --loop" >&2
  exit 1
fi

if ! grep -q -- "--delay requires --loop" <<<"$output"; then
  echo "expected error mentioning --delay requires --loop" >&2
  exit 1
fi
