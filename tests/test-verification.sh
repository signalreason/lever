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
      "title": "Verification smoke test",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Ensure codex stub returns success"
      ],
      "recommended": {
        "approach": "Treat verification as a simple placeholder run"
      },
      "observability": {
        "run_attempts": 0,
        "last_note": "",
        "last_run_id": "verification-init",
        "last_update_utc": "2026-02-01T00:00:00Z"
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

mkdir -p "$repo_dir/tests"
cat > "$repo_dir/tests/run.sh" <<'EOF2'
#!/usr/bin/env bash
set -euo pipefail
echo "run.sh ok"
EOF2
chmod +x "$repo_dir/tests/run.sh"

cat > "$stub_bin/pytest" <<'EOF2'
#!/usr/bin/env bash
set -euo pipefail
echo "pytest should not run" >&2
exit 99
EOF2
chmod +x "$stub_bin/pytest"

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
  git add README.md prd.json tests/run.sh
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

run_dir="$(ls -td "$repo_dir/.ralph/runs/T1"/* | head -n1)"
if ! grep -q "run.sh ok" "$run_dir/verify.log"; then
  echo "Expected tests/run.sh to be used for verification" >&2
  exit 1
fi
