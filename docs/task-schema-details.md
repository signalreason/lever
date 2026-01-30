# Task schema details

## Root structure
- `tasks` (required): array of task objects validated against `#/definitions/task`.
- The root object rejects any other properties (`additionalProperties: false`), so the file may only contain `tasks`.

## Task object field requirements
Every task object must include all of the following fields. Additional keys are allowed only at the task level (`additionalProperties: true`), so metadata or auxiliary information can be added without breaking validation.

- `task_id`: non-empty `string` (minimum length 1).
- `title`: non-empty `string`.
- `status`: `string` restricted to the enumerated values `["unstarted", "started", "blocked", "completed"]`.
- `model`: `string` restricted to `["gpt-5.1-codex-mini", "gpt-5.1-codex", "gpt-5.2-codex", "human"]`.
- `definition_of_done`: array with at least 1 entry (`minItems: 1`). Every entry must itself be a non-empty `string` (`minLength: 1`).
- `recommended`: object that **must** contain only an `approach` field. `approach` is a non-empty `string`, and the object rejects any additional properties.

Tasks no longer include an `assignee` field—remove it from existing entries.

## Observability metadata (optional)
When present, the `observability` object **must** include all of the fields listed below and may not include anything extra (`additionalProperties: false`). Partial observability metadata is rejected.

- `run_attempts`: `integer`, minimum value `0`.
- `last_note`: `string`.
- `last_update_utc`: `string` formatted as RFC 3339 / ISO 8601 `date-time`.
- `last_run_id`: non-empty `string` (minimum length 1).

Keep the observability object out of tasks that never ran; only add it when you have concrete metadata to record.
