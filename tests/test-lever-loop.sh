#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$TEST_DIR/helpers.sh"

require_cmd cargo
require_cmd git
require_cmd jq

cargo build --quiet

lever_bin="$PWD/target/debug/lever"
if [[ ! -x "$lever_bin" ]]; then
  echo "lever binary missing after cargo build" >&2
  exit 1
fi

stub_dir="$(make_temp_dir)"
trap 'rm -rf "$stub_dir"' EXIT

stub="$stub_dir/loop-stub"
cat > "$stub" <<'EOF2'
#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${LOG_PATH:-}" ]]; then
  echo "LOG_PATH is not set" >&2
  exit 1
fi

printf '%s\n' "$(date +%s%N)" >> "$LOG_PATH"
sleep "${SLEEP_DURATION:-0.2}"
EOF2
chmod +x "$stub"

codex_stub="$stub_dir/codex"
cat > "$codex_stub" <<'EOF2'
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
  "task_id": "loop-task",
  "outcome": "completed",
  "dod_met": true,
  "summary": "ok",
  "tests": {"ran": false, "commands": [], "passed": true},
  "notes": "",
  "blockers": []
}
JSON
EOF2
chmod +x "$codex_stub"

run_loop_limit_test() {
  local limit="$1"
  local workspace
  workspace="$(make_temp_dir)"
  local home_dir
  home_dir="$(make_temp_dir)"
  local log_dir
  log_dir="$(make_temp_dir)"

  cat > "$workspace/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "loop-task-1",
      "title": "Loop task 1",
      "model": "gpt-5.1-codex-mini",
      "status": "unstarted",
      "definition_of_done": [
        "Iterate loop calls"
      ],
      "recommended": {
        "approach": "Exercise loop iteration behavior."
      }
    },
    {
      "task_id": "loop-task-2",
      "title": "Loop task 2",
      "model": "gpt-5.1-codex-mini",
      "status": "unstarted",
      "definition_of_done": [
        "Iterate loop calls"
      ],
      "recommended": {
        "approach": "Exercise loop iteration behavior."
      }
    }
  ]
}
JSON

  init_git_repo "$workspace"

  mkdir -p "$home_dir/.prompts"
  cat > "$home_dir/.prompts/autonomous-senior-engineer.prompt.md" <<'EOF2'
Test prompt
EOF2

  local log="$log_dir/loop-limit-${limit}.log"

  HOME="$home_dir" \
  PATH="$stub_dir:$PATH" \
    GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com \
    GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com \
    "$lever_bin" \
    --workspace "$workspace" \
    --tasks "$workspace/prd.json" \
    --loop "$limit" \
    > "$log" 2>&1

  local run_count
  run_count="$(find "$workspace/.ralph/runs" -mindepth 2 -maxdepth 2 -type d | wc -l | tr -d ' ')"
  if [[ "$run_count" -ne "$limit" ]]; then
    echo "expected $limit invocations, got $run_count" >&2
    exit 1
  fi

  if ! grep -q "lever: --loop limit reached (${limit})" "$log"; then
    echo "missing --loop limit reached log for limit ${limit}" >&2
    exit 1
  fi

  if grep -q "shutdown requested" "$log"; then
    echo "unexpected shutdown message in loop limit test" >&2
    exit 1
  fi

  rm -rf "$workspace" "$home_dir" "$log_dir"
}

run_continuous_case() {
  local name="$1"
  shift
  local loop_args=("$@")
  local workspace
  workspace="$(make_temp_dir)"
  local home_dir
  home_dir="$(make_temp_dir)"
  local log_dir
  log_dir="$(make_temp_dir)"

  cat > "$workspace/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "loop-task",
      "title": "Loop task",
      "model": "gpt-5.1-codex-mini",
      "status": "unstarted",
      "definition_of_done": [
        "Iterate loop calls"
      ],
      "recommended": {
        "approach": "Exercise loop iteration behavior."
      }
    }
  ]
}
JSON

  init_git_repo "$workspace"

  mkdir -p "$home_dir/.prompts"
  cat > "$home_dir/.prompts/autonomous-senior-engineer.prompt.md" <<'EOF2'
Test prompt
EOF2

  local log="$log_dir/continuous-${name}.log"
  local invocations="$log_dir/continuous-${name}.invocations"

  : > "$invocations"

  HOME="$home_dir" LOG_PATH="$invocations" SLEEP_DURATION=0.3 "$lever_bin" \
    --workspace "$workspace" \
    --tasks "$workspace/prd.json" \
    "${loop_args[@]}" \
    --command-path "$stub" \
    > "$log" 2>&1 &

  local lever_pid=$!

  local attempts=0
  until [[ -s "$invocations" ]]; do
    sleep 0.01
    attempts=$((attempts + 1))
    if [[ "$attempts" -gt 200 ]]; then
      kill "$lever_pid" 2>/dev/null || true
      wait "$lever_pid" 2>/dev/null || true
      echo "lever never registered an iteration for ${name}" >&2
      exit 1
    fi
  done

  kill -INT "$lever_pid"

  if ! wait "$lever_pid"; then
    echo "lever exited non-zero after Ctrl-C for ${name}" >&2
    exit 1
  fi

  mapfile -t recorded < "$invocations"
  if [[ "${#recorded[@]}" -lt 1 ]]; then
    echo "expected at least one iteration before Ctrl-C for ${name}" >&2
    exit 1
  fi

  if ! grep -q "shutdown requested during task-agent execution" "$log"; then
    echo "missing shutdown message after Ctrl-C for ${name}" >&2
    exit 1
  fi

  if ! grep -q "lever: loop mode active; deferring task selection" "$log"; then
    echo "missing loop mode message for ${name}" >&2
    exit 1
  fi

  if grep -q "lever: --loop limit reached" "$log"; then
    echo "unexpected limit log for continuous case ${name}" >&2
    exit 1
  fi

  rm -rf "$workspace" "$home_dir" "$log_dir"
}

run_loop_limit_test 2
run_continuous_case "flag-no-value" "--loop"
run_continuous_case "explicit-zero" "--loop" "0"
