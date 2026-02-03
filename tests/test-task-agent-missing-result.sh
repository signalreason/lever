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
home_dir="$(make_temp_dir)"
trap 'rm -rf "$repo_dir" "$stub_bin" "$home_dir"' EXIT

cat > "$repo_dir/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "T1",
      "title": "Missing result.json",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Simulate missing result.json"
      ],
      "recommended": {
        "approach": "Allow the codex stub to exit without writing a result."
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
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output-last-message)
      shift 2
      ;;
    *)
      shift 1
      ;;
  esac
done

# Exit cleanly without writing result.json.
exit 0
EOF2
chmod +x "$stub_bin/codex"

init_git_repo "$repo_dir"

(
  cd "$repo_root"
  cargo build --quiet
)
lever_bin="$repo_root/target/debug/lever"

set +e
HOME="$home_dir" \
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

if [[ "$exit_code" -ne 10 ]]; then
  echo "Expected lever to exit 10 when result.json is missing, got: $exit_code" >&2
  exit 1
fi

status="$(jq -r '.tasks[0].status' "$repo_dir/prd.json")"
if [[ "$status" != "blocked" ]]; then
  echo "Expected task status to be blocked, got: $status" >&2
  exit 1
fi

note="$(jq -r '.tasks[0].observability.last_note // ""' "$repo_dir/prd.json")"
if [[ "$note" != *"codex.jsonl"* ]]; then
  echo "Expected last_note to reference codex log, got: $note" >&2
  exit 1
fi

run_dir="$(find "$repo_dir/.ralph/runs/T1" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
if [[ -z "$run_dir" ]]; then
  echo "Expected run directory to exist" >&2
  exit 1
fi

if [[ ! -f "$run_dir/codex.jsonl" ]]; then
  echo "Expected codex log at $run_dir/codex.jsonl" >&2
  exit 1
fi
