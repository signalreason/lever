#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

require_cmd cargo
require_cmd git
require_cmd jq
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
      "title": "Leverage the internal task-agent",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Run the task-agent through lever"
      ],
      "recommended": {
        "approach": "Let task-agent drive the run and update the tasks file."
      }
    }
  ]
}
JSON

ensure_workspace_prompt "$repo_dir"

cat > "$repo_dir/README.md" <<'EOF'
Test repo for lever execution
EOF

mkdir -p "$repo_dir/tests"
cat > "$repo_dir/tests/run.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo "verification run executed"
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

cat <<'JSONL'
{"type":"turn.completed","usage":{"input_tokens":5,"output_tokens":7}}
JSONL

cat > "$out_path" <<'JSON'
{
  "task_id": "T1",
  "outcome": "completed",
  "dod_met": true,
  "summary": "lever delegation",
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
  PATH="$stub_bin:$PATH" \
    ASSIGNEE="lever-test" \
    GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com \
    GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com \
    "$lever_bin" \
    --tasks prd.json \
    --task-id T1 \
    >/dev/null
)

status="$(jq -r '.tasks[0].status' "$repo_dir/prd.json")"
if [[ "$status" != "completed" ]]; then
  echo "Expected lever to mark the task as completed" >&2
  exit 1
fi

if git -C "$repo_dir" show-ref --verify --quiet refs/heads/ralph/T1; then
  echo "Expected ralph/T1 branch to be deleted after completion" >&2
  exit 1
fi

subject="$(git -C "$repo_dir" log -1 --pretty=%s)"
if [[ "$subject" != "Leverage the internal task-agent" ]]; then
  echo "Expected completion commit subject, got: $subject" >&2
  exit 1
fi
