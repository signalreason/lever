#!/usr/bin/env bash
set -euo pipefail

# task-agent: run exactly one task iteration via Codex CLI.
#
# Dependencies:
#   - codex (Codex CLI)
#   - jq
#   - git
#
# Typical usage:
#   bin/task-agent --tasks ./tasks.json --task-id TUI-010 --assignee ralph-01
#   bin/task-agent --tasks ./tasks.json --next --assignee ralph-01

TASKS_FILE=""
TASK_ID=""
ASSIGNEE="${ASSIGNEE:-task-agent}"
WORKSPACE="${WORKSPACE:-$PWD}"
NEXT=false
PROMPT_FILE="${PROMPT_FILE:-$HOME/.prompts/autonomous-senior-engineer.prompt.md}"
BASE_BRANCH="${BASE_BRANCH:-main}"
TASK_BRANCH=""
RUN_ATTEMPT=0
RATE_LIMIT_FILE=".ralph/rate_limit.json"
RATE_LIMIT_WINDOW_SECONDS=60
ORIG_BRANCH=""
ORIG_HEAD=""
PRE_RUN_HEAD=""
DIRTY_FILES_FILE=""
STASH_REF=""
STASH_MSG=""
RESET_TASK=false
MAX_RUN_ATTEMPTS=3
CODEX_STREAM_PID=""

usage() {
  cat <<'USAGE'
Usage:
  task-agent --tasks <path> (--task-id <id> | --next) [--assignee <name>] [--workspace <path>]

Options:
  --tasks      Path to tasks JSON file
  --task-id    Specific task_id to run
  --next       Select first task with status!=completed and model!=human
  --assignee   Name to store for observability
  --workspace  Repo directory to run in (default: current)
  --prompt     Prompt file to send to the LLM (default: ~/.prompts/autonomous-senior-engineer.prompt.md)
  --reset-task Reset attempt counter/status for the selected task before running
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tasks) TASKS_FILE="$2"; shift 2 ;;
    --task-id) TASK_ID="$2"; shift 2 ;;
    --next) NEXT=true; shift 1 ;;
    --assignee) ASSIGNEE="$2"; shift 2 ;;
    --workspace) WORKSPACE="$2"; shift 2 ;;
    --prompt) PROMPT_FILE="$2"; shift 2 ;;
    --reset-task) RESET_TASK=true; shift 1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown arg: $1" >&2; usage; exit 2 ;;
  esac
done

if [[ -z "$TASKS_FILE" ]]; then
  echo "--tasks is required" >&2
  exit 2
fi

if [[ "$NEXT" != true && -z "$TASK_ID" ]]; then
  echo "Specify --task-id or --next" >&2
  exit 2
fi

if [[ ! -d "$WORKSPACE" ]]; then
  echo "Workspace not found: $WORKSPACE" >&2
  exit 2
fi

WORKSPACE="$(cd "$WORKSPACE" && pwd)"

resolve_path() {
  local input="$1"
  local base="$2"
  if [[ "$input" == /* ]]; then
    printf '%s\n' "$input"
  else
    printf '%s\n' "$base/$input"
  fi
}

TASKS_FILE="$(resolve_path "$TASKS_FILE" "$WORKSPACE")"
PROMPT_FILE="$(resolve_path "$PROMPT_FILE" "$WORKSPACE")"

if [[ ! -f "$PROMPT_FILE" ]]; then
  echo "Prompt file not found: $PROMPT_FILE" >&2
  exit 2
fi

if [[ ! -f "$TASKS_FILE" ]]; then
  echo "Tasks file not found: $TASKS_FILE" >&2
  exit 2
fi

cd "$WORKSPACE"

if ! command -v jq >/dev/null 2>&1; then
  echo "Missing dependency: jq" >&2
  exit 2
fi

if ! command -v codex >/dev/null 2>&1; then
  echo "Missing dependency: codex (Codex CLI)" >&2
  exit 2
fi

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "Not a git repository: $WORKSPACE" >&2
  exit 2
fi

log_line() {
  local level="$1"
  shift
  local msg="$1"
  shift || true
  local kv="$*"
  msg="${msg//$'\n'/ }"
  local line
  line="$(printf '%s %s %s %s%s' "$(date -u +"%Y-%m-%dT%H:%M:%SZ")" "$level" "task-agent" "$msg" "${kv:+ $kv}")"
  if [[ "$level" == "ERROR" || "$level" == "WARN" ]]; then
    printf '%s\n' "$line" >&2
  else
    printf '%s\n' "$line"
  fi
}

root_selector='(if type=="object" and has("tasks") then .tasks else . end)'

first_incomplete_task_jq="
$root_selector
| map(select((.status // \"unstarted\") != \"completed\"))
| .[0]
"

checkout_task_branch() {
  if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "Not a git repository: $WORKSPACE" >&2
    exit 2
  fi

  git checkout "$BASE_BRANCH"

  if git show-ref --verify --quiet "refs/heads/$TASK_BRANCH"; then
    git checkout "$TASK_BRANCH"
  else
    git checkout -b "$TASK_BRANCH"
  fi
}

capture_repo_state() {
  ORIG_BRANCH="$(git rev-parse --abbrev-ref HEAD)"
  ORIG_HEAD="$(git rev-parse HEAD)"
  PRE_RUN_HEAD="$ORIG_HEAD"
}

record_dirty_files() {
  DIRTY_FILES_FILE="$(mktemp)"
  {
    git diff --name-only
    git diff --name-only --cached
    git ls-files --others --exclude-standard
  } | sort -u >"$DIRTY_FILES_FILE"
}

stash_local_changes() {
  if [[ -n "$(git status --porcelain)" ]]; then
    record_dirty_files
    STASH_MSG="ralph(task-agent): auto-stash $(date -u +"%Y%m%dT%H%M%SZ")-$$"
    git stash push -u -m "$STASH_MSG" >/dev/null
    STASH_REF="$(git stash list --format="%gd %gs" | awk -v msg="$STASH_MSG" '$0 ~ msg {print $1; exit}')"
    if [[ -z "$STASH_REF" ]]; then
      echo "Warning: auto-stash created but ref not found; check git stash list." >&2
    else
      echo "Stashed local changes as $STASH_REF." >&2
    fi
  fi
}

restore_local_changes() {
  if [[ -z "$STASH_REF" ]]; then
    return
  fi

  if [[ -z "$DIRTY_FILES_FILE" || ! -f "$DIRTY_FILES_FILE" ]]; then
    echo "Warning: missing dirty file list; leaving $STASH_REF for manual apply." >&2
    return
  fi

  local run_files_file
  run_files_file="$(mktemp)"
  if ! git diff --name-only "$PRE_RUN_HEAD" HEAD >"$run_files_file" 2>/dev/null; then
    echo "Warning: unable to compute run changes; leaving $STASH_REF for manual apply." >&2
    rm -f "$run_files_file"
    return
  fi

  if comm -12 <(sort "$DIRTY_FILES_FILE") <(sort "$run_files_file") | grep -q .; then
    echo "Warning: stash $STASH_REF overlaps run changes; apply manually." >&2
    rm -f "$run_files_file"
    return
  fi
  rm -f "$run_files_file"

  if [[ "$ORIG_BRANCH" == "HEAD" ]]; then
    if ! git checkout --detach "$ORIG_HEAD" >/dev/null 2>&1; then
      echo "Warning: unable to restore detached HEAD; leaving $STASH_REF." >&2
      return
    fi
  else
    if ! git checkout "$ORIG_BRANCH" >/dev/null 2>&1; then
      echo "Warning: unable to checkout $ORIG_BRANCH; leaving $STASH_REF." >&2
      return
    fi
  fi

  if git stash apply "$STASH_REF" >/dev/null 2>&1; then
    git stash drop "$STASH_REF" >/dev/null 2>&1
  else
    echo "Warning: stash $STASH_REF could not be applied cleanly; leaving stash for manual apply." >&2
  fi
}

cleanup() {
  stop_codex_log_stream
  restore_local_changes
  if [[ -n "$DIRTY_FILES_FILE" ]]; then
    rm -f "$DIRTY_FILES_FILE"
  fi
}

rate_limit_settings() {
  case "$MODEL" in
    gpt-5.1-codex-mini)
      echo "200000 500"
      ;;
    gpt-5.1-codex|gpt-5.2-codex)
      echo "500000 500"
      ;;
    *)
      echo "200000 500"
      ;;
  esac
}

estimate_prompt_tokens() {
  local prompt_path="$1"
  PROMPT_PATH="$prompt_path" python - <<'PY'
import os
import math
path = os.environ.get("PROMPT_PATH", "")
try:
    size = os.path.getsize(path)
except OSError:
    size = 0
estimate = max(1000, int(math.ceil(size / 4))) if size else 1000
print(estimate)
PY
}

rate_limit_sleep() {
  local estimated_tokens="$1"
  local tpm_limit rpm_limit
  read -r tpm_limit rpm_limit <<<"$(rate_limit_settings)"
  local sleep_seconds
  sleep_seconds="$(MODEL="$MODEL" RATE_LIMIT_FILE="$RATE_LIMIT_FILE" RATE_LIMIT_WINDOW_SECONDS="$RATE_LIMIT_WINDOW_SECONDS" TPM_LIMIT="$tpm_limit" RPM_LIMIT="$rpm_limit" EST_TOKENS="$estimated_tokens" python - <<'PY'
import json
import os
import time

rate_file = os.environ.get("RATE_LIMIT_FILE")
model = os.environ.get("MODEL")
window = int(os.environ.get("RATE_LIMIT_WINDOW_SECONDS", "60"))
tpm_limit = int(os.environ.get("TPM_LIMIT", "0"))
rpm_limit = int(os.environ.get("RPM_LIMIT", "0"))
estimated_tokens = int(os.environ.get("EST_TOKENS", "0"))
now = time.time()

requests = []
if rate_file and os.path.exists(rate_file):
    try:
        with open(rate_file, "r") as fh:
            payload = json.load(fh)
            requests = payload.get("requests", [])
    except Exception:
        requests = []

def is_recent(entry):
    try:
        return (now - float(entry.get("ts", 0))) < window
    except Exception:
        return False

recent = [r for r in requests if r.get("model") == model and is_recent(r)]
recent.sort(key=lambda r: float(r.get("ts", 0)))

sleep_for = 0.0
if rpm_limit > 0 and len(recent) >= rpm_limit:
    idx = len(recent) - rpm_limit
    expire_at = float(recent[idx].get("ts", 0)) + window
    sleep_for = max(sleep_for, expire_at - now)

if tpm_limit > 0:
    used = sum(int(r.get("tokens", 0)) for r in recent)
    if used + estimated_tokens > tpm_limit:
        over = used + estimated_tokens - tpm_limit
        dropped = 0
        for entry in recent:
            dropped += int(entry.get("tokens", 0))
            expire_at = float(entry.get("ts", 0)) + window
            if dropped >= over:
                sleep_for = max(sleep_for, expire_at - now)
                break

sleep_for = max(0.0, sleep_for)
print(int(sleep_for + 0.999))
PY
)"

  if [[ "$sleep_seconds" -gt 0 ]]; then
    echo "Rate limit throttle: sleeping ${sleep_seconds}s for ${MODEL}." >&2
    sleep "$sleep_seconds"
  fi
}

record_rate_usage() {
  local tokens_used="$1"
  MODEL="$MODEL" RATE_LIMIT_FILE="$RATE_LIMIT_FILE" RATE_LIMIT_WINDOW_SECONDS="$RATE_LIMIT_WINDOW_SECONDS" TOKENS_USED="$tokens_used" python - <<'PY'
import json
import os
import time

rate_file = os.environ.get("RATE_LIMIT_FILE")
model = os.environ.get("MODEL")
window = int(os.environ.get("RATE_LIMIT_WINDOW_SECONDS", "60"))
tokens = int(os.environ.get("TOKENS_USED", "0"))
now = time.time()

payload = {"requests": []}
if rate_file and os.path.exists(rate_file):
    try:
        with open(rate_file, "r") as fh:
            payload = json.load(fh)
    except Exception:
        payload = {"requests": []}

requests = payload.get("requests", [])
def is_recent(entry):
    try:
        return (now - float(entry.get("ts", 0))) < window
    except Exception:
        return False

requests = [r for r in requests if is_recent(r)]
requests.append({"ts": now, "model": model, "tokens": tokens})

payload["requests"] = requests
os.makedirs(os.path.dirname(rate_file), exist_ok=True)
with open(rate_file, "w") as fh:
    json.dump(payload, fh)
PY
}

parse_usage_tokens() {
  CODEX_LOG="$CODEX_LOG" python - <<'PY'
import json
import os

log_path = os.environ.get("CODEX_LOG", "")
if not log_path or not os.path.exists(log_path):
    print("")
    raise SystemExit

usage = None
with open(log_path, "r") as fh:
    for line in fh:
        line = line.strip()
        if not line or not line.startswith("{"):
            continue
        try:
            payload = json.loads(line)
        except json.JSONDecodeError:
            continue
        if payload.get("type") == "turn.completed":
            usage = payload.get("usage")

if not usage:
    print("")
    raise SystemExit

input_tokens = int(usage.get("input_tokens", 0))
output_tokens = int(usage.get("output_tokens", 0))
print(input_tokens + output_tokens)
PY
}

rate_limit_retry_delay() {
  CODEX_LOG="$CODEX_LOG" python - <<'PY'
import os
import re

log_path = os.environ.get("CODEX_LOG", "")
if not log_path or not os.path.exists(log_path):
    print("")
    raise SystemExit

pattern = re.compile(r"Please try again in ([0-9.]+)s", re.IGNORECASE)
with open(log_path, "r") as fh:
    for line in fh:
        if "rate limit" in line.lower() or "rate-limit" in line.lower():
            match = pattern.search(line)
            if match:
                print(match.group(1))
                raise SystemExit
print("")
PY
}

start_codex_log_stream() {
  if [[ -z "${CODEX_LOG:-}" ]]; then
    return
  fi
  : >"$CODEX_LOG"
  TASK_ID="$TASK_ID" RUN_ID="$RUN_ID" CODEX_LOG="$CODEX_LOG" python -u - <<'PY' &
import json
import os
import sys
import time
from datetime import datetime, timezone

log_path = os.environ.get("CODEX_LOG")
task_id = os.environ.get("TASK_ID", "")
run_id = os.environ.get("RUN_ID", "")

def iso_ts(value):
    if value is None:
        return None
    if isinstance(value, (int, float)):
        ts = float(value)
        if ts > 1e12:
            ts = ts / 1000.0
        return datetime.fromtimestamp(ts, tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    if isinstance(value, str):
        return value
    return None

def pick_ts(entry):
    for key in ("ts", "timestamp", "time", "created_at", "created"):
        if key in entry:
            value = iso_ts(entry.get(key))
            if value:
                return value
    return datetime.now(tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

def extract_message(entry):
    msg = entry.get("message")
    if isinstance(msg, str):
        return msg
    if isinstance(msg, dict):
        for key in ("content", "text", "message"):
            value = msg.get(key)
            if isinstance(value, str):
                return value
            if isinstance(value, list):
                parts = []
                for item in value:
                    if isinstance(item, dict):
                        text = item.get("text") or item.get("content")
                        if isinstance(text, str):
                            parts.append(text)
                    elif isinstance(item, str):
                        parts.append(item)
                if parts:
                    return " ".join(parts)
    if isinstance(msg, list):
        return " ".join([m for m in msg if isinstance(m, str)])
    return None

def compact(value, limit=400):
    if value is None:
        return ""
    value = value.replace("\n", " ").replace("\r", " ").strip()
    if len(value) > limit:
        return value[:limit] + "..."
    return value

if not log_path:
    raise SystemExit

with open(log_path, "r") as fh:
    while True:
        try:
            size = os.path.getsize(log_path)
        except OSError:
            size = None
        if size is not None and fh.tell() > size:
            fh.seek(0)
        line = fh.readline()
        if not line:
            time.sleep(0.1)
            continue
        line = line.strip()
        if not line:
            continue
        if line.startswith("{"):
            try:
                entry = json.loads(line)
            except json.JSONDecodeError:
                entry = None
            if entry is not None:
                level = "INFO"
                if entry.get("level") == "error" or entry.get("type") == "error" or entry.get("error"):
                    level = "ERROR"
                event_type = entry.get("type") or entry.get("event") or entry.get("level") or "event"
                detail = extract_message(entry) or entry.get("detail") or entry.get("error") or ""
                detail = compact(str(detail)) if detail else ""
                ts = pick_ts(entry)
                suffix = f" task_id={task_id} run_id={run_id}"
                if detail:
                    print(f"{ts} {level} codex {event_type} {detail}{suffix}", flush=True)
                else:
                    print(f"{ts} {level} codex {event_type}{suffix}", flush=True)
                continue
        ts = datetime.now(tz=timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        raw = compact(line)
        print(f"{ts} INFO codex raw {raw} task_id={task_id} run_id={run_id}", flush=True)
PY
  CODEX_STREAM_PID=$!
}

stop_codex_log_stream() {
  if [[ -n "${CODEX_STREAM_PID:-}" ]]; then
    kill "$CODEX_STREAM_PID" >/dev/null 2>&1 || true
    wait "$CODEX_STREAM_PID" >/dev/null 2>&1 || true
  fi
}

increment_attempt_count() {
  local tmp file
  tmp="$(mktemp)"

  jq --arg id "$TASK_ID" '
    def inc:
      .observability.run_attempts = ((.observability.run_attempts // 0) + 1);
    if type=="object" and has("tasks") then
      .tasks = (.tasks | map(if .task_id == $id then (inc) else . end))
    else
      map(if .task_id == $id then (inc) else . end)
    end
  ' "$TASKS_FILE" >"$tmp" && mv "$tmp" "$TASKS_FILE"

  jq -r --arg id "$TASK_ID" '
    '"$root_selector"'
    | map(select(.task_id == $id))
    | .[0]
    | .observability.run_attempts // 0
  ' "$TASKS_FILE"
}

current_attempt_count() {
  jq -r --arg id "$TASK_ID" '
    '"$root_selector"'
    | map(select(.task_id == $id))
    | .[0]
    | .observability.run_attempts // 0
  ' "$TASKS_FILE"
}

reset_task_attempts() {
  local note="${1:-}"
  local tmp
  tmp="$(mktemp)"

  jq \
    --arg id "$TASK_ID" \
    --arg st "unstarted" \
    --arg asg "$ASSIGNEE" \
    --arg rid "$RUN_ID" \
    --arg note "$note" \
    '
    def reset:
      .status = $st
      | .assignee = $asg
      | .observability.run_attempts = 0
      | .observability.last_run_id = $rid
      | .observability.last_update_utc = (now | todateiso8601)
      | (if ($note | length) > 0
          then .observability.last_note = $note
          else .
        end);

    if type=="object" and has("tasks") then
      .tasks = (.tasks | map(if .task_id == $id then (reset) else . end))
    else
      map(if .task_id == $id then (reset) else . end)
    end
  ' "$TASKS_FILE" >"$tmp" && mv "$tmp" "$TASKS_FILE"
}

update_task_status() {
  local new_status="$1"
  local note="${2:-}"
  local tmp
  tmp="$(mktemp)"

  jq \
    --arg id "$TASK_ID" \
    --arg st "$new_status" \
    --arg asg "$ASSIGNEE" \
    --arg rid "$RUN_ID" \
    --arg note "$note" \
    '
    def upd:
      .status = $st
      | .assignee = $asg
      | .observability.last_run_id = $rid
      | .observability.last_update_utc = (now | todateiso8601)
      | (if ($note | length) > 0
          then .observability.last_note = $note
          else .
        end);

    if type=="object" and has("tasks") then
      .tasks = (.tasks | map(if .task_id == $id then (upd) else . end))
    else
      map(if .task_id == $id then (upd) else . end)
    end
    ' "$TASKS_FILE" >"$tmp" && mv "$tmp" "$TASKS_FILE"
}

git_commit_progress() {
  local message="${1:-}"
  if [[ -z "$message" ]]; then
    message="ralph(${TASK_ID}): progress run ${RUN_ID}"
  fi

  if [[ -z "$(git status --porcelain)" ]]; then
    return
  fi

  git add -A
  git commit -m "$message" >/dev/null
}

finalize_successful_task() {
  local msg="ralph(${TASK_ID}): complete (run ${RUN_ID})"

  git checkout "$TASK_BRANCH"
  git rebase "$BASE_BRANCH" >/dev/null 2>&1 || true
  git reset --soft "$BASE_BRANCH"
  git add -A
  git commit -m "$msg" >/dev/null
  git checkout "$BASE_BRANCH"
  git merge --ff-only "$TASK_BRANCH" >/dev/null 2>&1
  git branch -D "$TASK_BRANCH" >/dev/null 2>&1
}

handle_interrupt() {
  trap - INT TERM
  local note="Run $RUN_ID interrupted on attempt ${RUN_ATTEMPT:-?}"
  log_line "WARN" "Run interrupted" "task_id=${TASK_ID}" "run_id=${RUN_ID}" "attempt=${RUN_ATTEMPT:-?}"
  update_task_status "started" "$note"
  git_commit_progress "ralph(${TASK_ID}): run ${RUN_ID} interrupted"
  exit 130
}

first_incomplete_task="$(jq -c "$first_incomplete_task_jq" "$TASKS_FILE")"

if [[ "$first_incomplete_task" == "null" || -z "$first_incomplete_task" ]]; then
  echo "No runnable task found" >&2
  exit 3
fi

first_task_id="$(jq -r '.task_id // ""' <<<"$first_incomplete_task")"
first_task_status="$(jq -r '.status // "unstarted"' <<<"$first_incomplete_task")"
first_task_model="$(jq -r '.model // ""' <<<"$first_incomplete_task")"

if [[ -z "$first_task_id" ]]; then
  echo "No runnable task found" >&2
  exit 3
fi

if [[ "$first_task_model" == "human" ]]; then
  echo "Task requires human: $first_task_id" >&2
  exit 4
fi

if [[ "$NEXT" == true ]]; then
  task_json="$first_incomplete_task"
else
  if [[ "$TASK_ID" != "$first_task_id" ]]; then
    echo "Task $TASK_ID cannot start until $first_task_id is completed." >&2
    exit 6
  fi
  task_json="$first_incomplete_task"
fi

TASK_ID="$(jq -r '.task_id' <<<"$task_json")"
MODEL="$(jq -r '.model' <<<"$task_json")"
STATUS="$(jq -r '.status' <<<"$task_json")"

if [[ "$MODEL" == "human" ]]; then
  echo "Task requires human: $TASK_ID" >&2
  exit 4
fi

case "$MODEL" in
  gpt-5.1-codex-mini|gpt-5.1-codex|gpt-5.2-codex) ;;
  *)
    echo "Unsupported model in task $TASK_ID: $MODEL" >&2
    exit 2
    ;;
esac

log_line "INFO" "Task selected" "task_id=${TASK_ID}" "model=${MODEL}" "status=${STATUS}"

capture_repo_state
stash_local_changes
trap cleanup EXIT

RUN_ID="$(date -u +"%Y%m%dT%H%M%SZ")-$$"
TASK_BRANCH="ralph/${TASK_ID}"
checkout_task_branch

log_line "INFO" "Run started" "task_id=${TASK_ID}" "run_id=${RUN_ID}" "branch=${TASK_BRANCH}"

if [[ "$RESET_TASK" == true ]]; then
  reset_task_attempts "Reset attempts via --reset-task"
fi

current_attempts="$(current_attempt_count)"
if [[ "$current_attempts" -ge "$MAX_RUN_ATTEMPTS" ]]; then
  update_task_status "blocked" "Attempt limit reached (${current_attempts}/${MAX_RUN_ATTEMPTS}). Use --reset-task after human intervention."
  git_commit_progress "ralph(${TASK_ID}): run ${RUN_ID} blocked (attempt limit)"
  log_line "WARN" "Attempt limit reached" "task_id=${TASK_ID}" "run_id=${RUN_ID}" "attempts=${current_attempts}"
  echo "Blocked: ${TASK_ID} reached attempt limit (${current_attempts}/${MAX_RUN_ATTEMPTS})." >&2
  exit 11
fi

RUN_ATTEMPT="$(increment_attempt_count)"
trap 'handle_interrupt' INT TERM
RUN_DIR=".ralph/runs/${TASK_ID}/${RUN_ID}"
mkdir -p "$RUN_DIR"

# Write the task snapshot used for this run
printf '%s\n' "$task_json" > "${RUN_DIR}/task.json"

# Ensure schema exists (used to make Codex output machine-parseable)
SCHEMA_PATH=".ralph/task_result.schema.json"
if [[ ! -f "$SCHEMA_PATH" ]]; then
  mkdir -p ".ralph"
  cat >"$SCHEMA_PATH" <<'JSON'
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "required": ["task_id", "outcome", "dod_met", "summary", "tests", "notes", "blockers"],
  "properties": {
    "task_id": { "type": "string" },
    "outcome": { "type": "string", "enum": ["completed", "blocked", "started"] },
    "dod_met": { "type": "boolean" },
    "summary": { "type": "string" },
    "tests": {
      "type": "object",
      "additionalProperties": false,
      "required": ["ran", "commands", "passed"],
      "properties": {
        "ran": { "type": "boolean" },
        "commands": { "type": "array", "items": { "type": "string" } },
        "passed": { "type": "boolean" }
      }
    },
    "notes": { "type": "string" },
    "blockers": { "type": "array", "items": { "type": "string" } }
  }
}
JSON
fi

if [[ "$STATUS" == "unstarted" || "$STATUS" == "blocked" ]]; then
  update_task_status "started" "Run $RUN_ID started (attempt $RUN_ATTEMPT)"
fi

# Build the per-run prompt for Codex
PROMPT_PATH="${RUN_DIR}/prompt.md"
cat "$PROMPT_FILE" >"$PROMPT_PATH"
printf '\n\nTask JSON (authoritative):\n%s\n' "$(cat "${RUN_DIR}/task.json")" >>"$PROMPT_PATH"

RESULT_PATH="${RUN_DIR}/result.json"
CODEX_LOG="${RUN_DIR}/codex.jsonl"
EST_TOKENS="$(estimate_prompt_tokens "$PROMPT_PATH")"

# Run Codex non-interactively.
# Note: global flags should come after the subcommand.
set +e
start_codex_log_stream
rate_limit_sleep "$EST_TOKENS"
CODEX_EXIT=1
rate_limit_retry=false
for attempt in 1 2 3; do
  rate_limit_retry=false
  log_line "INFO" "Codex exec start" "task_id=${TASK_ID}" "run_id=${RUN_ID}" "attempt=${attempt}" "model=${MODEL}"
  if [[ -n "${CODEX_API_KEY:-}" ]]; then
    env CODEX_API_KEY="${CODEX_API_KEY}" codex exec \
      --yolo \
      --model "$MODEL" \
      --output-schema "$SCHEMA_PATH" \
      --output-last-message "$RESULT_PATH" \
      --json \
      --skip-git-repo-check \
      - <"$PROMPT_PATH" >"$CODEX_LOG" 2>&1
  else
    codex exec \
      --yolo \
      --model "$MODEL" \
      --output-schema "$SCHEMA_PATH" \
      --output-last-message "$RESULT_PATH" \
      --json \
      --skip-git-repo-check \
      - <"$PROMPT_PATH" >"$CODEX_LOG" 2>&1
  fi
  CODEX_EXIT=$?
  log_line "INFO" "Codex exec end" "task_id=${TASK_ID}" "run_id=${RUN_ID}" "attempt=${attempt}" "exit=${CODEX_EXIT}"
  if [[ -s "$RESULT_PATH" ]]; then
    break
  fi
  retry_delay="$(rate_limit_retry_delay)"
  if [[ -n "$retry_delay" ]]; then
    rate_limit_retry=true
    sleep_seconds="$(python - <<PY
import math
print(int(math.ceil(float("${retry_delay}"))) if "${retry_delay}" else 0)
PY
)"
    if [[ "$sleep_seconds" -gt 0 ]]; then
      echo "Rate limit retry: sleeping ${sleep_seconds}s before retry ${attempt}/3." >&2
      sleep "$sleep_seconds"
    fi
  else
    break
  fi
done
set -e
stop_codex_log_stream

TOKENS_USED="$(parse_usage_tokens)"
if [[ -n "$TOKENS_USED" ]]; then
  TOKENS_USED="$TOKENS_USED" record_rate_usage "$TOKENS_USED"
else
  TOKENS_USED="$EST_TOKENS" record_rate_usage "$EST_TOKENS"
fi

# If Codex did not produce a final message, mark blocked and exit
if [[ ! -s "$RESULT_PATH" ]]; then
  update_task_status "blocked" "Codex produced no result.json (exit=$CODEX_EXIT). See ${CODEX_LOG}"
  git_commit_progress "ralph(${TASK_ID}): run ${RUN_ID} blocked (no result)"
  log_line "ERROR" "Missing result.json" "task_id=${TASK_ID}" "run_id=${RUN_ID}" "exit=${CODEX_EXIT}"
  echo "Blocked: missing result.json. See ${CODEX_LOG}" >&2
  exit 10
fi

OUTCOME="$(jq -r '.outcome' "$RESULT_PATH")"
DOD_MET="$(jq -r '.dod_met' "$RESULT_PATH")"

has_python_tests() {
  if [[ -f "pytest.ini" || -f "pyproject.toml" || -f "setup.cfg" || -f "tox.ini" ]]; then
    return 0
  fi
  if [[ -d "tests" ]]; then
    if command -v rg >/dev/null 2>&1; then
      rg --files -g '*.py' tests >/dev/null 2>&1 && return 0
    else
      find tests -type f -name '*.py' -print -quit | grep -q .
    fi
  fi
  return 1
}

# Optional: lightweight verification pass if repo has a standard script
VERIFY_OK=true
VERIFY_LOG="${RUN_DIR}/verify.log"
: >"$VERIFY_LOG"

if [[ "$OUTCOME" == "completed" && "$DOD_MET" == "true" ]]; then
  if [[ -x "./scripts/ci.sh" ]]; then
    ./scripts/ci.sh >"$VERIFY_LOG" 2>&1 || VERIFY_OK=false
  elif [[ -f "Makefile" ]] && grep -qE '^[[:space:]]*ci:' Makefile; then
    make ci >"$VERIFY_LOG" 2>&1 || VERIFY_OK=false
  elif [[ -x "./tests/run.sh" ]]; then
    ./tests/run.sh >"$VERIFY_LOG" 2>&1 || VERIFY_OK=false
  elif command -v pytest >/dev/null 2>&1 && has_python_tests; then
    pytest -q >"$VERIFY_LOG" 2>&1 || VERIFY_OK=false
  fi
fi

if [[ "$OUTCOME" == "completed" && "$DOD_MET" == "true" && "$VERIFY_OK" == "true" ]]; then
  update_task_status "completed" "Run $RUN_ID completed"
  git_commit_progress "ralph(${TASK_ID}): run ${RUN_ID} completed"
  finalize_successful_task
  log_line "INFO" "Run completed" "task_id=${TASK_ID}" "run_id=${RUN_ID}" "verify_ok=${VERIFY_OK}"
  echo "COMPLETED ${TASK_ID} (model=${MODEL}, run=${RUN_ID})"
  exit 0
fi

if [[ "$OUTCOME" == "blocked" ]]; then
  update_task_status "blocked" "Run $RUN_ID blocked. See ${RESULT_PATH}"
  git_commit_progress "ralph(${TASK_ID}): run ${RUN_ID} blocked"
  log_line "WARN" "Run blocked" "task_id=${TASK_ID}" "run_id=${RUN_ID}"
  echo "BLOCKED ${TASK_ID} (model=${MODEL}, run=${RUN_ID})" >&2
  exit 11
fi

# started or completed-but-failed-verification
note="Run $RUN_ID progress. outcome=${OUTCOME} dod_met=${DOD_MET} verify_ok=${VERIFY_OK}. See ${RESULT_PATH}"
update_task_status "started" "$note"
git_commit_progress "ralph(${TASK_ID}): run ${RUN_ID} progress"
log_line "INFO" "Run started/progress" "task_id=${TASK_ID}" "run_id=${RUN_ID}" "outcome=${OUTCOME}" "dod_met=${DOD_MET}" "verify_ok=${VERIFY_OK}"
echo "STARTED ${TASK_ID} (model=${MODEL}, run=${RUN_ID})" >&2
exit 12
