#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

require_cmd jq
require_cmd git
require_cmd cargo
require_cmd python

repo_root="$(cd "$TEST_DIR/.." && pwd)"
repo_dir="$(make_temp_dir)"
stub_bin="$(make_temp_dir)"
trap 'rm -rf "$repo_dir" "$stub_bin"' EXIT

cat > "$repo_dir/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "T1",
      "title": "Assembly input materialization",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Write assembly task input",
        "Persist it in run artifacts"
      ],
      "recommended": {
        "approach": "Keep the context tight"
      },
      "verification": {
        "commands": ["echo verify"]
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

run_dir="$(find "$repo_dir/.ralph/runs/T1" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
if [[ -z "$run_dir" ]]; then
  echo "Expected run directory to exist" >&2
  exit 1
fi

assembly_task="$run_dir/assembly-task.json"
if [[ ! -f "$assembly_task" ]]; then
  echo "Expected assembly task input at $assembly_task" >&2
  exit 1
fi

if [[ "$(jq -r '.task_id' "$assembly_task")" != "T1" ]]; then
  echo "Expected task_id to be T1" >&2
  exit 1
fi

if [[ "$(jq -r '.title' "$assembly_task")" != "Assembly input materialization" ]]; then
  echo "Expected title to match" >&2
  exit 1
fi

if [[ "$(jq -r '.status' "$assembly_task")" != "unstarted" ]]; then
  echo "Expected status to match" >&2
  exit 1
fi

if [[ "$(jq -r '.model' "$assembly_task")" != "gpt-5.1-codex-mini" ]]; then
  echo "Expected model to match" >&2
  exit 1
fi

if [[ "$(jq -r '.definition_of_done | length' "$assembly_task")" != "2" ]]; then
  echo "Expected two definition_of_done entries" >&2
  exit 1
fi

if [[ "$(jq -r '.definition_of_done[0]' "$assembly_task")" != "Write assembly task input" ]]; then
  echo "Expected first definition_of_done entry to match" >&2
  exit 1
fi

if [[ "$(jq -r '.definition_of_done[1]' "$assembly_task")" != "Persist it in run artifacts" ]]; then
  echo "Expected second definition_of_done entry to match" >&2
  exit 1
fi

if [[ "$(jq -r '.recommended.approach' "$assembly_task")" != "Keep the context tight" ]]; then
  echo "Expected recommended approach to match" >&2
  exit 1
fi

if [[ "$(jq -r '.verification.commands | length' "$assembly_task")" != "1" ]]; then
  echo "Expected one verification command" >&2
  exit 1
fi

if [[ "$(jq -r '.verification.commands[0]' "$assembly_task")" != "echo verify" ]]; then
  echo "Expected verification command to match" >&2
  exit 1
fi
