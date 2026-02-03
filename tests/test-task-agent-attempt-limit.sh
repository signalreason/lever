#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

require_cmd cargo
require_cmd git
require_cmd jq

repo_root="$(cd "$TEST_DIR/.." && pwd)"
repo_dir="$(make_temp_dir)"
stub_bin="$(make_temp_dir)"
trap 'rm -rf "$repo_dir" "$stub_bin"' EXIT

cat > "$repo_dir/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "T1",
      "title": "Attempt limit guard",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Block when attempts reach max"
      ],
      "recommended": {
        "approach": "Seed run_attempts at the max and ensure codex never runs."
      },
      "observability": {
        "run_attempts": 3,
        "last_note": "prior failure",
        "last_run_id": "seed-run",
        "last_update_utc": "2026-02-01T00:00:00Z"
      }
    }
  ]
}
JSON

ensure_workspace_prompt "$repo_dir"

cat > "$repo_dir/README.md" <<'README'
Test repo
README

cat > "$stub_bin/codex" <<'CODEX'
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  printf '%s\n' "codex stub 0.0.0"
  exit 0
fi

if [[ -z "${CODEX_CALLED_FILE:-}" ]]; then
  echo "CODEX_CALLED_FILE not set" >&2
  exit 2
fi

printf '%s\n' "codex invoked" >> "$CODEX_CALLED_FILE"
exit 99
CODEX
chmod +x "$stub_bin/codex"

init_git_repo "$repo_dir"

cargo build --quiet --manifest-path "$repo_root/Cargo.toml"
lever_bin="$repo_root/target/debug/lever"

set +e
PATH="$stub_bin:$PATH" \
CODEX_CALLED_FILE="$repo_dir/codex-called.log" \
  GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com \
  GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com \
  "$lever_bin" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --task-id T1 \
  >/dev/null 2>&1
exit_code=$?
set -e

if [[ "$exit_code" -ne 11 ]]; then
  echo "Expected lever to exit 11 when attempt limit reached, got: $exit_code" >&2
  exit 1
fi

status="$(jq -r '.tasks[0].status' "$repo_dir/prd.json")"
if [[ "$status" != "blocked" ]]; then
  echo "Expected task status to be blocked, got: $status" >&2
  exit 1
fi

if [[ -f "$repo_dir/codex-called.log" ]]; then
  echo "Expected codex to not be invoked when attempt limit reached" >&2
  exit 1
fi
