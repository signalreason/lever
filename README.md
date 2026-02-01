# Lever

Lever contains the Ralph loop and task agent for running Codex-driven tasks against a repo. The Rust `lever` binary now serves as the canonical entry point, while the existing shell scripts remain as compatibility wrappers for workflows that still rely on them.

## Requirements
- bash
- jq
- git
- python (for token estimates and rate limit bookkeeping)
- codex (Codex CLI)

## Install

Place the scripts on your PATH, for example:

```bash
ln -s "$PWD/bin/ralph-loop.sh" ~/bin/ralph-loop
ln -s "$PWD/bin/task-agent.sh" ~/bin/task-agent
```

Build the new `lever` CLI with Cargo:

```bash
cargo install --path .
```

This installs `lever` into your Cargo bin directory (usually `~/.cargo/bin`). Add that directory to your `PATH` if it is not already present. For local development you can also run `cargo build --bin lever` and reference `target/debug/lever` directly.

## Prompt setup

Link the default prompt into `~/.prompts` so the scripts can find it automatically:

```bash
mkdir -p ~/.prompts
ln -s "$PWD/prompts/autonomous-senior-engineer.prompt.md" \
  ~/.prompts/autonomous-senior-engineer.prompt.md
```

## Lever CLI

The `lever` binary is now the canonical entry point: it reuses the shell workflow for compatibility while adding the new loop primitives in one command. Run `lever` once to execute the next runnable task (`status != completed`, `model != human`), add `--task-id <id>` to pin a specific task, or pass `--loop` (see below) to keep invoking task-agent runs until a stop reason occurs.

### Defaults and discovery

- `--tasks` defaults to `prd.json`, falling back to `tasks.json` in the current directory if the flagged file is absent.
- The workspace is inferred from the tasks file’s directory (current directory when `prd.json`/`tasks.json` is in the repo root).
- `--prompt` defaults to `$HOME/.prompts/autonomous-senior-engineer.prompt.md`; the CLI validates the file exists before running.
- `--command-path` defaults to `bin/task-agent.sh` (workspace-relative if a slash-less name) but you can point it at another executable for testing or to call `lever` again.
- Every iteration forwards the resolved `--tasks`, `--workspace`, and `--prompt` values to the configured task agent so the behavior matches the legacy wrappers.

### Loop semantics

`--loop` accepts an optional count. Passing `--loop` with no value (or `--loop 0`) keeps cycling until a terminal stop reason occurs (no tasks, human input request, blocked run, etc.). Any positive integer limits the number of task-agent invocations; once the limit is reached, `lever` logs `lever: --loop limit reached (<count>)` and exits even if runnable tasks remain. Without `--loop`, `lever` runs only one iteration, so you can rely on the existing `--task-id` or implicit selection behavior for ad-hoc task-agent runs.

### Examples

```bash
# Continuous loop (same as `--loop 0`)
lever --loop --tasks prd.json
```

```bash
# Fixed iteration loop
lever --loop 3 --tasks prd.json
```

Prefer `lever` for new automation; the legacy shell scripts below remain as wrappers until users fully migrate.

## Ralph loop

This legacy script mirrors the Ralph loop behavior; prefer `lever --loop` once the Rust binary is on your `PATH`, but keep the crawler on hand for compatibility.

The loop drives tasks in a tasks JSON file (default: `prd.json`) using the task agent.
It operates on the current working directory by default.
Use `prd.schema.json` for editor validation and autocomplete.

```bash
ralph-loop \
  --tasks prd.json
```

Common options:
- `--tasks <path>`: tasks JSON file (default: `prd.json` in the current working directory).
- `--prompt <path>`: prompt file used by the task agent (default: `~/.prompts/autonomous-senior-engineer.prompt.md`).
- `--assignee <name>`: optional label forwarded to the task agent for log tagging (default: `ralph-loop`); task metadata no longer stores this value.
- `--task-agent <path>`: task agent executable (default: `task-agent` on PATH).
- `--log-file <path>`: append logs to this file (default: `.ralph/ralph.log`).
- `--delay <seconds>`: pause between cycles.

## Task agent

This script provides the original single-iteration behavior; the Rust `lever` binary exposes the same semantics via `lever --task-id` or a standalone invocation.

Run exactly one task iteration via Codex CLI.

```bash
task-agent \
  --tasks prd.json \
  --task-id ASM-001
```

Select the next runnable task:

```bash
task-agent \
  --tasks prd.json \
  --next \
```

Pass `--assignee <name>` if you want to tag the run in logs; task metadata no longer stores an `assignee` field.
```

## Logs

If you want a single stream you can tail with `lnav`, pipe stdout+stderr:

```bash
ralph-loop --tasks prd.json 2>&1 | lnav -
```

To keep a copy while watching:

```bash
ralph-loop --tasks prd.json 2>&1 | tee .ralph/ralph.log | lnav -
```

## Tests

```bash
./tests/run.sh
```

## Task schema guidance

Tasks must conform to `prd.schema.json`. The schema requires the following metadata for every task entry:

- `task_id`: non-empty string that uniquely identifies the task.
- `title`: non-empty string summarizing the work.
- `status`: one of `"unstarted"`, `"started"`, `"blocked"`, or `"completed"`.
- `model`: one of `"gpt-5.1-codex-mini"`, `"gpt-5.1-codex"`, `"gpt-5.2-codex"`, or `"human"`.
- `definition_of_done`: non-empty array of non-empty strings describing completion criteria.
- `recommended`: object requiring an `approach` string (no other keys allowed).

The optional `observability` object must appear only when there is recent run metadata, and it must include `run_attempts` (integer ≥ 0), `last_note` (string), `last_update_utc` (RFC 3339 / ISO 8601 string), and `last_run_id` (non-empty string).

The `assignee` property has been removed from the schema, so new tasks should no longer include it.

`bin/task-agent.sh` now checks that every selected task exposes `title`, `definition_of_done`, and `recommended.approach` before starting a run. Missing metadata surface as an error, and the new prompt includes each definition-of-done entry plus the recommended approach so the model can reason about the required behavior.

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

### use quality commit messages
the current commit messages are only useful for debugging. use the task title
for commit messages, and follow this commit message spec:

```
{
  "id": "cbea.git-commit.compact.v1",
  "message_format": "<subject>\n\n<body?>",
  "subject": {
    "single_line": true,
    "max_chars": 50,
    "capitalize_first_char": true,
    "no_trailing_period": true,
    "mood": "imperative",
    "imperative_test_prefix": "If applied, this commit will "
  },
  "body": {
    "present_requires_blank_line_after_subject": true,
    "wrap_hard_at": 72,
    "focus": ["what", "why"],
    "deprioritize": ["how"]
  }
}
```
