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
      "title": "Assembly exec logs",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Run assembly",
        "Capture stdout and stderr"
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

summary=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --summary-json)
      summary="$2"
      shift 2
      ;;
    *)
      shift 1
      ;;
  esac
done

if [[ -n "$summary" ]]; then
  mkdir -p "$(dirname "$summary")"
  printf '%s\n' "{}" > "$summary"
fi

echo "assembly stdout"
echo "assembly stderr" >&2
EOF2
chmod +x "$stub_bin/assembly"

init_git_repo "$repo_dir"

(
  cd "$repo_root"
  cargo build --quiet
)
lever_bin="$repo_root/target/debug/lever"

PATH="$stub_bin:$PATH" \
ASSIGNEE="test-assignee" \
  GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com \
  GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com \
  "$lever_bin" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --task-id T1 \
  --context-compile \
  >/dev/null

run_dir="$(find "$repo_dir/.ralph/runs/T1" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
if [[ -z "$run_dir" ]]; then
  echo "Expected run directory to exist" >&2
  exit 1
fi

stdout_log="$run_dir/assembly.stdout.log"
stderr_log="$run_dir/assembly.stderr.log"

if [[ ! -f "$stdout_log" ]]; then
  echo "Expected assembly stdout log at $stdout_log" >&2
  exit 1
fi

if [[ ! -f "$stderr_log" ]]; then
  echo "Expected assembly stderr log at $stderr_log" >&2
  exit 1
fi

if ! grep -Fxq "assembly stdout" "$stdout_log"; then
  echo "Expected stdout log to contain assembly output" >&2
  exit 1
fi

if ! grep -Fxq "assembly stderr" "$stderr_log"; then
  echo "Expected stderr log to contain assembly output" >&2
  exit 1
fi
