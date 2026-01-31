# Lever

Lever contains the Ralph loop and task agent for running Codex-driven tasks against a repo.

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

## Prompt setup

Link the default prompt into `~/.prompts` so the scripts can find it automatically:

```bash
mkdir -p ~/.prompts
ln -s "$PWD/prompts/autonomous-senior-engineer.prompt.md" \
  ~/.prompts/autonomous-senior-engineer.prompt.md
```

## Ralph loop

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
- `--assignee <name>`: assignee label (default: `ralph-loop`).
- `--task-agent <path>`: task agent executable (default: `task-agent` on PATH).
- `--log-file <path>`: append logs to this file (default: `.ralph/ralph.log`).
- `--delay <seconds>`: pause between cycles.

## Task agent

Run exactly one task iteration via Codex CLI.

```bash
task-agent \
  --tasks prd.json \
  --task-id ASM-001 \
  --assignee ralph-loop
```

Select the next runnable task:

```bash
task-agent \
  --tasks prd.json \
  --next \
  --assignee ralph-loop
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
