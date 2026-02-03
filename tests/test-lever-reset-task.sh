#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

require_cmd cargo
require_cmd git
require_cmd jq

repo_root="$(cd "$TEST_DIR/.." && pwd)"
repo_dir="$(make_temp_dir)"
stub_bin="$(make_temp_dir)"
home_dir="$(make_temp_dir)"
trap 'rm -rf "$repo_dir" "$stub_bin" "$home_dir"' EXIT

cat > "$repo_dir/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "T1",
      "title": "Reset task attempts",
      "status": "blocked",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Reset attempts and run"
      ],
      "recommended": {
        "approach": "Exercise reset-task behavior."
      },
      "observability": {
        "run_attempts": 2,
        "last_note": "prior failure",
        "last_run_id": "old-run",
        "last_update_utc": "2026-02-01T00:00:00Z"
      }
    }
  ]
}
JSON

mkdir -p "$home_dir/.prompts"
cat > "$home_dir/.prompts/autonomous-senior-engineer.prompt.md" <<'EOF'
Test prompt
EOF

cat > "$repo_dir/README.md" <<'EOF'
Test repo for reset-task behavior
EOF

mkdir -p "$repo_dir/tests"
cat > "$repo_dir/tests/run.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "verification run executed"
EOF
chmod +x "$repo_dir/tests/run.sh"

cat > "$stub_bin/codex" <<'EOF'
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
  "summary": "reset run ok",
  "tests": {"ran": false, "commands": [], "passed": true},
  "notes": "",
  "blockers": []
}
JSON
EOF
chmod +x "$stub_bin/codex"

init_git_repo "$repo_dir"

cargo build --quiet --manifest-path "$repo_root/Cargo.toml"
lever_bin="$repo_root/target/debug/lever"

(
  cd "$repo_dir"
  HOME="$home_dir" \
    PATH="$stub_bin:$PATH" \
    ASSIGNEE="lever-test" \
    GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com \
    GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com \
    "$lever_bin" \
    --tasks prd.json \
    --task-id T1 \
    --reset-task \
    >/dev/null
)

status="$(jq -r '.tasks[0].status' "$repo_dir/prd.json")"
if [[ "$status" != "completed" ]]; then
  echo "Expected reset-task run to complete, got status: $status" >&2
  exit 1
fi

run_attempts="$(jq -r '.tasks[0].observability.run_attempts' "$repo_dir/prd.json")"
if [[ "$run_attempts" != "1" ]]; then
  echo "Expected run_attempts to reset to 1 after run, got: $run_attempts" >&2
  exit 1
fi

run_count="$(find "$repo_dir/.ralph/runs/T1" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')"
if [[ "$run_count" -ne 1 ]]; then
  echo "Expected exactly one run directory, got: $run_count" >&2
  exit 1
fi

run_id="$(find "$repo_dir/.ralph/runs/T1" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
run_id="$(basename "$run_id")"

last_run_id="$(jq -r '.tasks[0].observability.last_run_id' "$repo_dir/prd.json")"
if [[ "$last_run_id" != "$run_id" ]]; then
  echo "Expected last_run_id to match run dir ($run_id), got: $last_run_id" >&2
  exit 1
fi

if [[ "$last_run_id" == "old-run" ]]; then
  echo "Expected last_run_id to change from seed value" >&2
  exit 1
fi
