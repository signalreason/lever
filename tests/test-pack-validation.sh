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
      "title": "Pack validation",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Validate pack files"
      ],
      "recommended": {
        "approach": "Stub assembly"
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

out_dir=""
summary=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --out)
      out_dir="$2"
      shift 2
      ;;
    --summary-json)
      summary="$2"
      shift 2
      ;;
    *)
      shift 1
      ;;
  esac
done

if [[ -z "$out_dir" ]]; then
  echo "Missing --out" >&2
  exit 2
fi

mkdir -p "$out_dir"
required=(manifest.json index.json context.md policy.md lint.json)
for file in "${required[@]}"; do
  if [[ -n "${MISSING_FILE:-}" && "$file" == "$MISSING_FILE" ]]; then
    continue
  fi
  printf '%s\n' "data" > "$out_dir/$file"
done

if [[ -n "$summary" ]]; then
  mkdir -p "$(dirname "$summary")"
  printf '%s\n' "{}" > "$summary"
fi
EOF2
chmod +x "$stub_bin/assembly"

init_git_repo "$repo_dir"

(
  cd "$repo_root"
  cargo build --quiet
)
lever_bin="$repo_root/target/debug/lever"

for missing in "${required[@]}"; do
  marker_file="$repo_dir/codex-required-${missing}.marker"
  rm -f "$marker_file"
  set +e
  missing_output=$(PATH="$stub_bin:$PATH" \
    MISSING_FILE="$missing" \
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
  missing_status=$?
  set -e

  if [[ $missing_status -eq 0 ]]; then
    echo "Expected missing pack files to fail with required policy (missing $missing)" >&2
    exit 1
  fi

  if [[ "$missing_output" != *"Missing required pack files"* ]]; then
    echo "Expected missing pack files error, got: $missing_output" >&2
    exit 1
  fi

  if [[ "$missing_output" != *"$missing"* ]]; then
    echo "Expected missing pack files error to mention $missing, got: $missing_output" >&2
    exit 1
  fi

  if [[ -f "$marker_file" ]]; then
    echo "Expected required policy to fail before Codex execution (missing $missing)" >&2
    exit 1
  fi

  run_dir="$(ls -td "$repo_dir/.ralph/runs/T1"/* | head -n 1)"
  if [[ -z "$run_dir" ]]; then
    echo "Expected run directory to exist (missing $missing)" >&2
    exit 1
  fi

  compile_report="$run_dir/context-compile.json"
  if [[ ! -f "$compile_report" ]]; then
    echo "Expected context compile report at $compile_report (missing $missing)" >&2
    exit 1
  fi

  status="$(jq -r '.status' "$compile_report")"
  policy_outcome="$(jq -r '.policy_outcome' "$compile_report")"
  missing_files="$(jq -r '.pack_missing | join(",")' "$compile_report")"

  if [[ "$status" != "failed" ]]; then
    echo "Expected context compile status failed (missing $missing), got: $status" >&2
    exit 1
  fi

  if [[ "$policy_outcome" != "blocked" ]]; then
    echo "Expected policy_outcome blocked (missing $missing), got: $policy_outcome" >&2
    exit 1
  fi

  if [[ "$missing_files" != *"$missing"* ]]; then
    echo "Expected pack_missing to include $missing, got: $missing_files" >&2
    exit 1
  fi
done

set +e
marker_file="$repo_dir/codex-best-effort.marker"
rm -f "$marker_file"
warning_output=$(PATH="$stub_bin:$PATH" \
  MISSING_FILE="policy.md" \
  CODEX_MARKER="$marker_file" \
  GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com \
  GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com \
  "$lever_bin" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --task-id T1 \
  --context-compile \
  --context-failure-policy best-effort \
  2>&1)
warning_status=$?
set -e

if [[ $warning_status -ne 0 ]]; then
  echo "Expected best-effort pack validation to continue" >&2
  exit 1
fi

if [[ "$warning_output" != *"Missing required pack files"* ]]; then
  echo "Expected warning about missing pack files, got: $warning_output" >&2
  exit 1
fi

if [[ ! -f "$marker_file" ]]; then
  echo "Expected best-effort policy to continue into Codex execution" >&2
  exit 1
fi
