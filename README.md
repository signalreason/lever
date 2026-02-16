# Lever

Lever contains the Ralph loop and task agent for running Codex-driven tasks against a repo. The Rust `lever` binary is the canonical entry point.

## Requirements
- bash
- jq
- git
- python (for token estimates and rate limit bookkeeping)
- codex (Codex CLI)

## Install

Build the `lever` CLI with Cargo:

```bash
cargo install --path .
```

This installs `lever` into your Cargo bin directory (usually `~/.cargo/bin`). Add that directory to your `PATH` if it is not already present. For local development you can also run `cargo build --bin lever` and reference `target/debug/lever` directly.

## Prompt setup

The default prompt is `prompts/autonomous-senior-engineer.prompt.md` under the workspace. If you run `lever` in a repo without that file and do not supply `--prompt`, `lever` will fail with `Prompt file not found: <workspace>/prompts/autonomous-senior-engineer.prompt.md`. Pass `--prompt /path/to/prompt.md` in that case.

## Lever CLI

The `lever` binary is the canonical entry point. Run `lever` once to execute the next runnable task (`status != completed`, `model != human`), add `--task-id <id>` to pin a specific task, use `--next` to force "next runnable" selection, or pass `--loop` (see below) to keep invoking task-agent runs until a stop reason occurs.

### Defaults and discovery

- `--tasks` defaults to `prd.json`, falling back to `tasks.json` in the current directory if the flagged file is absent.
- `--workspace` defaults to the current directory; when `--tasks` is omitted the tasks file is discovered relative to that workspace.
- `--prompt` defaults to `prompts/autonomous-senior-engineer.prompt.md` under the workspace; the CLI validates the file exists before running.
- `--command-path` defaults to `internal` (the Rust task agent). You can point it at another executable for testing or for delegating work to a different task agent binary.
- Every iteration forwards the resolved `--tasks`, `--workspace`, and `--prompt` values to the configured task agent so the behavior stays consistent with the legacy workflow.
- `--assignee` is forwarded to external task agents when `--command-path` is not `internal`.
- `--reset-task` clears attempt counters for the selected task before running.
- `--delay` inserts a sleep between loop iterations (seconds, default 0; only valid with `--loop`).
- `--next` selects the first task whose status is not `completed` and whose model is not `human`; it cannot be combined with `--task-id`.

### Loop semantics

`--loop` accepts an optional count. Passing `--loop` with no value (or `--loop 0`) keeps cycling until a terminal stop reason occurs (no tasks, human input request, blocked run, etc.). Any positive integer limits the number of task-agent invocations; once the limit is reached, `lever` logs `lever: --loop limit reached (<count>)` and exits even if runnable tasks remain. Without `--loop`, `lever` runs only one iteration, so you can rely on the existing `--task-id` or implicit selection behavior for ad-hoc task-agent runs.

### Exit codes

`lever` mostly forwards the task agent’s exit code. In loop mode it interprets some codes to decide when to stop.

- `0`: Success (single iteration completed) or loop ended normally (no remaining tasks, loop limit reached, or clean shutdown).
- `1`: Lever stop reason (human input required, blocked by dependencies, or blocked run detected in loop mode).
- `2`: Invalid task metadata, unsupported model, or invalid task selection input.
- `3`: Task agent reports no runnable tasks.
- `4`: Task agent selected a human task.
- `6`: Task agent reports a dependency ordering issue.
- `10`: Task agent blocked because no `result.json` was produced.
- `11`: Task agent blocked (attempt limit reached before run).
- `12`: Task agent recorded progress (run completed without deterministic success).
- `130`: Interrupted (SIGINT/CTRL-C).

### Examples

```bash
# Continuous loop (same as `--loop 0`)
lever --loop --tasks prd.json
```

```bash
# Fixed iteration loop
lever --loop 3 --tasks prd.json
```

Prefer `lever` for new automation and tooling.

## Logs

If you want a single stream you can tail with `lnav`, pipe stdout+stderr:

```bash
lever --loop --tasks prd.json 2>&1 | lnav -
```

To keep a copy while watching:

```bash
lever --loop --tasks prd.json 2>&1 | tee .ralph/ralph.log | lnav -
```

## Tests

```bash
./tests/run.sh
```

`./tests/run.sh` now runs schema validation up front via the Rust validator binary. You can run schema validation directly with:

```bash
cargo run --quiet --bin validate_prd -- --tasks prd.json --schema prd.schema.json
```

## Assembly contract validation

Lever pins the Assembly CLI contract in `docs/assembly-contract.md`. You can validate a local Assembly installation with:

```bash
cargo run --quiet --bin validate_assembly_contract -- --assembly assembly
```

## Task schema guidance

Tasks must conform to `prd.schema.json`. The schema requires the following metadata for every task entry:

- `task_id`: non-empty string that uniquely identifies the task.
- `title`: non-empty string summarizing the work.
- `status`: one of `"unstarted"`, `"started"`, `"blocked"`, or `"completed"`.
- `model`: one of `"gpt-5.1-codex-mini"`, `"gpt-5.1-codex"`, `"gpt-5.2-codex"`, or `"human"`.
- `definition_of_done`: non-empty array of non-empty strings describing completion criteria.
- `recommended`: object requiring an `approach` string (no other keys allowed).
- `verification` (optional): object with optional `commands` array of non-empty shell command strings. When present, these commands run (in order) as the deterministic verification step.

The optional `observability` object must appear only when there is recent run metadata, and it must include `run_attempts` (integer ≥ 0), `last_note` (string), `last_update_utc` (RFC 3339 / ISO 8601 string), and `last_run_id` (non-empty string).

The `assignee` property has been removed from the schema, so new tasks should no longer include it.

The Rust task agent now checks that every selected task exposes `title`, `definition_of_done`, and `recommended.approach` before starting a run. Missing metadata surface as an error, and the prompt includes each definition-of-done entry plus the recommended approach so the model can reason about the required behavior.

Minimal compliant example:

```
    {
      "task_id": "ASM-004",
      "title": "Update README schema guidance",
      "model": "gpt-5.1-codex-mini",
      "status": "started",
      "definition_of_done": [
        "README describes every required task field",
        "Example reflects current schema",
        "Observability guidance notes required keys"
      ],
      "recommended": {
        "approach": "Keep the example minimal but complete and valid."
      }
    }
```
