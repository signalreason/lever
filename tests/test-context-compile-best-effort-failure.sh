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
      "title": "Best-effort context compile failure",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Keep running when context compilation fails"
      ],
      "recommended": {
        "approach": "Stub assembly to fail"
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

cat > "$stub_bin/assembly" <<'EOF2'
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

echo "assembly build failed" >&2
exit 3
EOF2
chmod +x "$stub_bin/assembly"

init_git_repo "$repo_dir"

(
  cd "$repo_root"
  cargo build --quiet
)
lever_bin="$repo_root/target/debug/lever"

set +e
run_output=$(PATH="$stub_bin:$PATH" \
  GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com \
  GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com \
  "$lever_bin" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --task-id T1 \
  --context-compile \
  --context-failure-policy best-effort \
  2>&1)
run_status=$?
set -e

if [[ $run_status -ne 0 ]]; then
  echo "Expected best-effort policy to continue after assembly failure" >&2
  exit 1
fi

if [[ "$run_output" != *"Context compilation failed (best-effort); continuing without compiled context."* ]]; then
  echo "Expected best-effort warning about context compilation failure, got: $run_output" >&2
  exit 1
fi

if [[ "$run_output" != *"Context compile report"* ]]; then
  echo "Expected context compile report log, got: $run_output" >&2
  exit 1
fi

run_dir="$(find "$repo_dir/.ralph/runs/T1" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
if [[ -z "$run_dir" ]]; then
  echo "Expected run directory to exist" >&2
  exit 1
fi

prompt_file="$run_dir/prompt.md"
if [[ ! -f "$prompt_file" ]]; then
  echo "Expected prompt to be written at $prompt_file" >&2
  exit 1
fi

if grep -q "Compiled context:" "$prompt_file"; then
  echo "Expected prompt to omit compiled context after best-effort failure" >&2
  exit 1
fi

pack_rel="${run_dir#"$repo_dir"/}/pack"
note="$(jq -r '.tasks[0].observability.last_note // ""' "$repo_dir/prd.json")"
if [[ "$note" != *"context_compile=failed"* ]]; then
  echo "Expected last_note to include context compile status, got: $note" >&2
  exit 1
fi

if [[ "$note" != *"policy=best-effort"* ]]; then
  echo "Expected last_note to include policy, got: $note" >&2
  exit 1
fi

if [[ "$note" != *"policy_outcome=continued"* ]]; then
  echo "Expected last_note to include policy outcome, got: $note" >&2
  exit 1
fi

if [[ "$note" != *"pack_dir=$pack_rel"* ]]; then
  echo "Expected last_note to include pack_dir, got: $note" >&2
  exit 1
fi

compile_report="$run_dir/context-compile.json"
if [[ ! -f "$compile_report" ]]; then
  echo "Expected context compile report at $compile_report" >&2
  exit 1
fi

status="$(jq -r '.status' "$compile_report")"
policy="$(jq -r '.policy' "$compile_report")"
policy_outcome="$(jq -r '.policy_outcome' "$compile_report")"
pack_dir="$(jq -r '.pack_dir' "$compile_report")"
missing_first="$(jq -r '.pack_missing[0]' "$compile_report")"

if [[ "$status" != "failed" ]]; then
  echo "Expected context compile status failed, got: $status" >&2
  exit 1
fi

if [[ "$policy" != "best-effort" ]]; then
  echo "Expected context compile policy best-effort, got: $policy" >&2
  exit 1
fi

if [[ "$policy_outcome" != "continued" ]]; then
  echo "Expected policy_outcome continued, got: $policy_outcome" >&2
  exit 1
fi

if [[ "$pack_dir" != "$pack_rel" ]]; then
  echo "Expected pack_dir $pack_rel, got: $pack_dir" >&2
  exit 1
fi

if [[ "$missing_first" != "manifest.json" ]]; then
  echo "Expected pack_missing to list required files, got: $missing_first" >&2
  exit 1
fi

result_file="$run_dir/result.json"
if [[ ! -f "$result_file" ]]; then
  echo "Expected result.json from Codex run at $result_file" >&2
  exit 1
fi
