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
      "title": "Legacy already done",
      "status": "completed",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Nothing left"
      ],
      "recommended": {
        "approach": "Skip"
      }
    },
    {
      "task_id": "T2",
      "title": "Legacy disabled context compile",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Keep legacy prompt and notes"
      ],
      "recommended": {
        "approach": "No context compilation"
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
  "task_id": "T2",
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
  GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com \
  GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com \
  "$lever_bin" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --next \
  --no-context-compile \
  >/dev/null

status_t1="$(jq -r '.tasks[0].status' "$repo_dir/prd.json")"
status_t2="$(jq -r '.tasks[1].status' "$repo_dir/prd.json")"
if [[ "$status_t1" != "completed" ]]; then
  echo "Expected T1 to remain completed, got: $status_t1" >&2
  exit 1
fi
if [[ "$status_t2" != "completed" ]]; then
  echo "Expected T2 to be completed, got: $status_t2" >&2
  exit 1
fi

run_dir="$(find "$repo_dir/.ralph/runs/T2" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
if [[ -z "$run_dir" ]]; then
  echo "Expected run directory to exist for T2" >&2
  exit 1
fi

prompt_file="$run_dir/prompt.md"
if [[ ! -f "$prompt_file" ]]; then
  echo "Expected prompt at $prompt_file" >&2
  exit 1
fi

expected_file="$repo_dir/expected-prompt.txt"
cat > "$expected_file" <<'EOF'
Test prompt


Task title: Legacy disabled context compile

Definition of done:
  - Keep legacy prompt and notes

Recommended approach:
No context compilation

Task JSON (authoritative):
{"definition_of_done":["Keep legacy prompt and notes"],"model":"gpt-5.1-codex-mini","recommended":{"approach":"No context compilation"},"status":"unstarted","task_id":"T2","title":"Legacy disabled context compile"}
EOF

if ! diff -u "$expected_file" "$prompt_file"; then
  echo "Expected prompt to match legacy output when context compile is disabled" >&2
  exit 1
fi

if grep -q "Compiled context:" "$prompt_file"; then
  echo "Expected prompt to omit compiled context when disabled" >&2
  exit 1
fi

if grep -q "Lint summary" "$prompt_file"; then
  echo "Expected prompt to omit lint summary when disabled" >&2
  exit 1
fi

note="$(jq -r '.tasks[1].observability.last_note // ""' "$repo_dir/prd.json")"
if [[ "$note" == *"context_compile="* ]]; then
  echo "Expected last_note to omit context compile details, got: $note" >&2
  exit 1
fi

compile_report="$run_dir/context-compile.json"
if [[ -f "$compile_report" ]]; then
  echo "Expected no context compile report when disabled, found: $compile_report" >&2
  exit 1
fi
