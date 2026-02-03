#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

require_cmd cargo
require_cmd git

repo_root="$(cd "$TEST_DIR/.." && pwd)"
repo_dir="$(make_temp_dir)"
stub_bin="$(make_temp_dir)"
trap 'rm -rf "$repo_dir" "$stub_bin"' EXIT

cat > "$repo_dir/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "FIRST",
      "title": "First pending task",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": ["placeholder"],
      "recommended": {"approach": "n/a"}
    },
    {
      "task_id": "SECOND",
      "title": "Later task",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": ["placeholder"],
      "recommended": {"approach": "n/a"}
    }
  ]
}
JSON

init_git_repo "$repo_dir"

cat > "$repo_dir/prompt.md" <<'EOF2'
Test prompt
EOF2

cat > "$stub_bin/codex" <<'EOF2'
#!/usr/bin/env bash
set -euo pipefail

for arg in "$@"; do
  if [[ "$arg" == "--version" ]]; then
    exit 0
  fi
done

echo "codex should not be invoked for task ordering" >&2
exit 99
EOF2
chmod +x "$stub_bin/codex"

set +e
output="$(
  PATH="$stub_bin:$PATH" \
  cargo run --quiet --manifest-path "$repo_root/Cargo.toml" \
    -- --workspace "$repo_dir" --tasks "$repo_dir/prd.json" --prompt "$repo_dir/prompt.md" --task-id SECOND 2>&1
)"
status=$?
set -e

if [[ $status -ne 6 ]]; then
  echo "Expected exit code 6 for dependency ordering, got $status" >&2
  exit 1
fi

if ! grep -q "Task SECOND cannot start until FIRST is completed." <<<"$output"; then
  echo "Expected blocking-task message on stderr" >&2
  exit 1
fi
