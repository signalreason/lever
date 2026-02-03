#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

require_cmd jq
require_cmd cargo
require_cmd git

repo_root="$(cd "$TEST_DIR/.." && pwd)"
repo_dir="$(make_temp_dir)"
home_dir="$(make_temp_dir)"
stub_dir="$(make_temp_dir)"
args_dir="$(make_temp_dir)"
trap 'rm -rf "$repo_dir" "$home_dir" "$stub_dir" "$args_dir"' EXIT
cat > "$repo_dir/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "T1",
      "title": "Ralph loop stub",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Verify the loop delegates task execution"
      ],
      "recommended": {
        "approach": "Keep this stub simple"
      },
      "observability": {
        "run_attempts": 0,
        "last_note": "",
        "last_run_id": "ralph-loop-init",
        "last_update_utc": "2026-02-01T00:00:00Z"
      }
    }
  ]
}
JSON

init_git_repo "$repo_dir"
workspace_real="$(cd "$repo_dir" && pwd -P)"

mkdir -p "$home_dir/.prompts"
cat > "$home_dir/.prompts/autonomous-senior-engineer.prompt.md" <<'EOF2'
Test prompt
EOF2
prompt_real="$(cd "$home_dir" && pwd -P)/.prompts/autonomous-senior-engineer.prompt.md"

mkdir -p "$stub_dir"
cat > "$stub_dir/task-agent" <<'EOF2'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$@" > "${ARGS_FILE}"
exit 3
EOF2
chmod +x "$stub_dir/task-agent"

args_file="$args_dir/args.txt"

(
  cd "$repo_root"
  cargo build --quiet
)
lever_bin="$repo_root/target/debug/lever"

HOME="$home_dir" \
ARGS_FILE="$args_file" "$lever_bin" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --command-path "$stub_dir/task-agent" \
  --assignee test-assignee \
  --loop 1 \
  --delay 0 \
  >/dev/null

if ! grep -Fxq -- "--workspace" "$args_file"; then
  echo "Expected --workspace to be passed to task agent" >&2
  exit 1
fi
if ! grep -Fxq -- "$workspace_real" "$args_file"; then
  echo "Expected workspace path to be passed to task agent" >&2
  exit 1
fi

if ! grep -Fxq -- "--tasks" "$args_file"; then
  echo "Expected tasks path to be resolved against workspace" >&2
  exit 1
fi
if ! grep -Fxq -- "$workspace_real/prd.json" "$args_file"; then
  echo "Expected tasks path to be resolved against workspace" >&2
  exit 1
fi

if ! grep -Fxq -- "--prompt" "$args_file"; then
  echo "Expected prompt path to be resolved against workspace" >&2
  exit 1
fi
if ! grep -Fxq -- "$prompt_real" "$args_file"; then
  echo "Expected prompt path to use the default ~/.prompts location" >&2
  exit 1
fi
