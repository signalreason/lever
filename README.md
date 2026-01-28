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

## Ralph loop

The loop drives tasks in a tasks JSON file (default: `prd.json`) using the task agent.
It operates on the current working directory by default.

```bash
ralph-loop \
  --tasks prd.json \
  --prompt prompts/autonomous-senior-engineer.prompt.md
```

Common options:
- `--tasks <path>`: tasks JSON file (default: `prd.json` in the current working directory).
- `--prompt <path>`: prompt file used by the task agent.
- `--assignee <name>`: assignee label (default: `ralph-loop`).
- `--task-agent <path>`: task agent executable (default: `task-agent` on PATH).
- `--delay <seconds>`: pause between cycles.

## Task agent

Run exactly one task iteration via Codex CLI.

```bash
task-agent \
  --tasks prd.json \
  --task-id ASM-001 \
  --assignee ralph-loop \
  --prompt prompts/autonomous-senior-engineer.prompt.md
```

Select the next runnable task:

```bash
task-agent \
  --tasks prd.json \
  --next \
  --assignee ralph-loop \
  --prompt prompts/autonomous-senior-engineer.prompt.md
```

## Tests

```bash
./tests/run.sh
```
