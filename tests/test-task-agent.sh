#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

require_cmd jq
require_cmd git
require_cmd python

repo_dir="$(make_temp_dir)"
stub_bin="$(make_temp_dir)"
home_dir="$(make_temp_dir)"
trap 'rm -rf "$repo_dir" "$stub_bin" "$home_dir"' EXIT
cat > "$repo_dir/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "T1",
      "title": "Task agent smoke test",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Allow the stub codex to finish"
      ],
      "recommended": {
        "approach": "Treat the stub run as a simple success path"
      }
    }
  ]
}
JSON

mkdir -p "$home_dir/.prompts"
cat > "$home_dir/.prompts/autonomous-senior-engineer.prompt.md" <<'EOF2'
Test prompt
EOF2

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

(
  cd "$repo_dir"
  git init -b main >/dev/null
  git add README.md prd.json
  GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com \
  GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com \
  git commit -m "init" >/dev/null
)

HOME="$home_dir" \
PATH="$stub_bin:$PATH" \
  GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com \
  GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com \
  "$TEST_DIR/../bin/task-agent.sh" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --task-id T1 \
  --assignee test-assignee \
  >/dev/null

status="$(jq -r '.tasks[0].status' "$repo_dir/prd.json")"
if [[ "$status" != "completed" ]]; then
  echo "Expected task status to be completed, got: $status" >&2
  exit 1
fi
