#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

require_cmd cargo
require_cmd git

repo_dir="$(make_temp_dir)"
trap 'rm -rf "$repo_dir"' EXIT

cat > "$repo_dir/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "T1",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [],
      "recommended": {
        "approach": ""
      }
    }
  ]
}
JSON

cat > "$repo_dir/prompt.md" <<'EOF2'
Test prompt
EOF2

marker_file="$repo_dir/marker"
cat > "$repo_dir/stub-cmd" <<'EOF2'
#!/usr/bin/env bash
set -euo pipefail
: "${MARKER_FILE:?}"
: > "$MARKER_FILE"
EOF2
chmod +x "$repo_dir/stub-cmd"

init_git_repo "$repo_dir"

set +e
output="$(MARKER_FILE="$marker_file" cargo run --quiet --manifest-path "$TEST_DIR/../Cargo.toml" \
  -- --tasks "$repo_dir/prd.json" --prompt "$repo_dir/prompt.md" --command-path "$repo_dir/stub-cmd" 2>&1)"
status=$?
set -e

if [[ $status -eq 0 ]]; then
  echo "Expected lever to fail on missing task metadata" >&2
  exit 1
fi

if ! grep -q "Task T1 missing required metadata: title, definition_of_done, recommended.approach" <<<"$output"; then
  echo "Expected lever to report missing metadata, got: $output" >&2
  exit 1
fi

if [[ -f "$marker_file" ]]; then
  echo "Expected command-path not to run when metadata is invalid" >&2
  exit 1
fi
