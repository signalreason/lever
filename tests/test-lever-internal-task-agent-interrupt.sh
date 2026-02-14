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
      "title": "Interrupt internal task-agent",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Handle interrupts"
      ],
      "recommended": {
        "approach": "Simulate a Ctrl-C while codex is running."
      }
    }
  ]
}
JSON

ensure_workspace_prompt "$repo_dir"

cat > "$repo_dir/README.md" <<'EOF'
Test repo for lever execution
EOF

cat > "$stub_bin/codex" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
sleep 5
EOF
chmod +x "$stub_bin/codex"

init_git_repo "$repo_dir"

cargo build --quiet --manifest-path "$repo_root/Cargo.toml"
lever_bin="$repo_root/target/debug/lever"

(
  cd "$repo_dir"
  PATH="$stub_bin:$PATH" \
    GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com \
    GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com \
    "$lever_bin" \
    --tasks prd.json \
    --task-id T1 &
  lever_pid=$!

  attempts=0
  while [[ ! -d ".ralph/runs/T1" ]]; do
    sleep 0.05
    attempts=$((attempts + 1))
    if [[ "$attempts" -gt 200 ]]; then
      kill "$lever_pid" 2>/dev/null || true
      wait "$lever_pid" 2>/dev/null || true
      echo "Timed out waiting for run directory" >&2
      exit 1
    fi
  done

  sleep 0.1
  kill -INT "$lever_pid"

  if ! wait "$lever_pid"; then
    echo "Expected lever to exit cleanly after interrupt" >&2
    exit 1
  fi
)

status="$(jq -r '.tasks[0].status' "$repo_dir/prd.json")"
if [[ "$status" != "started" ]]; then
  echo "Expected task status to remain started after interrupt" >&2
  exit 1
fi

note="$(jq -r '.tasks[0].observability.last_note // ""' "$repo_dir/prd.json")"
if [[ "$note" != *interrupted* ]]; then
  echo "Expected interrupt note in task observability" >&2
  exit 1
fi

subject="$(git -C "$repo_dir" log -1 --pretty=%s)"
if [[ "$subject" != "Interrupt internal task-agent" ]]; then
  echo "Expected interrupt commit subject, got: $subject" >&2
  exit 1
fi
