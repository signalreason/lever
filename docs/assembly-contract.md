# Assembly CLI Contract (Lever)

Version: 2026-02-16

This document pins the Assembly CLI interface that Lever depends on for context compilation. Lever validates the interface before running context compilation and treats mismatches as configuration errors.

## Required command

Lever expects `assembly build` to exist and accept the following flags:

- `--repo <PATH>`: repository root directory.
- `--task <PATH>`: task input file path. Lever passes `@<path>` for file contents.
- `--task-id <ID>`: task identifier (from the tasks JSON).
- `--out <DIR>`: output pack directory (e.g., `.ralph/runs/<task_id>/<run_id>/pack`).
- `--token-budget <TOKENS>`: maximum tokens for compiled context.
- `--exclude <GLOB>`: additive exclude glob (repeatable).
- `--exclude-runtime <GLOB>`: runtime artifact exclusion glob (repeatable).
- `--summary-json <PATH>`: path for a machine-readable build summary.

## Required pack outputs

After a successful build, Assembly must write a complete pack under the output directory containing:

- `manifest.json`
- `index.json`
- `context.md`
- `policy.md`
- `lint.json`

## Summary JSON

`--summary-json` must create a valid JSON document that reflects the build result. Lever currently validates that the flag exists; future integration will validate the file contents when context compilation is wired in.
