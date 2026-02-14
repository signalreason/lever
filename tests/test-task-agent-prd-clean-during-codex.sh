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
      "title": "Keep prd clean during codex run",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Codex runs with clean tracked task state"
      ],
      "recommended": {
        "approach": "Do not mutate prd.json before codex executes"
      },
      "observability": {
        "run_attempts": 0,
        "last_note": "",
        "last_run_id": "seed",
        "last_update_utc": "2026-02-01T00:00:00Z"
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

if ! git diff --quiet -- prd.json; then
  echo "prd.json changed before codex completed" >&2
  exit 88
fi

cat > "$out_path" <<'JSON'
{
  "task_id": "T1",
  "outcome": "completed",
  "dod_met": true,
  "summary": "ok",
  "tests": {"ran": false, "commands": [], "passed": true},
  "notes": "",
  "blockers": []
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

PATH="$stub_bin:$PATH" \
ASSIGNEE="test-assignee" \
  GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com \
  GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com \
  "$lever_bin" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --task-id T1 \
  >/dev/null

status="$(jq -r '.tasks[0].status' "$repo_dir/prd.json")"
if [[ "$status" != "completed" ]]; then
  echo "Expected task status to be completed, got: $status" >&2
  exit 1
fi
