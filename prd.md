# Product Requirements Document: Lever CLI (Rust)

## Overview
Move the complete Ralph loop and task agent behavior into the Rust `lever` binary so it can run from any repo without relying on `./bin/task-agent.sh` or `./bin/ralph-loop.sh`. The CLI should preserve current semantics, flags, and output as closely as possible while removing the shell script dependency.

## Goals
- Deliver a single Rust binary `lever` that covers the behavior of both `task-agent` and `ralph-loop`.
- Default behavior is a single task-agent iteration.
- `--loop` runs multiple iterations (or continuous loop) and stops on Ctrl-C.
- Preserve existing user workflows and flags as closely as possible.
- Remove reliance on shell scripts so `lever` can be executed from any repo/location.

## Non-Goals
- Rewriting Codex CLI behavior or internal prompts.
- Changing task JSON formats or artifact layout under `.ralph/`.
- Modifying output formats beyond what is needed for Rust CLI.
- Adding new orchestration features unrelated to loop behavior.

## Users & Use Cases
- **Primary user**: developers running task automation locally via `task-agent` or `ralph-loop`.
- **Use cases**:
  - Run a single task iteration for a specific task ID.
  - Run repeated iterations for a task file until a count is reached or the user stops it.
  - Use a single binary in CI or local scripts.

## CLI Requirements
### Command Name
- `lever`

### Core Behavior
- Default: one iteration, equivalent to `task-agent`.
- Loop mode: `--loop N` runs exactly `N` iterations; stop early on Ctrl-C.
- `--loop` with no value runs until Ctrl-C.

### Arguments & Flags
- `--tasks <PATH>` (optional): path to tasks JSON file. If not provided, look for `prd.json` or `tasks.json` in the current directory, in that order, and use the first one found. If neither exists, exit with an error.
- `--task-id <ID>` (optional): if provided, run that task from the tasks file.
- `--next` (optional): run the next task with `status != completed` and `model != human`.
- `--prompt <PATH>`: prompt template file.
- `--workspace <PATH>`: repo directory to run in (default: current directory).
- `--assignee <NAME>`: log tag only (not persisted to tasks).
- `--reset-task`: reset attempt counters/status for the selected task before running.
- `--delay <SECONDS>`: pause between loop iterations (from ralph-loop).
- `--loop [N]`: run loop mode continuously if `N` is omitted; run exactly `N` iterations if provided (e.g., `--loop 3`).

### Exit Codes
- `0` on successful completion of the requested iterations.
- Non-zero if any iteration fails; error should be surfaced with a concise message.

## Functional Requirements
1. **Tasks file discovery**: if `--tasks` is not given, search for `prd.json` then `tasks.json` in the current directory and use the first one found; error if neither exists.
2. **Task selection**:
   - If `--task-id` is given, run that task (error if not found).
   - If `--next` is given, run the first task with `status != completed` and `model != human`.
   - If neither `--task-id` nor `--next` is given, and `--loop` is not set, default to the first task with `status != completed` and `model != human` (matching existing lever behavior).
3. **Single iteration**: runs full task-agent logic exactly once (including task validation, logging, git stashing/branching, and rate limiting).
4. **Loop iteration**: repeats the iteration logic `N` times if provided; otherwise runs until Ctrl-C or a stop reason (no tasks, human task, blocked, dependency).
5. **Ctrl-C handling**: stop cleanly after current iteration, exit with `0` unless the current iteration failed.
6. **No shell dependency**: all execution paths are inside the Rust binary; no invocation of `bin/task-agent.sh` or `bin/ralph-loop.sh`.
7. **Logging**: output should remain consistent with existing scripts where possible.
8. **Exit code parity**: preserve exit codes used by the existing shell scripts for stop reasons and errors.

## Non-Functional Requirements
- **Reliability**: no silent failures; errors must bubble up.
- **Portability**: run on macOS and Linux with standard Rust toolchain.
- **Performance**: overhead of Rust wrapper should be negligible.
- **Maintainability**: centralized CLI parsing and iteration logic.

## UX & Error Handling
- Clear error message for missing required flags.
- Validate `--loop` is either omitted (continuous mode) or a non-negative integer.
- Print a short summary per iteration (start/end) if it doesn’t break existing output conventions.

## Implementation Plan
1. **Inventory behavior and exit codes**
   - Read `bin/task-agent.sh` and `bin/ralph-loop.sh` to enumerate flags, defaults, and exit codes.
   - Map all stop reasons and their numeric exit codes to a Rust enum.
2. **Port task-agent logic into Rust**
   - Implement CLI flags in `src/main.rs` using `clap` for: `--tasks`, `--task-id`, `--next`, `--prompt`, `--workspace`, `--assignee`, `--reset-task`.
   - Port task validation, task selection, rate-limit bookkeeping, git stashing/branch switching, and prompt construction.
   - Replace shell execs (`jq`, `git`, `codex`) with Rust implementations or explicit `Command` invocations where appropriate.
3. **Port loop logic into Rust**
   - Implement `--loop [N]` and `--delay` in Rust.
   - Preserve stop-reason handling (human task, blocked task, unmet dependencies).
4. **Remove shell scripts as execution dependencies**
   - Delete or deprecate `bin/task-agent.sh` and `bin/ralph-loop.sh`.
   - Update any references to these scripts in tests and docs.
5. **Update documentation and tests**
   - Update `README.md` to remove install instructions for shell scripts and document new flags.
   - Update `tests/` to call `lever` directly, including loop behavior and exit codes.
6. **Polish**
   - Add targeted Rust unit tests for task selection and exit code mapping.
   - Add integration tests in `tests/` for representative flows.

## Dependencies
- Rust toolchain (stable)
- `clap` for CLI parsing
- `ctrlc` for signal handling

## Risks & Mitigations
- **Behavior drift**: mitigate by wrapping existing scripts initially.
- **Ctrl-C handling**: ensure graceful shutdown to avoid corrupting `.ralph/` artifacts.
- **Flag compatibility**: document any differences clearly in README.
- **Task selection behavior**: ensure the “next not completed” selection matches existing expectations.

## Testing Requirements
- Unit tests for CLI parsing, tasks file discovery, and loop count validation.
- Unit tests for task selection and exit code mapping.
- Integration tests that run `lever` with known tasks files (reuse existing bash harness).
- Test Ctrl-C handling manually or via integration harness.

## Documentation Updates
- Update `README.md` with new `lever` usage and examples.
- Document loop semantics, exit codes, and Ctrl-C behavior.
- Remove references to installing shell scripts and note they are removed.

## Open Questions
- Should `--loop 0` be treated as a no-op (exit 0) or as "loop forever"? (Existing `lever` treats 0 as infinite.)
- Should `--next` be required for single-iteration runs without `--task-id`, or should `lever` continue auto-selecting the next task by default?
