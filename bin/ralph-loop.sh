#!/usr/bin/env bash
set -euo pipefail

# Stop with a message unless we are told to keep going.
die() {
  printf 'ralph-loop: %s\n' "$*" >&2
  exit 1
}

info() {
  printf '%s\n' "$*"
}

TRAIT_NAME="Ralph Wiggum"
WORKSPACE="${WORKSPACE:-$PWD}"
TASKS_FILE="${TASKS_FILE:-prd.json}"
PROMPT_FILE="${PROMPT_FILE:-prompts/autonomous-senior-engineer.prompt.md}"
ASSIGNEE="${ASSIGNEE:-ralph-loop}"
TASK_AGENT_BIN="${TASK_AGENT_BIN:-task-agent}"
DELAY_SECONDS=0

resolve_path() {
  local input="$1"
  local base="$2"
  if [[ "$input" == /* ]]; then
    printf '%s\n' "$input"
  else
    printf '%s\n' "$base/$input"
  fi
}

usage() {
  cat <<'USAGE'
Usage: ralph-loop [options]

Options:
  --tasks PATH        Tasks file (default: prd.json)
  --prompt PATH       Prompt file sent to task-agent (required)
  --assignee NAME     Assignee metadata (default: ralph-loop)
  --task-agent PATH   Path to task-agent driver (default: task-agent on PATH)
  --delay SECONDS     Pause between iterations (default: 0)
  --workspace PATH    Workspace directory (default: current directory)
  -h, --help          Show this help message
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tasks) TASKS_FILE="$2"; shift 2 ;;
    --prompt) PROMPT_FILE="$2"; shift 2 ;;
    --assignee) ASSIGNEE="$2"; shift 2 ;;
    --task-agent) TASK_AGENT_BIN="$2"; shift 2 ;;
    --delay) DELAY_SECONDS="$2"; shift 2 ;;
    --workspace) WORKSPACE="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "Unknown option $1";;
  esac
done

if [[ ! -d "$WORKSPACE" ]]; then
  die "Workspace not found: $WORKSPACE"
fi

WORKSPACE="$(cd "$WORKSPACE" && pwd)"
TASKS_FILE="$(resolve_path "$TASKS_FILE" "$WORKSPACE")"
PROMPT_FILE="$(resolve_path "$PROMPT_FILE" "$WORKSPACE")"

if [[ ! -f "$TASKS_FILE" ]]; then
  die "Tasks file not found: $TASKS_FILE"
fi

if [[ ! -f "$PROMPT_FILE" ]]; then
  die "Prompt file not found: $PROMPT_FILE"
fi

if [[ "$TASK_AGENT_BIN" == */* ]]; then
  TASK_AGENT_BIN="$(resolve_path "$TASK_AGENT_BIN" "$WORKSPACE")"
  if [[ ! -x "$TASK_AGENT_BIN" ]]; then
    die "Task agent driver is not executable: $TASK_AGENT_BIN"
  fi
else
  TASK_AGENT_CMD="$TASK_AGENT_BIN"
  TASK_AGENT_BIN="$(command -v "$TASK_AGENT_CMD" || true)"
  if [[ -z "$TASK_AGENT_BIN" ]]; then
    die "Task agent driver not found on PATH: $TASK_AGENT_CMD"
  fi
fi

if ! command -v jq >/dev/null 2>&1; then
  die "jq is required"
fi

TASK_SOURCE='if type=="object" and has("tasks") then .tasks else . end'

first_incomplete_task_fields() {
  jq -r '
    '"$TASK_SOURCE"'
    | map(select((.status // "unstarted") != "completed"))
    | .[0]
    | if . == null then "" else
        [.task_id // "", (.status // "unstarted"), (.model // "")] | @tsv
      end
  ' "$TASKS_FILE"
}

cycle=0
stop_reason=""
info "Starting the ${TRAIT_NAME} loop; tasks file=$TASKS_FILE"

while true; do
  task_fields="$(first_incomplete_task_fields)"
  if [[ -z "$task_fields" ]]; then
    info "No remaining tasks to drive."
    break
  fi

  IFS=$'\t' read -r task_id task_status task_model <<<"$task_fields"
  if [[ -z "$task_id" ]]; then
    die "Unable to determine the next task to run."
  fi

  if [[ "$task_model" == "human" ]]; then
    stop_reason="Next task ${task_id} requires human input."
    break
  fi

  if [[ "$task_status" == "blocked" ]]; then
    info "Resuming blocked task ${task_id}."
  fi

  cycle=$((cycle + 1))
  info "Cycle $cycle: running ${task_id} (status=${task_status})"

  set +e
  "$TASK_AGENT_BIN" --tasks "$TASKS_FILE" --task-id "$task_id" --assignee "$ASSIGNEE" --prompt "$PROMPT_FILE" --workspace "$WORKSPACE"
  exit_code=$?
  set -e

  if [[ $exit_code -eq 0 ]]; then
    :
  elif [[ $exit_code -eq 130 ]]; then
    die "Task agent interrupted by SIGINT while running ${task_id}."
  elif [[ $exit_code -eq 3 ]]; then
    info "Task agent reported no runnable tasks (code 3); stopping."
    break
  elif [[ $exit_code -eq 4 ]]; then
    stop_reason="Task ${task_id} requires human input."
    break
  elif [[ $exit_code -eq 5 || $exit_code -eq 6 ]]; then
    stop_reason="Task ${task_id} cannot start due to unmet dependencies."
    break
  elif [[ $exit_code -eq 10 || $exit_code -eq 11 ]]; then
    stop_reason="Task ${task_id} blocked; manual intervention required."
    break
  elif [[ $exit_code -lt 10 ]]; then
    die "Task agent failed (exit code $exit_code)"
  else
    info "Task agent ended with $exit_code (continuing)."
  fi

  if [[ "$DELAY_SECONDS" != "0" ]]; then
    sleep "$DELAY_SECONDS"
  fi
done

if [[ -n "$stop_reason" ]]; then
  die "$stop_reason"
fi

info "Ralph loop done after $cycle cycles."
