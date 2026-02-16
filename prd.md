# Product Requirements Document: Assembly Context in Lever

## Overview
Integrate `assembly` context compilation into the `lever` task-agent workflow so each run can attach deterministic repo context (`pack/context.md`) to the Codex prompt. The integration must preserve existing run semantics and artifact layout while adding optional, auditable context packs per run.

## Goals
- Compile context for each task-agent run using `assembly build`.
- Store pack artifacts under the run directory (`.ralph/runs/<task_id>/<run_id>/pack/`).
- Inject compiled context into the final prompt sent to Codex.
- Keep integration deterministic and traceable via `manifest.json`.
- Ship changes in dependency-first order and grouped by repo.

## Non-Goals
- Replacing Lever task selection, verification, or git guard behavior.
- Replacing Assembly scoring heuristics beyond what is needed for safe Lever integration.
- Introducing cross-repo tasks that require simultaneous edits in both repos.

## Current State
- `lever` currently builds prompt text from base prompt + task metadata in `src/task_agent.rs` (`build_prompt`), then runs Codex.
- `assembly` already compiles deterministic `pack/` outputs (`manifest.json`, `index.json`, `context.md`, `policy.md`, `lint.json`) via `assembly build`.
- No first-class handoff exists between these systems today.

## Target Workflow
1. Lever selects a task and creates run artifacts as it does today.
2. Lever calls Assembly to compile a pack into the current run directory.
3. Lever appends `pack/context.md` (and optionally lint summary) into the generated prompt.
4. Lever logs context compile status and pack paths in run logs.
5. Codex executes with the augmented prompt; verification and task status behavior remain unchanged.

## Repo Scope and Dependency
- Assembly-side prerequisites are tracked in `/Users/xwoj/src/assembly/prd.md`.
- Lever work in this document starts only after the Assembly PRD is completed and released.

## Lever Plan (After Assembly Prerequisites)
1. Add context compile configuration in CLI and runtime config.
   - Add flags for enabling/disabling context compile, selecting required vs best-effort behavior, and setting context token budget.
   - Add optional Assembly executable path override for local/custom installations.
2. Add Assembly invocation in `task_agent` run lifecycle.
   - After run directory creation and task snapshot write, invoke Assembly with:
     - `--repo <workspace>`
     - `--task @<run_dir>/task.json` (or a dedicated task-brief file)
     - `--task-id <task_id>`
     - `--out <run_dir>/pack`
     - configured token budget + additive excludes
   - Capture stdout/stderr into run artifacts for debugging.
3. Extend prompt building to consume compiled context.
   - Update prompt assembly in `src/task_agent.rs` to append compiled context blocks from `pack/context.md`.
   - Include concise provenance in prompt (`manifest` path and commit SHA).
4. Define failure policy and exit semantics.
   - Best-effort mode: log warning and continue without compiled context.
   - Required mode: fail run before Codex execution if context compile fails or expected pack files are missing.
5. Add tests and fixtures.
   - Unit tests for command construction and prompt augmentation.
   - Bash integration tests with a stub Assembly command for: success, best-effort failure, required-mode failure, and pack path correctness.
6. Update Lever docs.
   - Update `README.md` and `REPO_MAP.md` with context compile lifecycle and new flags.

## Delivery Order
1. Complete `/Users/xwoj/src/assembly/prd.md` (Assembly prerequisites).
2. Pin Lever integration to the updated Assembly interface.
3. Ship Lever with context compile defaulting to best-effort.
4. After soak period and test confidence, consider flipping default to required mode.

## Acceptance Criteria
- Every context-enabled Lever run writes a complete pack under `.ralph/runs/<task_id>/<run_id>/pack/`.
- Prompt includes deterministic compiled context from `pack/context.md`.
- Lever behavior without context compile remains backward compatible.
- Required mode fails early and clearly when context compile fails.
- Integration tests cover success and both failure policies.
- Documentation in both repos reflects the final operator workflow.

## Risks and Mitigations
- Assembly availability mismatch across environments.
  - Mitigation: explicit Assembly path flag + clear startup validation message.
- Context bloat from unintended files.
  - Mitigation: enforce `.ralph/**` exclusion and additive glob controls.
- Prompt size regression.
  - Mitigation: configurable context token budget and explicit truncation handling.
- Integration drift across repos.
  - Mitigation: versioned CLI contract and dependency-first rollout.

## Open Questions
- Should Lever default to best-effort context compile forever, or switch to required mode once rollout stabilizes?
- Should lint findings from `pack/lint.json` be injected into prompt, or only logged for operator visibility?
- Should the task input to Assembly remain `@task.json`, or move to a generated task-brief markdown file for better relevance?
