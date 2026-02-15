# Repo Map

- Stack: Rust CLI (`lever`) with Bash integration tests.
- Runtime model: `src/main.rs` provides CLI orchestration; `src/task_agent.rs` is the internal task runner.
- Source of truth: task data in `prd.json` (or `tasks.json` fallback), validated by `prd.schema.json`.
- Run artifacts: generated under `.ralph/` (not source-controlled as canonical config).

## Top-Level Layout

- `src/`
  - `main.rs`: CLI args, task discovery/selection, `--loop` behavior, internal vs external command path, git workspace guard.
  - `task_agent.rs`: task execution lifecycle (selection, prompt build, Codex run, result parsing, status updates, verification, commits).
  - `rate_limit.rs`: request/token window accounting stored in `.ralph/rate_limit.json`.
  - `task_metadata.rs`: required metadata validation (`title`, `definition_of_done`, `recommended.approach`).
- `tests/`
  - `run.sh`: executes all `tests/test-*.sh`.
  - `helpers.sh`: shared shell helpers.
  - `test-lever-*.sh`, `test-task-agent-*.sh`, `test-verification*.sh`: coverage for selection, looping, reset/attempt limits, metadata, interruption, verification, and command-path behavior.
- `prompts/`
  - `autonomous-senior-engineer.prompt.md`: base prompt template used unless `--prompt` overrides.
- `docs/`
  - `cli-contract.md`: CLI semantics and expected behavior.
  - `task-schema-details.md`: task JSON schema details and constraints.
- `.github/workflows/tests.yml`
  - CI workflow that installs deps and runs `./tests/run.sh`.
- `README.md`
  - Operator-facing usage and CLI contract summary.
- `AGENTS.md`
  - Repo-specific contribution instructions used by coding agents.
- `prd.json` / `prd.md` / `prd.schema.json`
  - Task backlog, product doc, and task schema.

## Primary Execution Flow

1. `lever` resolves workspace/tasks/prompt/command paths and validates CLI arg combinations (`src/main.rs`).
2. It picks a task via explicit `--task-id` or next-runnable logic, with additional loop stop-reason handling (`src/main.rs`).
3. The internal task agent validates task metadata/model, initializes run directories, and writes task/prompt snapshots (`src/task_agent.rs`).
4. Codex runs with JSON schema output; logs and result files are written under `.ralph/runs/<task_id>/<run_id>/` (`src/task_agent.rs`).
5. The task agent updates task status + observability fields in the tasks file, runs verification, and commits progress (`src/task_agent.rs`).
6. `lever` decides whether to continue looping, stop, or propagate an exit condition (`src/main.rs`).

## Verification Resolution Order

`src/task_agent.rs` chooses verification in this order:

1. Task-level `verification.commands` in task JSON.
2. `./scripts/ci.sh` if executable.
3. `make ci` if a `Makefile` has a `ci:` target.
4. `./tests/run.sh` if executable.
5. `pytest -q` if pytest is available and Python tests are detected.

## Operational Files Under `.ralph/`

- `.ralph/runs/<task_id>/<run_id>/task.json`: task snapshot at execution start.
- `.ralph/runs/<task_id>/<run_id>/prompt.md`: assembled prompt sent to Codex.
- `.ralph/runs/<task_id>/<run_id>/codex.jsonl`: Codex JSON event stream.
- `.ralph/runs/<task_id>/<run_id>/result.json`: structured result payload.
- `.ralph/runs/<task_id>/<run_id>/verify.log`: verification output.
- `.ralph/rate_limit.json`: rolling token/request history.
- `.ralph/task_result.schema.json`: schema enforced for Codex result output.

## Quick Audit Commands

- `rg -n "resolve_paths|determine_selected_task|run_loop_iterations" src/main.rs`
- `rg -n "run_task_agent|select_task|build_prompt|run_codex|run_verification" src/task_agent.rs`
- `rg -n "rate_limit_settings|rate_limit_sleep_seconds|record_rate_usage" src/rate_limit.rs`
- `rg -n "validate_task_metadata" src/task_metadata.rs src/main.rs src/task_agent.rs`
