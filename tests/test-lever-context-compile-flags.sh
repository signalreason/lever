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

cat > "$stub_dir/assembly" <<'EOF2'
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  printf '%s\n' "assembly 1.2.3"
  exit 0
fi

if [[ "${1:-}" == "build" && "${2:-}" == "--help" ]]; then
  cat <<'HELP'
Usage: assembly build [OPTIONS]

Options:
  --repo <PATH>           Repository root
  --task <PATH>           Task input file (supports @file)
  --task-id <ID>          Task identifier
  --out <DIR>             Output pack directory
  --token-budget <TOKENS> Token budget for context
  --exclude <GLOB>        Additive exclude glob (repeatable)
  --exclude-runtime <GLOB> Runtime artifact exclusion glob (repeatable)
  --summary-json <PATH>   Write machine-readable summary JSON
HELP
  exit 0
fi

echo "unexpected args: $*" >&2
exit 1
EOF2
chmod +x "$stub_dir/assembly"

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
  --assembly-path "$stub_dir/assembly" \
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

args_file="$args_dir/args-best-effort.txt"
ARGS_FILE="$args_file" "$lever_bin" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --command-path "$stub_dir/flag-stub" \
  --task-id T1 \
  --context-failure-policy best-effort \
  >/dev/null

if ! grep -Fxq -- "--context-failure-policy" "$args_file"; then
  echo "Expected --context-failure-policy to be passed to task agent" >&2
  exit 1
fi

if ! grep -Fxq -- "best-effort" "$args_file"; then
  echo "Expected best-effort policy value to be passed to task agent" >&2
  exit 1
fi

args_file="$args_dir/args-required.txt"
ARGS_FILE="$args_file" "$lever_bin" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --command-path "$stub_dir/flag-stub" \
  --task-id T1 \
  --context-failure-policy required \
  >/dev/null

if ! grep -Fxq -- "--context-failure-policy" "$args_file"; then
  echo "Expected --context-failure-policy to be passed to task agent (required)" >&2
  exit 1
fi

if ! grep -Fxq -- "required" "$args_file"; then
  echo "Expected required policy value to be passed to task agent" >&2
  exit 1
fi

args_file="$args_dir/args-token-budget.txt"
ARGS_FILE="$args_file" "$lever_bin" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --command-path "$stub_dir/flag-stub" \
  --task-id T1 \
  --context-token-budget 12345 \
  >/dev/null

if ! grep -Fxq -- "--context-token-budget" "$args_file"; then
  echo "Expected --context-token-budget to be passed to task agent" >&2
  exit 1
fi

if ! grep -Fxq -- "12345" "$args_file"; then
  echo "Expected token budget value to be passed to task agent" >&2
  exit 1
fi

args_file="$args_dir/args-lint-summary.txt"
ARGS_FILE="$args_file" "$lever_bin" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --command-path "$stub_dir/flag-stub" \
  --task-id T1 \
  --prompt-lint-summary \
  >/dev/null

if ! grep -Fxq -- "--prompt-lint-summary" "$args_file"; then
  echo "Expected --prompt-lint-summary to be passed to task agent" >&2
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

set +e
policy_output="$($lever_bin \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --command-path "$stub_dir/flag-stub" \
  --task-id T1 \
  --context-failure-policy nope \
  2>&1)"
policy_status=$?
set -e

if [[ $policy_status -eq 0 ]]; then
  echo "Expected invalid policy to fail" >&2
  exit 1
fi

if ! grep -q "possible values" <<<"$policy_output"; then
  echo "Expected invalid policy error to list possible values" >&2
  exit 1
fi

set +e
budget_output="$($lever_bin \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --command-path "$stub_dir/flag-stub" \
  --task-id T1 \
  --context-token-budget 0 \
  2>&1)"
budget_status=$?
set -e

if [[ $budget_status -eq 0 ]]; then
  echo "Expected invalid token budget to fail" >&2
  exit 1
fi

if ! grep -q "invalid value" <<<"$budget_output"; then
  echo "Expected invalid token budget error to mention invalid value" >&2
  exit 1
fi

if ! grep -q "context-token-budget" <<<"$budget_output"; then
  echo "Expected invalid token budget error to mention --context-token-budget" >&2
  exit 1
fi
