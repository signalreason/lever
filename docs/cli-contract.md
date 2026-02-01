# Lever CLI contract

This spec documents the behavior that the upcoming Rust CLI must match so it can replace `bin/ralph-loop.sh` and `bin/task-agent.sh` without breaking the established workflow.  The CLI exposes two complementary entrypoints: `lever loop`, which maps directly to the behavior of the existing Ralph loop, and `lever task`, which mirrors the task-agent driver.  The focus is on flag mappings, discovery defaults, task selection rules, loop semantics (including `--loop 0`), and the hand-off between the loop runner and the task agent.

## Defaults and discovery order

- **Tasks file lookup:** when `--tasks` is not provided, probe the workspace for `prd.json` first and fall back to `tasks.json` if present.  Abort with a clear error if neither exists.  All paths are resolved relative to the workspace before the file is read, mirroring the `TASKS_FILE` checks in the bash scripts.
- **Tasks file lookup:** when `--tasks` is not provided, both commands probe the workspace for `prd.json` before falling back to `tasks.json`.  Abort with a clear error if neither exists.  All paths are resolved relative to the workspace before the file is read, mirroring the `TASKS_FILE` checks in the bash scripts.
- **Prompt file:** default to `$HOME/.prompts/autonomous-senior-engineer.prompt.md`.  `--prompt` overrides this path, and the CLI must validate that the prompt file is present before starting a run.
- **Workspace:** defaults to the current working directory.  `--workspace` changes the directory, and every other path (`--tasks`, `--prompt`, the task-agent binary) is resolved relative to the workspace.
- **Assignee log label:** the loop defaults to `ralph-loop` and the single task runner defaults to `task-agent`.  The value is only used for log metadata and is never written back to the task file.
- **Task agent binary:** `ralph-loop` currently resolves `TASK_AGENT_BIN` the same way as the `--task-agent` flag: if the argument contains a slash it is treated as a workspace-relative path that must be executable; otherwise the CLI looks the command up on `PATH`.

## `lever loop`

This command is a direct replacement for `bin/ralph-loop.sh`.  Each cycle:

1. reads the tasks file (array or `{ "tasks": ... }`) via `jq` and selects the first entry whose `status` is not `completed` (missing `status` counts as `unstarted`).
2. rejects runs whose `model` is `"human"` and stops with the matching stop reason.
3. invokes the task agent with the task id, workspace, prompt, and optional assignee.
4. respects the task agent exit code to determine whether to continue, stop (with a reason), or fail.

Because `lever loop` replaces the Ralph loop script, it should forward the same metadata to downstream runs: `--tasks`, `--prompt`, `--workspace`, and `--assignee` must pass through to `lever task` (or whichever task-agent binary `--task-agent` points at), mirroring how `ralph-loop.sh` invoked `bin/task-agent.sh`.  `--task-agent` itself selects the executable run per cycle; when omitted, the default should resolve to the bundled `lever task` binary (or whatever is on `PATH` if a slash-less name is provided).

Flags:

| Flag | Behavior | Pass-through |
| --- | --- | --- |
| `--tasks <path>` | overrides the default tasks file (same as `TASKS_FILE` in the script). | resolved before handing to `task-agent --tasks`. |
| `--prompt <path>` | overrides the prompt file used for both loop logging and the task agent prompt. | forwarded via `--prompt` to `task-agent`. |
| `--assignee <name>` | overrides `ASSIGNEE` and is forwarded to `task-agent --assignee`. | yes. |
| `--task-agent <path>` | identifies which binary to run for a task invocation. | used only for the loop’s executor. |
| `--delay <seconds>` | sleeps between cycles (default `0`). | n/a |
| `--workspace <path>` | changes the workspace directory. | also passed to `task-agent --workspace`. |
| `--loop <count>` | limit for task agent invocations; default `0`. | n/a |

`--loop` semantics: `0` (the default) lets the loop behave as today—keep cycling until a stop reason occurs (i.e., continuous mode).  Passing `--loop` with no numeric value also triggers continuous mode, so `lever loop --loop` and `lever loop --loop 0` behave identically.  Any positive integer caps the number of task-agent invocations, counting each cycle regardless of exit code, and the loop should log when the limit is reached before exiting even if runnable tasks remain.  Use `--delay` between cycles, but do not sleep after a terminal stop reason.

The loop should interpret task agent exit codes just as the bash script does:

- `0`: continue to the next cycle (unless `--loop` limit reached).
- `3`: “no runnable task”; log and exit cleanly.
- `4`: “task requires human”; stop and surface the message.
- `5`/`6`: “dependencies missing / cannot start”; record the stop reason and exit.
- `10`/`11`: “blocked (no result or attempt limit)”; stop with the recorded reason.
- `130`: propagate as an interruption (SIGINT), failing the loop.
- `<10` (other): treat as a hard failure and exit reports.
- `>11`: log the exit code but keep looping (these typically indicate `started`/`progress` states or other benign states).

If the task agent exits `0` and the loop still has cycles available (per `--loop` and `--tasks` content), the loop waits `--delay` seconds and restarts.

## `lever task`

This command mirrors `bin/task-agent.sh`'s single iteration.  Flags are:

| Flag | Behavior | Notes |
| --- | --- | --- |
| `--tasks <path>` | selects the tasks JSON file (same discovery order as the loop). | resolved relative to the workspace. |
| `--task-id <id>` | run the exact `task_id`, but only if it matches the first runnable task. | requires the task to be first in order; otherwise exit code `6`. |
| `--next` | pick the first task whose `status != completed` and whose `model != human` (same search as the loop). | cannot be combined with `--task-id`. |
| `--assignee <name>` | logging label; not written to task metadata. | optional, defaults to `task-agent`. |
| `--workspace <path>` | ensures `git` commands and file paths run from this directory. | identical to the loop’s workspace. |
| `--prompt <path>` | overrides the prompt file for the Codex run. | also used when building the per-run prompt. |
| `--reset-task` | before running, reset the selected task’s status to `unstarted`, zero `observability.run_attempts`, and stamp `observability.last_run_id`. | helpful when re-running blocked tasks after manual fixes. |

Before running Codex, the task agent must ensure:

- `jq`, `codex`, and `git` are installed and the workspace is a git repo.
- The chosen task exposes `title`, a non-empty `definition_of_done[]`, and `recommended.approach`.  Missing metadata raises an error.
- The `model` is one of `gpt-5.1-codex-mini`, `gpt-5.1-codex`, or `gpt-5.2-codex`.  `human` tasks exit with code `4`.

## Task selection rules

- Both commands interpret the tasks file as either a bare array or `{ "tasks": [...] }` (the `root_selector` in the scripts).  Use `jq`'s ordering to maintain deterministic selection.
- “Runnable” means `status != completed` and `model != human`.  The loop and the task agent pick the first runnable task in file order.
- `--task-id` can target a later task only if every earlier task has `status == completed`; otherwise the agent exits with code `6` and explains which task is blocking progress.
- When the first runnable task has `model == "human"`, the agent exits `4` (hooked by the loop to stop).  The loop surfaces “human input required” as the stop reason.
- Any exit code ≥`10` signals a blocked/failed state (`10` for no output, `11` for hitting `MAX_RUN_ATTEMPTS`, `12` for partial progress).  The loop stops on `10`/`11` with an explanatory reason and treats `12` as a benign status (it keeps looping if cycles remain).

## Task agent run behavior

- Create `.ralph/runs/<task_id>/<run_id>` and write the snapshot (`task.json`), prompt (`prompt.md`), and codex log (`codex.jsonl`).  The prompt includes the base prompt file, the task title, every DoD bullet, the recommended approach, and the authoritative JSON.
- Maintain a rate-limit cache under `.ralph/rate_limit.json` using the routine from the script (same TPM/RPM defaults per model).
- Run `codex exec --yolo --model <model> --output-schema .ralph/task_result.schema.json --output-last-message <result> --json --skip-git-repo-check`, streaming logs to `<run>/codex.jsonl` and collecting tokens for rate tracking.
- Interpret the `result.json` schema (`outcome`, `dod_met`, `tests`, `notes`, `blockers`).  If the file is missing, exit `10` and mark the task `blocked`.
- After Codex finishes successfully, optionally run verification (in order): `./scripts/ci.sh`, `make ci`, `./tests/run.sh`, `pytest -q` (only if Python tests exist).  Only run the first script that exists/executable.  Log success/failure and include the command + log path with `log_line` for visibility.
- Update the task status: on a successful `completed` outcome with `dod_met == true` and verification passing, set `status = completed`.  On `blocked` outcomes or attempt limits (≥`MAX_RUN_ATTEMPTS` = 3) set `status = blocked`.  Otherwise, keep it `started`.  Always stamp `observability` with `last_run_id`, `last_update_utc`, and, when appropriate, `last_note`.
- Create a feature branch `ralph/<task_id>`, commit the run’s changes, and merge them back into `main` with a fast-forward if the run completes.  Teardown ensures the workspace returns to the original branch and any auto-stashed changes are restored.

## Pass-through and integration expectations

- `lever loop` must pass `--tasks`, `--prompt`, `--workspace`, and `--assignee` through to `lever task` exactly as the bash script does.  `--task-agent` controls which binary is executed, and the Rust loop should allow swapping in a local `lever task` implementation for testing.
- The loop also passes `--prompt` implicitly by copying the prompt file into the per-run prompt built by the task agent, so the values must stay synchronized.
- The `--loop` count applies only at the loop level; the task agent remains unaware of it.

Use this contract to drive both implementation and regression tests.  Whenever the Rust implementation diverges (e.g., added flags, new defaults), update this document and align the bash scripts/tests accordingly.
