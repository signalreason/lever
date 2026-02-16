#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

require_cmd cargo
require_cmd git

repo_root="$(cd "$TEST_DIR/.." && pwd)"
repo_dir="$(make_temp_dir)"
stub_dir="$(make_temp_dir)"
args_dir="$(make_temp_dir)"
trap 'rm -rf "$repo_dir" "$stub_dir" "$args_dir"' EXIT

cat > "$repo_dir/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "T1",
      "title": "Context compile flag pass-through",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Ensure context compile flags are parsed"
      ],
      "recommended": {
        "approach": "Keep this test minimal"
      }
    }
  ]
}
JSON

ensure_workspace_prompt "$repo_dir"
init_git_repo "$repo_dir"

mkdir -p "$stub_dir"
cat > "$stub_dir/flag-stub" <<'EOF2'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$@" > "${ARGS_FILE}"
exit 0
EOF2
chmod +x "$stub_dir/flag-stub"

(
  cd "$repo_root"
  cargo build --quiet
)
lever_bin="$repo_root/target/debug/lever"

args_file="$args_dir/args-enable.txt"
ARGS_FILE="$args_file" "$lever_bin" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --command-path "$stub_dir/flag-stub" \
  --task-id T1 \
  --context-compile \
  >/dev/null

if ! grep -Fxq -- "--context-compile" "$args_file"; then
  echo "Expected --context-compile to be passed to task agent" >&2
  exit 1
fi

args_file="$args_dir/args-disable.txt"
ARGS_FILE="$args_file" "$lever_bin" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --command-path "$stub_dir/flag-stub" \
  --task-id T1 \
  --no-context-compile \
  >/dev/null

if ! grep -Fxq -- "--no-context-compile" "$args_file"; then
  echo "Expected --no-context-compile to be passed to task agent" >&2
  exit 1
fi

set +e
conflict_output="$($lever_bin \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --command-path "$stub_dir/flag-stub" \
  --task-id T1 \
  --context-compile \
  --no-context-compile \
  2>&1)"
conflict_status=$?
set -e

if [[ $conflict_status -eq 0 ]]; then
  echo "Expected conflicting context compile flags to fail" >&2
  exit 1
fi

if ! grep -q "cannot be used" <<<"$conflict_output"; then
  echo "Expected a conflict error for context compile flags" >&2
  exit 1
fi
