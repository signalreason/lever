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
      "title": "Prompt lint summary",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Include lint summary"
      ],
      "recommended": {
        "approach": "Parse lint findings"
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
printf '%s\n' "{}" > "$out_dir/manifest.json"
printf '%s\n' "{}" > "$out_dir/index.json"
printf '%s\n' "Compiled context line 1" "Compiled context line 2" > "$out_dir/context.md"
printf '%s\n' "{}" > "$out_dir/policy.md"
cat > "$out_dir/lint.json" <<'JSON'
{
  "issues": [
    {"severity": "error", "message": "Bad thing", "path": "src/main.rs", "line": 10, "rule": "R100"},
    {"severity": "warning", "message": "Needs attention", "path": "src/lib.rs", "line": 5, "rule": "W200"},
    {"severity": "note", "message": "Optional improvement", "path": "src/lib.rs", "line": 8}
  ]
}
JSON

if [[ -n "$summary" ]]; then
  mkdir -p "$(dirname "$summary")"
  printf '%s\n' "{}" > "$summary"
fi
EOF2
chmod +x "$stub_bin/assembly"

init_git_repo "$repo_dir"
commit_sha="$(git -C "$repo_dir" rev-parse --short=12 HEAD)"

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
  --context-compile \
  --prompt-lint-summary \
  >/dev/null

run_dir="$(find "$repo_dir/.ralph/runs/T1" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
if [[ -z "$run_dir" ]]; then
  echo "Expected run directory to exist" >&2
  exit 1
fi

manifest_rel="${run_dir#"$repo_dir"/}/pack/manifest.json"

prompt_file="$run_dir/prompt.md"
if [[ ! -f "$prompt_file" ]]; then
  echo "Expected prompt at $prompt_file" >&2
  exit 1
fi

expected_file="$repo_dir/expected-prompt.txt"
cat > "$expected_file" <<EOF
Test prompt


Task title: Prompt lint summary

Definition of done:
  - Include lint summary

Recommended approach:
Parse lint findings

Task JSON (authoritative):
{"definition_of_done":["Include lint summary"],"model":"gpt-5.1-codex-mini","recommended":{"approach":"Parse lint findings"},"status":"unstarted","task_id":"T1","title":"Prompt lint summary"}

Lint summary:
Totals: error=1 warning=1 note=1 info=0 other=0
Findings (showing 3 of 3):
- [error] src/main.rs:10 R100 - Bad thing
- [warning] src/lib.rs:5 W200 - Needs attention
- [note] src/lib.rs:8 - Optional improvement

Compiled context:
Provenance: manifest=$manifest_rel commit=$commit_sha
Compiled context line 1
Compiled context line 2
EOF

if ! diff -u "$expected_file" "$prompt_file"; then
  echo "Expected prompt to include lint summary before compiled context" >&2
  exit 1
fi
