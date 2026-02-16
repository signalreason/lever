#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

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
      "title": "Prompt without context",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Do not append compiled context"
      ],
      "recommended": {
        "approach": "Skip context compilation"
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

prompt_file="$run_dir/prompt.md"
if [[ ! -f "$prompt_file" ]]; then
  echo "Expected prompt at $prompt_file" >&2
  exit 1
fi

expected_file="$repo_dir/expected-prompt.txt"
cat > "$expected_file" <<'EOF'
Test prompt


Task title: Prompt without context

Definition of done:
  - Do not append compiled context

Recommended approach:
Skip context compilation

Task JSON (authoritative):
{"definition_of_done":["Do not append compiled context"],"model":"gpt-5.1-codex-mini","recommended":{"approach":"Skip context compilation"},"status":"unstarted","task_id":"T1","title":"Prompt without context"}
EOF

if ! diff -u "$expected_file" "$prompt_file"; then
  echo "Expected prompt to remain unchanged when compiled context is absent" >&2
  exit 1
fi
