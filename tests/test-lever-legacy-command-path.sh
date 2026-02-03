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

cat > "$repo_dir/prd.json" <<'JSON_EOF'
{
  "tasks": [
    {
      "task_id": "LEGACY-1",
      "title": "Legacy command-path fallback",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Run the internal task-agent when legacy command-path is missing"
      ],
      "recommended": {
        "approach": "Pass bin/task-agent.sh and ensure the internal agent runs."
      }
    }
  ]
}
JSON_EOF

ensure_workspace_prompt "$repo_dir"

cat > "$repo_dir/README.md" <<'README_EOF'
Test repo for lever legacy command-path
README_EOF

cat > "$stub_bin/codex" <<'CODEX_EOF'
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

cat <<'JSONL_EOF'
{"type":"turn.completed","usage":{"input_tokens":5,"output_tokens":7}}
JSONL_EOF

cat > "$out_path" <<'JSON_OUT_EOF'
{
  "task_id": "LEGACY-1",
  "outcome": "completed",
  "dod_met": true,
  "summary": "legacy fallback",
  "tests": {"ran": false, "commands": [], "passed": true},
  "notes": "",
  "blockers": []
}
JSON_OUT_EOF
CODEX_EOF
chmod +x "$stub_bin/codex"

init_git_repo "$repo_dir"

cargo build --quiet --manifest-path "$repo_root/Cargo.toml"
lever_bin="$repo_root/target/debug/lever"

(
  cd "$repo_dir"
  PATH="$stub_bin:$PATH" \
    GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com \
    GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com \
    "$lever_bin" \
    --tasks prd.json \
    --task-id LEGACY-1 \
    --command-path bin/task-agent.sh \
    >/dev/null
)

status="$(jq -r '.tasks[0].status' "$repo_dir/prd.json")"
if [[ "$status" != "completed" ]]; then
  echo "Expected lever to mark the task as completed" >&2
  exit 1
fi
