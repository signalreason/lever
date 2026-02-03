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
home_dir="$(make_temp_dir)"
trap 'rm -rf "$repo_dir" "$stub_bin" "$home_dir"' EXIT

cat > "$repo_dir/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "T1",
      "title": "Reject unsupported model",
      "status": "unstarted",
      "model": "gpt-5.3-codex",
      "definition_of_done": [
        "Fail fast on unsupported model"
      ],
      "recommended": {
        "approach": "Let the internal task agent validate the model allow-list."
      }
    }
  ]
}
JSON

mkdir -p "$home_dir/.prompts"
cat > "$home_dir/.prompts/autonomous-senior-engineer.prompt.md" <<'EOF_PROMPT'
Test prompt
EOF_PROMPT

cat > "$repo_dir/README.md" <<'EOF_README'
Test repo for unsupported model
EOF_README

cat > "$stub_bin/codex" <<'EOF_CODEX'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "--version" ]]; then
  echo "codex stub"
  exit 0
fi
exit 0
EOF_CODEX
chmod +x "$stub_bin/codex"

init_git_repo "$repo_dir"

(
  cd "$repo_root"
  cargo build --quiet --manifest-path "$repo_root/Cargo.toml"
)
lever_bin="$repo_root/target/debug/lever"

set +e
output=$(
  HOME="$home_dir" \
  PATH="$stub_bin:$PATH" \
  ASSIGNEE="test-assignee" \
  GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com \
  GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com \
  "$lever_bin" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --task-id T1 \
  2>&1
)
exit_code=$?
set -e

if [[ "$exit_code" -ne 2 ]]; then
  echo "Expected lever to exit 2 for unsupported model, got: $exit_code" >&2
  exit 1
fi

if [[ "$output" != *"Unsupported model in task T1: gpt-5.3-codex"* ]]; then
  echo "Expected unsupported model error to include task id and model, got: $output" >&2
  exit 1
fi
