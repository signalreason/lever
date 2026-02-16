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
      "title": "Required context compile failure",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Fail fast when context compilation fails"
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

if [[ "${1:-}" == "--version" ]]; then
  printf '%s\n' "codex 0.0.0"
  exit 0
fi

marker="${CODEX_MARKER:-}"
if [[ -n "$marker" ]]; then
  printf '%s\n' "codex invoked" > "$marker"
fi

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

marker_file="$repo_dir/codex-required.marker"
rm -f "$marker_file"

set +e
run_output=$(PATH="$stub_bin:$PATH" \
  CODEX_MARKER="$marker_file" \
  GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com \
  GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com \
  "$lever_bin" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --task-id T1 \
  --context-compile \
  --context-failure-policy required \
  2>&1)
run_status=$?
set -e

if [[ $run_status -ne 13 ]]; then
  echo "Expected required policy to exit 13 on assembly failure, got: $run_status" >&2
  exit 1
fi

if [[ "$run_output" != *"Assembly failed"* ]]; then
  echo "Expected required failure to mention assembly error, got: $run_output" >&2
  exit 1
fi

if [[ "$run_output" != *"Blocked:"* ]]; then
  echo "Expected required failure to be reported as blocked, got: $run_output" >&2
  exit 1
fi

if [[ "$run_output" != *"Context compile report"* ]]; then
  echo "Expected context compile report log, got: $run_output" >&2
  exit 1
fi

if [[ -f "$marker_file" ]]; then
  echo "Expected required policy to fail before Codex execution" >&2
  exit 1
fi

run_dir="$(find "$repo_dir/.ralph/runs/T1" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
if [[ -z "$run_dir" ]]; then
  echo "Expected run directory to exist" >&2
  exit 1
fi

pack_rel="${run_dir#"$repo_dir"/}/pack"
note="$(jq -r '.tasks[0].observability.last_note // ""' "$repo_dir/prd.json")"
if [[ "$note" != *"context_compile=failed"* ]]; then
  echo "Expected last_note to include context compile status, got: $note" >&2
  exit 1
fi

if [[ "$note" != *"policy=required"* ]]; then
  echo "Expected last_note to include policy, got: $note" >&2
  exit 1
fi

if [[ "$note" != *"policy_outcome=blocked"* ]]; then
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

if [[ "$status" != "failed" ]]; then
  echo "Expected context compile status failed, got: $status" >&2
  exit 1
fi

if [[ "$policy" != "required" ]]; then
  echo "Expected context compile policy required, got: $policy" >&2
  exit 1
fi

if [[ "$policy_outcome" != "blocked" ]]; then
  echo "Expected policy_outcome blocked, got: $policy_outcome" >&2
  exit 1
fi

if [[ "$pack_dir" != "$pack_rel" ]]; then
  echo "Expected pack_dir $pack_rel, got: $pack_dir" >&2
  exit 1
fi
