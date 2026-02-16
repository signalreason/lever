# Lever CLI contract

This spec documents the behavior of the Rust `lever` CLI. The CLI supports a single entrypoint with two modes:

- Single-iteration mode (default): runs one task-agent iteration.
- Loop mode (`--loop`): repeats task-agent iterations until a stop reason or iteration limit is reached.

The focus is on flag mappings, discovery defaults, task selection rules, loop semantics (including `--loop 0`), and the hand-off between the loop runner and the task agent (internal or external).

## Defaults and discovery order

- **Tasks file lookup:** when `--tasks` is not provided, probe the workspace for `prd.json` first and fall back to `tasks.json` if present. Abort with a clear error if neither exists. All paths are resolved relative to the workspace before the file is read.
- **Prompt file:** default to `prompts/autonomous-senior-engineer.prompt.md` under the workspace. `--prompt` overrides this path, and the CLI must validate that the prompt file is present before starting a run.
- **Workspace:** defaults to the current working directory. `--workspace` changes the directory, and every other path (`--tasks`, `--prompt`, the task-agent binary) is resolved relative to the workspace.
- **Assignee log label:** `ASSIGNEE` is read by the internal task agent for log metadata and is never written back to the task file.
- **Task agent binary:** `--command-path` selects the executable used per iteration. The default is `internal` (the Rust task agent). If the argument contains a slash it is resolved relative to the workspace; otherwise the CLI looks the command up on `PATH`.
- **Assembly binary:** `--assembly-path` overrides the Assembly executable (default `assembly`). Paths with slashes are resolved relative to the workspace; bare commands are resolved via `PATH`. Lever validates the Assembly CLI contract when context compilation is enabled or an override is supplied.

## Loop mode (`--loop`)

When `--loop` is provided, `lever` behaves as a loop runner. Each cycle:

1. reads the tasks file (array or `{ "tasks": ... }`) and selects the first entry whose `status` is not `completed` (missing `status` counts as `unstarted`).
2. rejects runs whose `model` is `"human"` and stops with the matching stop reason.
3. invokes the task agent with the task id, workspace, prompt, and optional assignee.
4. respects the task agent exit code to determine whether to continue, stop (with a reason), or fail.

Flags:

| Flag | Behavior | Notes |
| --- | --- | --- |
| `--tasks <path>` | overrides the default tasks file. | resolved before the task agent executes. |
| `--prompt <path>` | overrides the prompt file used for both loop logging and the task agent prompt. | forwarded via `--prompt`. |
| `--prompt-lint-summary` | inject a concise lint summary from `pack/lint.json` into the prompt when available. | forwarded via `--prompt-lint-summary`. |
| `--assignee <name>` | overrides the assignee label (used by external task agents). | not written to task metadata. |
| `--command-path <path>` | identifies which binary to run for a task invocation. | `internal` selects the Rust task agent. |
| `--assembly-path <path>` | overrides the Assembly executable for context compilation. | validated against `docs/assembly-contract.md`. |
| `--delay <seconds>` | sleeps between cycles (default `0`). | requires `--loop`. |
| `--workspace <path>` | changes the workspace directory. | also passed to the task agent. |
| `--loop <count>` | limit for task-agent invocations; default `0`. | n/a |

`--loop` semantics: `0` (the default) lets the loop behave as continuous mode—keep cycling until a stop reason occurs. Passing `--loop` with no numeric value also triggers continuous mode, so `lever --loop` and `lever --loop 0` behave identically. Any positive integer caps the number of task-agent invocations, counting each cycle regardless of exit code, and the loop should log when the limit is reached before exiting even if runnable tasks remain. Use `--delay` between cycles, but do not sleep after a terminal stop reason.

The loop should interpret task agent exit codes as follows:

- `0`: continue to the next cycle (unless `--loop` limit reached).
- `3`: “no runnable task”; log and exit cleanly.
- `4`: “task requires human”; stop and surface the message.
- `5`/`6`: “dependencies missing / cannot start”; record the stop reason and exit.
- `10`/`11`: “blocked (no result or attempt limit)”; stop with the recorded reason.
- `130`: propagate as an interruption (SIGINT), failing the loop.
- `<10` (other): treat as a hard failure and exit reports.
- `>11`: log the exit code but keep looping (these typically indicate `started`/`progress` states or other benign states).

If the task agent exits `0` and the loop still has cycles available (per `--loop` and `--tasks` content), the loop waits `--delay` seconds and restarts.

## Single-iteration mode (default)

The default run mirrors a single task-agent iteration. Flags are:

| Flag | Behavior | Notes |
| --- | --- | --- |
| `--tasks <path>` | selects the tasks JSON file (same discovery order as the loop). | resolved relative to the workspace. |
| `--task-id <id>` | run the exact `task_id`, but only if it matches the first runnable task. | requires the task to be first in order; otherwise exit code `6`. |
| `--next` | pick the first task whose `status != completed` and whose `model != human` (same search as the loop). | cannot be combined with `--task-id`. |
| `--assignee <name>` | logging label for external task agents. | optional. |
| `--workspace <path>` | ensures `git` commands and file paths run from this directory. | identical to loop mode. |
| `--prompt <path>` | overrides the prompt file for the Codex run. | also used when building the per-run prompt. |
| `--prompt-lint-summary` | inject a concise lint summary from `pack/lint.json` into the prompt when available. | requires a successful context compilation to produce `lint.json`. |
| `--reset-task` | before running, reset the selected task’s status to `unstarted`, zero `observability.run_attempts`, and stamp `observability.last_run_id`. | helpful when re-running blocked tasks after manual fixes. |

Before running Codex, the task agent must ensure:

- `codex` and `git` are installed and the workspace is a git repo.
- The chosen task exposes `title`, a non-empty `definition_of_done[]`, and `recommended.approach`. Missing metadata raises an error.
- The `model` is one of `gpt-5.1-codex-mini`, `gpt-5.1-codex`, or `gpt-5.2-codex`. `human` tasks exit with code `4`.

## Task selection rules

- Both modes interpret the tasks file as either a bare array or `{ "tasks": [...] }`.
- “Runnable” means `status != completed` and `model != human`. The loop and the task agent pick the first runnable task in file order.
- `--task-id` can target a later task only if every earlier task has `status == completed`; otherwise the agent exits with code `6` and explains which task is blocking progress.
- When the first runnable task has `model == "human"`, the agent exits `4` (hooked by the loop to stop). The loop surfaces “human input required” as the stop reason.
- Any exit code ≥`10` signals task-agent state (`10` for no output, `11` for hitting `MAX_RUN_ATTEMPTS` = 3, `12` for partial progress). The loop stops on `10`/`11` with an explanatory reason and treats `12` as a benign status (it keeps looping if cycles remain).

## Task agent run behavior

- Create `.ralph/runs/<task_id>/<run_id>` and write the snapshot (`task.json`), assembly task input (`assembly-task.json`), prompt (`prompt.md`), and codex log (`codex.jsonl`). When context compilation is enabled, also write the context compile report (`context-compile.json`). The prompt includes the base prompt file, the task title, every DoD bullet, the recommended approach, the authoritative JSON, and (when enabled) a concise lint summary derived from `pack/lint.json`.
- Maintain a rate-limit cache under `.ralph/rate_limit.json` using the default TPM/RPM caps per model.
- Run `codex exec --yolo --model <model> --output-schema .ralph/task_result.schema.json --output-last-message <result> --json --skip-git-repo-check`, streaming logs to `<run>/codex.jsonl` and collecting tokens for rate tracking.
- Interpret the `result.json` schema (`outcome`, `dod_met`, `tests`, `notes`, `blockers`). If the file is missing, exit `10` and mark the task `blocked`.
- After Codex finishes, run deterministic verification when `dod_met == true`. If `task.verification.commands` is configured, execute those commands in order via `bash -lc`; otherwise fall back to auto-detection in order: `./scripts/ci.sh`, `make ci`, `./tests/run.sh`, `pytest -q` (only if Python tests exist). Log success/failure and include command + log path with `log_line`.
- Update task status only after Codex returns: set `status = completed` when `dod_met == true` and verification passes, set `status = blocked` only for runner-detected hard blocks (attempt limit or missing `result.json`), otherwise keep `status = started`. Always stamp `observability` with `last_run_id`, `last_update_utc`, and (when available) `last_note`.
- Create a feature branch `ralph/<task_id>`, commit the run’s changes, and merge them back into `main` with a fast-forward if the run completes. Teardown ensures the workspace returns to the original branch and any auto-stashed changes are restored.

Use this contract to drive both implementation and regression tests.
