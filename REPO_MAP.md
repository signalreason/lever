# Repo Map

- Languages/frameworks/runtime: Rust CLI (`src/main.rs`, `src/task_agent.rs`) with a bash test harness (`tests/run.sh`).
- Entrypoint: `lever` binary (`src/main.rs`) orchestrates loop mode, task selection, and delegation to the internal task agent.
- Configuration surface: CLI flags and environment variables (see `README.md`); prompt templates live under `prompts/`.
- Data stores/external services: canonical task data in `prd.json` (or `tasks.json`) plus schema `prd.schema.json`; runtime artifacts land in `.ralph/` (run snapshots under `.ralph/runs/...` and `.ralph/rate_limit.json`). External deps include the Codex CLI, git, and optional verification hooks (`scripts/ci.sh`, `Makefile`, `tests/run.sh`, `pytest`).

## Primary flow

1. `lever` loads the tasks file and selects a task (`src/main.rs`).
2. Loop mode (`--loop`) repeats runs, respecting exit codes and delay (`src/main.rs`).
3. The internal task agent validates task metadata, records run state, and builds the prompt (`src/task_agent.rs`).
4. Codex is invoked via `codex exec`; results are streamed to `.ralph/runs/<task_id>/<run_id>/codex.jsonl` (`src/task_agent.rs`).
5. The task agent updates `prd.json` status/observability, runs verification hooks, and manages git branches for successful runs (`src/task_agent.rs`).

## Important files

- `src/main.rs` — CLI parsing, path resolution, loop behavior, and delegation to the internal task agent.
- `src/task_agent.rs` — task selection, prompt assembly, Codex invocation, verification, git lifecycle, and task status updates.
- `src/rate_limit.rs` — prompt token estimation and `.ralph/rate_limit.json` handling.
- `prompts/autonomous-senior-engineer.prompt.md` — base prompt template.
- `tests/run.sh` — runs every `tests/test-*.sh` script.
- `tests/test-lever-task-agent.sh` — integration test for `lever` running the internal task agent.
- `tests/test-task-agent.sh` — smoke test for task completion and status update via `lever`.
- `tests/test-ralph-loop.sh` — validates loop argument propagation to a stubbed task agent.
- `tests/test-verification.sh` — verifies hook selection behavior via `lever`.

## Key behaviors to audit

- Task selection rules and `--task-id` gating (`src/main.rs`).
- Loop stop reasons and exit code handling (`src/main.rs`).
- Run artifact layout under `.ralph/` (`src/task_agent.rs`).
- Rate-limit behavior and token estimation (`src/rate_limit.rs`).
- Verification hook ordering and logging (`src/task_agent.rs`).

## Quick checks

- `rg -n "resolve_loop_mode|run_loop_iterations" src/main.rs`
- `rg -n "run_task_agent|build_prompt|run_codex" src/task_agent.rs`
- `rg -n "rate_limit" src/rate_limit.rs`
- `rg -n "verification" src/task_agent.rs`
