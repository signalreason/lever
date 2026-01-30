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

Tasks must look similar to the example below and include the mandatory metadata the schema enforces:

```
    {
      "task_id": "ASM-001",
      "title": "Define pack/manifest.json schema in INTERFACE.md",
      "model": "gpt-5.1-codex",
      "status": "completed",
      "definition_of_done": [
        "INTERFACE.md exists in repo root.",
        "Manifest section lists required fields with types and constraints.",
        "repo_commit fallback of \"unknown\" and created_at RFC3339 are documented.",
        "Minimal manifest example JSON included."
      ],
      "recommended": {
        "approach": "Keep schema descriptions precise and machine-oriented; prefer tables or bullet lists."
      },
      "observability": {
        "run_attempts": 1,
        "last_note": "Run 20260127T224200Z-65016 completed",
        "last_update_utc": "2026-01-27T22:42:00Z",
        "last_run_id": "65016"
      }
    }
```

Required task fields: `task_id`, `title`, `model`, `status`, `definition_of_done`, and `recommended`. `definition_of_done` must be a non-empty array of non-empty strings, and `recommended` is an object that requires an `approach` string (additional properties are rejected).

Observability metadata is optional, but when present it must include:
- `run_attempts`: integer ≥ 0
- `last_note`: string
- `last_update_utc`: RFC 3339 / ISO 8601 timestamp (`string` with `date-time` format)
- `last_run_id`: non-empty `string`

The `assignee` property has been removed from the schema, so tasks should no longer include it.

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
