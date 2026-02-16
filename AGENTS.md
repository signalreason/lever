# Repository Guidelines

## Project Structure
- `prompts/`: Prompt templates passed to Codex (start with `prompts/autonomous-senior-engineer.prompt.md`).
- `tests/`: Bash test harness and test cases (`tests/run.sh`, `tests/test-*.sh`).
- `README.md`: Usage and CLI examples.

## Setup & Requirements
This repo is Rust-first with bash test harnesses. You need `bash`, `cargo`, `jq`, `git`, `python`, and the `codex` CLI available on `PATH` (see `README.md`).

## Build, Test, and Development Commands
- `./tests/run.sh`: Validates `prd.json` against `prd.schema.json`, then runs every `tests/test-*.sh` script and reports failures.
- `cargo run --quiet --bin validate_prd -- --tasks prd.json --schema prd.schema.json`: Validates task JSON against the schema.
- `cargo run -- --loop --tasks prd.json --prompt prompts/autonomous-senior-engineer.prompt.md`: Drives a tasks file in a loop.
- `cargo run -- --tasks prd.json --task-id ASM-001 --prompt prompts/autonomous-senior-engineer.prompt.md`: Runs a single task iteration.

## Coding Style & Naming Conventions
- Shell scripts should be `bash` with `set -euo pipefail`.
- Use long flags and clear variable names (`TASKS_FILE`, `PROMPT_FILE`).
- Prefer small helper functions over inline blocks; keep error messages actionable.
- Test files use `tests/test-*.sh` naming.

## Testing Guidelines
- Framework: plain Bash scripts.
- Run all tests with `./tests/run.sh`.
- Each test should exit non-zero on failure and print a concise failure reason.

## Commit & Pull Request Guidelines
- Commit messages are short, imperative, and scoped to the change (e.g., "Generalize scripts and add tests").
- PRs should include:
  - A short summary of behavior changes.
  - The exact commands run and their results (e.g., `./tests/run.sh`).
  - Linked task IDs if applicable (e.g., `ASM-001`).

## Configuration & Artifacts
- The task agent writes run artifacts under `.ralph/` (e.g., `./.ralph/runs/...`).
- Treat task JSON as the source of truth; avoid editing run artifacts by hand.

## Change Management
- Any edits to `prd.json` must remain compliant with `prd.schema.json`.
- When making changes of any kind, check `README.md` and `AGENTS.md` to see if they need updates.
