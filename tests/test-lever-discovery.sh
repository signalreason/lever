#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$TEST_DIR/helpers.sh"

require_cmd cargo
require_cmd true
require_cmd git

cleanup_dirs=()
cleanup() {
  for dir in "${cleanup_dirs[@]:-}"; do
    rm -rf "$dir"
  done
}
trap cleanup EXIT

register_dir() {
  cleanup_dirs+=("$1")
}

true_bin="$(command -v true)"

run_discovery_order_test() {
  local workspace
  workspace="$(make_temp_dir)"
  register_dir "$workspace"

  cat > "$workspace/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "discovery-prd-task",
      "title": "Discovery PRD task",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Exercise task discovery order"
      ],
      "recommended": {
        "approach": "Select the prd.json entry when available."
      }
    }
  ]
}
JSON

  cat > "$workspace/tasks.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "discovery-tasks-task",
      "title": "Discovery tasks.json task",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Exercise task discovery order"
      ],
      "recommended": {
        "approach": "Select the prd.json entry when available."
      }
    }
  ]
}
JSON

  init_git_repo "$workspace"

  local expected_tasks_path
  expected_tasks_path="$(cd "$workspace" && pwd -P)/prd.json"

  local output
  output=$(
    (
      cd "$workspace"
      cargo run --quiet --manifest-path "$TEST_DIR/../Cargo.toml" -- \
        --command-path "$true_bin"
    ) 2>&1
  )

  if ! grep -q "lever: tasks=${expected_tasks_path}" <<<"$output"; then
    echo "Expected lever to prefer prd.json when both candidates exist" >&2
    exit 1
  fi

  if ! grep -q "lever: selected task discovery-prd-task" <<<"$output"; then
    echo "Expected lever to select the PRD task when prd.json exists" >&2
    exit 1
  fi
}

run_missing_discovery_error() {
  local workspace
  workspace="$(make_temp_dir)"
  register_dir "$workspace"
  init_git_repo "$workspace"

  set +e
  local output
  output=$(
    (
      cd "$workspace"
      cargo run --quiet --manifest-path "$TEST_DIR/../Cargo.toml" -- \
        --command-path "$true_bin"
    ) 2>&1
  )
  local status=$?
  set -e

  if [[ $status -eq 0 ]]; then
    echo "Expected lever to fail when no candidate tasks file exists" >&2
    exit 1
  fi

  if ! grep -q "No tasks file specified and neither prd.json nor tasks.json exist in the current directory" <<<"$output"; then
    echo "Unexpected discovery failure message: $output" >&2
    exit 1
  fi
}

run_missing_explicit_file_error() {
  local workspace
  workspace="$(make_temp_dir)"
  register_dir "$workspace"
  init_git_repo "$workspace"

  set +e
  local output
  output=$(
    (
      cd "$workspace"
      cargo run --quiet --manifest-path "$TEST_DIR/../Cargo.toml" -- \
        --tasks missing.json \
        --command-path "$true_bin"
    ) 2>&1
  )
  local status=$?
  set -e

  if [[ $status -eq 0 ]]; then
    echo "Expected lever to fail when an explicit tasks file is missing" >&2
    exit 1
  fi

  if ! grep -q "The specified tasks file missing.json does not exist or is not a file" <<<"$output"; then
    echo "Unexpected explicit missing file message: $output" >&2
    exit 1
  fi
}

run_discovery_order_test
run_missing_discovery_error
run_missing_explicit_file_error
