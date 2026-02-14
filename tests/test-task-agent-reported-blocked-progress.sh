#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

require_cmd jq
require_cmd git
require_cmd cargo

repo_root="$(cd "$TEST_DIR/.." && pwd)"
repo_dir="$(make_temp_dir)"
stub_bin="$(make_temp_dir)"
trap 'rm -rf "$repo_dir" "$stub_bin"' EXIT

cat > "$repo_dir/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "T1",
      "title": "Reported blocked should not hard block",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Runner should keep task started when model reports blocked"
      ],
      "recommended": {
        "approach": "Treat model-reported blocked as advisory only"
      }
    }
  ]
}
JSON

ensure_workspace_prompt "$repo_dir"

cat > "$repo_dir/README.md" <<'EOF2'
Test repo
EOF2

cat > "$stub_bin/codex" <<'EOF2'
#!/usr/bin/env bash
set -euo pipefail
out_path=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output-last-message)
      out_path="$2"
      shift 2
      ;;
    *)
      shift 1
      ;;
  esac
done

if [[ -z "$out_path" ]]; then
  echo "Missing --output-last-message" >&2
  exit 2
fi

cat > "$out_path" <<'JSON'
{
  "task_id": "T1",
  "outcome": "blocked",
  "dod_met": false,
  "summary": "blocked by model",
  "tests": {"ran": true, "commands": ["pnpm test missing"], "passed": false},
  "notes": "model marked blocked",
  "blockers": ["model blocker"]
}
JSON
EOF2
chmod +x "$stub_bin/codex"

init_git_repo "$repo_dir"

(
  cd "$repo_root"
  cargo build --quiet
)
lever_bin="$repo_root/target/debug/lever"

set +e
PATH="$stub_bin:$PATH" \
ASSIGNEE="test-assignee" \
  GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com \
  GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com \
  "$lever_bin" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --task-id T1 \
  >/dev/null 2>&1
exit_code=$?
set -e

if [[ "$exit_code" -ne 12 ]]; then
  echo "Expected lever to exit 12 for progress state, got: $exit_code" >&2
  exit 1
fi

status="$(jq -r '.tasks[0].status' "$repo_dir/prd.json")"
if [[ "$status" != "started" ]]; then
  echo "Expected task status to stay started, got: $status" >&2
  exit 1
fi

attempts="$(jq -r '.tasks[0].observability.run_attempts' "$repo_dir/prd.json")"
if [[ "$attempts" != "1" ]]; then
  echo "Expected run_attempts to be 1, got: $attempts" >&2
  exit 1
fi

note="$(jq -r '.tasks[0].observability.last_note // ""' "$repo_dir/prd.json")"
if [[ "$note" != *"reported_outcome=blocked"* ]]; then
  echo "Expected last_note to include reported_outcome=blocked, got: $note" >&2
  exit 1
fi
