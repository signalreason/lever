# Product Requirements Document: Lever CLI (Rust)

## Overview
Unify the existing `task-agent` and `ralph-loop` entrypoints into a single Rust binary named `lever`. The new CLI defaults to `task-agent` behavior (single iteration) and optionally runs in loop mode when `--loop` is provided. The goal is to provide a single, ergonomic interface while preserving existing behavior, flags, and output semantics.

## Goals
- Deliver a single Rust binary `lever` that covers the behavior of both `task-agent` and `ralph-loop`.
- Default behavior is a single task-agent iteration.
- `--loop` runs multiple iterations (or continuous loop) and stops on Ctrl-C.
- Preserve existing user workflows and flags as closely as possible.
- Keep compatibility with current bash scripts (initially via delegation or full port, depending on scope).

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
- (Optional extension, if approved) `--loop` with no value runs until Ctrl-C.

### Arguments & Flags
- `--tasks <PATH>` (optional): path to tasks JSON file. If not provided, look for `prd.json` or `tasks.json` in the current directory, in that order, and use the first one found. If neither exists, exit with an error.
- `--task-id <ID>` (optional): if provided, run that task from the tasks file.
- `--prompt <PATH>`: prompt template file.
- `--loop [N]`: run loop mode continuously if `N` is omitted; run exactly `N` iterations if provided (e.g., `--loop 3`).
- Pass through any other flags currently supported by `task-agent`/`ralph-loop`.

### Exit Codes
- `0` on successful completion of the requested iterations.
- Non-zero if any iteration fails; error should be surfaced with a concise message.

## Functional Requirements
1. **Tasks file discovery**: if `--tasks` is not given, search for `prd.json` then `tasks.json` in the current directory and use the first one found; error if neither exists.
2. **Task selection**:\n+   - If `--task-id` is given, run that task (error if not found).\n+   - If `--task-id` is not given and `--loop` is not provided, run the next task that is not in `completed` status.\n+3. **Single iteration**: runs `task-agent` logic exactly once.
4. **Loop iteration**: repeats the iteration logic `N` times if provided; otherwise runs until Ctrl-C.
5. **Ctrl-C handling**: stop cleanly after current iteration, exit with `0` unless the current iteration failed.
6. **Compatibility**: initial implementation may wrap existing bash scripts to preserve behavior.
7. **Logging**: output should remain consistent with existing scripts where possible.

## Non-Functional Requirements
- **Reliability**: no silent failures; errors must bubble up.
- **Portability**: run on macOS and Linux with standard Rust toolchain.
- **Performance**: overhead of Rust wrapper should be negligible.
- **Maintainability**: centralized CLI parsing and iteration logic.

## UX & Error Handling
- Clear error message for missing required flags.
- Validate `--loop` is either omitted (continuous mode) or a non-negative integer.
- Print a short summary per iteration (start/end) if it doesn’t break existing output conventions.

## Implementation Plan (Phased)
### Phase 1: Rust wrapper
- Create Rust CLI with `clap` that supports `--loop [N]`.
- Implement `run_once()` that spawns existing `bin/task-agent.sh` using `std::process::Command`.
- Implement loop logic in Rust.
- Implement Ctrl-C handling using `ctrlc` crate.

### Phase 2: Partial port
- Move shared bash logic into Rust where feasible.
- Reduce reliance on shell script execution.

### Phase 3: Full port
- Replace `task-agent.sh` and `ralph-loop.sh` with thin wrappers or deprecate them.

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
- Integration tests that run `lever` with a known tasks file (can reuse existing bash test harness).
- Test Ctrl-C handling manually or via integration harness.

## Documentation Updates
- Update `README.md` with new `lever` usage and examples.
- Document loop semantics and Ctrl-C behavior.
- Note any deprecation of `task-agent` and `ralph-loop` scripts.

## Open Questions
- Should `--loop 0` be treated as a no-op (exit 0) or as an error?
- Are there additional flags currently supported by `task-agent`/`ralph-loop` that must be mirrored explicitly?
