#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

require_cmd jq

repo_dir="$(make_temp_dir)"
home_dir="$(make_temp_dir)"
trap 'rm -rf "$repo_dir" "$home_dir"' EXIT
cat > "$repo_dir/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "T1",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini"
    }
  ]
}
JSON

mkdir -p "$home_dir/.prompts"
cat > "$home_dir/.prompts/autonomous-senior-engineer.prompt.md" <<'EOF2'
Test prompt
EOF2

stub_dir="$repo_dir/stubs"
mkdir -p "$stub_dir"
cat > "$stub_dir/task-agent" <<'EOF2'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$@" > "${ARGS_FILE}"
exit 3
EOF2
chmod +x "$stub_dir/task-agent"

args_file="$repo_dir/args.txt"
run_dir="$(make_temp_dir)"
trap 'rm -rf "$run_dir"' EXIT

HOME="$home_dir" \
ARGS_FILE="$args_file" "$TEST_DIR/../bin/ralph-loop.sh" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --task-agent "$stub_dir/task-agent" \
  --assignee test-assignee \
  --delay 0 \
  >/dev/null

if ! grep -Fxq -- "--workspace" "$args_file"; then
  echo "Expected --workspace to be passed to task agent" >&2
  exit 1
fi
if ! grep -Fxq -- "$repo_dir" "$args_file"; then
  echo "Expected workspace path to be passed to task agent" >&2
  exit 1
fi

if ! grep -Fxq -- "--tasks" "$args_file"; then
  echo "Expected tasks path to be resolved against workspace" >&2
  exit 1
fi
if ! grep -Fxq -- "$repo_dir/prd.json" "$args_file"; then
  echo "Expected tasks path to be resolved against workspace" >&2
  exit 1
fi

if ! grep -Fxq -- "--prompt" "$args_file"; then
  echo "Expected prompt path to be resolved against workspace" >&2
  exit 1
fi
if ! grep -Fxq -- "$home_dir/.prompts/autonomous-senior-engineer.prompt.md" "$args_file"; then
  echo "Expected prompt path to use the default ~/.prompts location" >&2
  exit 1
fi
