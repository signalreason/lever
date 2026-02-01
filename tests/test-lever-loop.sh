#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$TEST_DIR/helpers.sh"

require_cmd cargo

cargo build --quiet

lever_bin="$PWD/target/debug/lever"
if [[ ! -x "$lever_bin" ]]; then
  echo "lever binary missing after cargo build" >&2
  exit 1
fi

workspace="$(make_temp_dir)"
stub_dir="$(make_temp_dir)"
trap 'rm -rf "$workspace" "$stub_dir"' EXIT

cat > "$workspace/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "loop-task",
      "model": "gpt-5.1-codex-mini",
      "status": "unstarted"
    }
  ]
}
JSON

stub="$stub_dir/loop-stub"
cat > "$stub" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${LOG_PATH:-}" ]]; then
  echo "LOG_PATH is not set" >&2
  exit 1
fi

printf '%s\n' "$(date +%s%N)" >> "$LOG_PATH"
sleep "${SLEEP_DURATION:-0.2}"
EOF
chmod +x "$stub"

run_loop_limit_test() {
  local limit="$1"
  local log="$workspace/loop-limit-${limit}.log"
  local invocations="$workspace/loop-limit-${limit}.invocations"

  : > "$invocations"

  LOG_PATH="$invocations" SLEEP_DURATION=0.01 "$lever_bin" \
    --tasks "$workspace/prd.json" \
    --loop "$limit" \
    --command-path "$stub" \
    > "$log" 2>&1

  mapfile -t recorded < "$invocations"
  if [[ "${#recorded[@]}" -ne "$limit" ]]; then
    echo "expected $limit invocations, got ${#recorded[@]}" >&2
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
}

run_continuous_case() {
  local name="$1"
  shift
  local loop_args=("$@")
  local log="$workspace/continuous-${name}.log"
  local invocations="$workspace/continuous-${name}.invocations"

  : > "$invocations"

  LOG_PATH="$invocations" SLEEP_DURATION=0.3 "$lever_bin" \
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
}

run_loop_limit_test 2
run_continuous_case "flag-no-value" "--loop"
run_continuous_case "explicit-zero" "--loop" "0"
