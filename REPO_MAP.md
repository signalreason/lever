## Inventory
- Languages/frameworks/runtime: lean Bash-based tooling (`bin/ralph-loop.sh:1`, `bin/task-agent.sh:1`) augmented by small Python helpers for token estimation, rate-limiting, and Codex log parsing (`bin/task-agent.sh:275`).
- Build system/package managers: none; repo relies on shell scripts plus the required CLI stack documented in `README.md:5` (bash, jq, git, python, codex).
- Entrypoints: `bin/ralph-loop.sh:1` is the loop driver, `bin/task-agent.sh:1` runs a single Codex task, and `tests/run.sh:1` orchestrates shell-based integration tests.
- Configuration surface: environment variables in both scripts (`bin/ralph-loop.sh:14`, `bin/task-agent.sh:16`) cover workspace, task file, prompt path, assignee tag, git base branch, and rate-limit artifact location; prompt text lives under `prompts/autonomous-senior-engineer.prompt.md:1` with README linking for defaults (`README.md:21`).
- Tests: `tests/run.sh:1` sweeps every `tests/test-*.sh` fixture (which themselves declare dependencies on `jq`, `git`, `python` via `tests/helpers.sh:1`).
- Data stores/external services: canonical data is `prd.json:1` plus schema `prd.schema.json:1` and clarifying doc `docs/task-schema-details.md:1`; runtime artifacts land in `.ralph/` (run snapshots `.ralph/runs/...` and throttling file `.ralph/rate_limit.json` from `bin/task-agent.sh:28`/`934`); external deps include the Codex CLI, git, and optional log tailing via `lnav` (`README.md:71`).

## Mental model
1. Tasks live in `prd.json:1` and must obey `prd.schema.json:1` plus the checklist in `docs/task-schema-details.md:1` so the agents can reason about required metadata.
2. `bin/ralph-loop.sh:1` periodically scans the same JSON for the next incomplete, non-human task, passing task_id, files, and assignee to the agent while respecting configured workspace/prompt overrides (`bin/ralph-loop.sh:14`).
3. `bin/task-agent.sh:1` validates the selected task’s metadata (`bin/task-agent.sh:721`), captures git state, stashes dirty work (`bin/task-agent.sh:193`), and switches to an isolated `ralph/<task_id>` branch (`bin/task-agent.sh:163`).
4. The agent assembles the Codex prompt by combining the base prompt file (`bin/task-agent.sh:24`), metadata from the task, and the authoritative `task_json` snapshot, then writes schema/exec artifacts under `.ralph/runs/${TASK_ID}/${RUN_ID}` (`bin/task-agent.sh:934`).
5. Codex is invoked via `codex exec` with `--output-schema`/`--output-last-message`, rate-limit throttles, and retries, emitting JSON to `.ralph/runs/.../codex.jsonl` plus a condensed log stream (`bin/task-agent.sh:998`, `bin/task-agent.sh:446`).
6. After Codex returns, the agent reads `result.json`, logs summaries (`bin/task-agent.sh:1071`), and optionally runs verification hooks (`bin/task-agent.sh:1113-1123`) before updating `prd.json`’s `status`/`observability` via `jq` (`bin/task-agent.sh:615`).
7. Successful runs commit progress, rebase, and merge the temporary branch back into `main`, logging each stage for auditability (`bin/task-agent.sh:1186`).
8. The loop notices non-zero exit codes (SIGINT, blocked, dependency) and halts with actionable messages (`bin/ralph-loop.sh:148`).
9. `.ralph/rate_limit.json` keeps a sliding window per model to throttle future runs (`bin/task-agent.sh:28`, `bin/task-agent.sh:253`).
10. Tests under `tests/` mirror these flows with stubbed Codex/git behavior to keep the model honest (`tests/test-task-agent.sh:1`, `tests/test-ralph-loop.sh:1`, `tests/test-verification.sh:1`).

ASCII diagram:
```
[prd.json/tasks]
        |
        v
[bin/ralph-loop.sh]
        |
        v
[bin/task-agent.sh]
   /      |         \
  v       v          v
[git ops][codex CLI][.ralph artifacts]
```

Top 3 runtime flows:
1. Ralph loop cycle
   - `first_incomplete_task_fields` (`bin/ralph-loop.sh:100`) selects the next non-completed entry.
   - It builds `TASK_AGENT_ARGS` with explicit `--tasks`, `--task-id`, `--prompt`, `--workspace`, and optional `--assignee` (`bin/ralph-loop.sh:140`).
   - The loop invokes the agent, inspects exit codes (`bin/ralph-loop.sh:147-171`), and stops/pauses depending on human, blocked, or failure signals.
2. Task agent setup and git handling
   - Parses CLI args, enforces `--tasks`/`--task-id` or `--next`, and resolves paths (`bin/task-agent.sh:56-108`).
   - Records repo state, stashes dirty changes, and creates/checks out `ralph/<task_id>` (`bin/task-agent.sh:178-198`, `bin/task-agent.sh:163`).
   - Captures `RUN_ID`, dumps the task snapshot, and ensures `.ralph/task_result.schema.json` exists for Codex output (`bin/task-agent.sh:927-945`).
3. Codex execution and completion
   - Streams Codex JSON for logging (`bin/task-agent.sh:446`), sleeps when reaching rate limits (`bin/task-agent.sh:253`), and retries up to three times (`bin/task-agent.sh:998-1029`).
   - Validates `result.json`, logs summary/warnings (`bin/task-agent.sh:1071-1080`), and runs repo verification hooks (`bin/task-agent.sh:1113-1128`).
   - Updates `prd.json` status/observability (`bin/task-agent.sh:621`), commits progress, and merges finished branches (`bin/task-agent.sh:1186`).

## Start-here reading order
1. `README.md:5` — requirements, install hints, and example commands for the Ralph loop/task agent.
2. `bin/ralph-loop.sh:1` — entrypoint orchestrating task selection, argument resolution, and inter-loop error handling.
3. `bin/task-agent.sh:1` — single-task executor with git management, Codex integration, rate limiting, and verification hooks.
4. `prd.schema.json:1` — JSON Schema that every entry in `prd.json` must obey.
5. `docs/task-schema-details.md:1` — distilled checklist of required task metadata and observability fields.
6. `prompts/autonomous-senior-engineer.prompt.md:1` — base prompt stitched into every run (`README.md:21` explains how it is linked).
7. `tests/run.sh:1` — test runner that picks up every `tests/test-*.sh` fixture.
8. `tests/test-task-agent.sh:1` — ensures the agent updates task status and commits when Codex reports success.
9. `tests/test-ralph-loop.sh:1` — validates argument propagation from loop to task agent with stubs.
10. `tests/test-verification.sh:1` — confirms verification chooses `tests/run.sh` before falling back to `pytest`.

## Run / test / build
- Dev run: `ralph-loop --tasks prd.json` (setup and options described near `README.md:31`).
- Test: `./tests/run.sh` (`README.md:85`).
- Lint/format: not documented anywhere; no lint script exists in repo so nothing to run.
- Build/release: not defined; repository ships as standalone shell tooling, so the canonical way to “build” is just `git`/`codex` script edits.
- Production run: same as Dev run—launch `ralph-loop` so it iterates `prd.json` and drives `task-agent` per the README guidance (`README.md:31`).

## Debug map
- Logging: `bin/task-agent.sh:127` prints via `log_line` (timestamp + level) and `bin/task-agent.sh:446` streams Codex JSON to `.ralph/runs/.../codex.jsonl`; `bin/ralph-loop.sh:10` exposes `info`/`die` messages on stdout/stdin for loop lifecycle.
- Error propagation: both scripts use `set -euo pipefail`, `die()`/`echo >&2` helpers, and status-specific exits (`bin/ralph-loop.sh:148-171`, `bin/task-agent.sh:721-1004`) so failures bubble up to the loop or CLI caller.
- Retries/timeouts: `rate_limit_sleep`/`rate_limit_retry_delay` keep requests under per-model TPM/RPM caps (`bin/task-agent.sh:253-334`), `for attempt in 1 2 3` around the Codex invocation thwarts transient errors, and a configurable `sleep` backs off between loop cycles (`bin/ralph-loop.sh:174`).
- Common failure modes: missing prompt/tasks files (`bin/task-agent.sh:97`, `bin/ralph-loop.sh:68`), Codex producing no `result.json` (`bin/task-agent.sh:1057`), reaching the max run attempts (`bin/task-agent.sh:927`) or encountering human-block status codes (`bin/task-agent.sh:1037`, `bin/ralph-loop.sh:159`).

## Risks and footguns
1. `bin/task-agent.sh:163` checkout/rebase path resets `BASE_BRANCH`, creates/deletes `ralph/<task_id>`, and soft-resets before merging, so interrupted runs can orphan branches or drop work if git history isn’t clean.
2. `bin/task-agent.sh:193` auto-stashes dirty files and later restores them while comparing recorded file lists, which can leave partial stashes when run artifacts overlap the original dirty set (no locking around `DIRTY_FILES_FILE`).
3. `bin/task-agent.sh:934` writes to `.ralph/runs/${TASK_ID}/${RUN_ID}` and `.ralph/task_result.schema.json` without safeguards, so concurrent runs that share the same `TASK_ID` or a corrupted file can clobber each other’s artifacts.
4. `bin/task-agent.sh:998` relies on `codex exec` to produce `result.json`; if Codex exits non-zero or body is malformed the agent immediately blocks the task (`bin/task-agent.sh:1057`) with limited recovery beyond `--reset-task`.
5. `bin/task-agent.sh:615` updates `prd.json` in place via `jq` and `mv`, giving no locks—parallel invocations (if triggered manually) can interleave writes and corrupt the task file.

## Safe extension points
1. Documentation updates or schema clarity belong in `docs/task-schema-details.md:1` and `README.md:5`, keeping the runtime untouched.
2. Prompt tinkering is safe via `prompts/autonomous-senior-engineer.prompt.md:1` since `bin/task-agent.sh:24` simply concatenates it with task metadata.
3. Adding new shell tests works with `tests/run.sh:1`, which automatically discovers every `tests/test-*.sh` helper pattern defined alongside `tests/helpers.sh:1`.
4. `prd.json:1` itself is data-only—adding or reordering tasks or adjusting `definition_of_done` entries requires no code change and is validated by `prd.schema.json:1`.
5. Auxiliary log/verification hooks can plug into `bin/task-agent.sh:1113-1128` (it checks for `scripts/ci.sh`, `Makefile`, `tests/run.sh`, or `pytest`) without touching the main Codex execution path.

## Verification commands
1. `rg -n "TASK_AGENT_ARGS" bin/ralph-loop.sh` (checks that loop builds the CLI args for `task-agent`).
2. `rg -n "first_incomplete_task_fields" bin/ralph-loop.sh` (confirms reachable task selection logic before each cycle).
3. `rg -n "checkout_task_branch" bin/task-agent.sh` (validates git branch handling). 
4. `rg -n "RATE_LIMIT" bin/task-agent.sh` (inspects throttling environment and file paths). 
5. `rg -n "codex exec" bin/task-agent.sh` (targets the exact Codex invocation block). 
6. `rg -n "update_task_status" bin/task-agent.sh` (verifies status/observability updates). 
7. `rg -n "SCHEMA_PATH" bin/task-agent.sh` (ensures schema scaffolding exists before Codex runs). 
8. `rg -n "observability" prd.schema.json` (re-checks schema requirements for telemetry). 
9. `rg -n "prompt" README.md` (validates user-facing instructions for prompt linkage). 
10. `rg -n "tests/run.sh" tests/test-verification.sh` (ensures verification test references the expected helper script).

## Fast experiments
1. Run `./tests/run.sh` to exercise every shell test; expect clean output saying `Running test-*.sh` and zero exit, confirming `tests/run.sh:1` still discovers fixtures.
2. Reuse the stubbed flow from `tests/test-task-agent.sh:1`: initialize a git repo, supply `codex` stub writing `result.json`, invoke `bin/task-agent.sh --task-id T1`, and watch the script log the run, update `prd.json`, and create `.ralph/runs/T1/<run>/run_id`; inspect `codex.jsonl` for the streamed log.
3. Emulate `tests/test-ralph-loop.sh:1` by pointing `ralph-loop` at a fake workspace with a stub `task-agent` that writes its arguments; verify the `args.txt` file now contains resolved `--tasks`, `--prompt`, `--workspace`, and `--assignee`, and tail `.ralph/ralph.log` for loop progress.
