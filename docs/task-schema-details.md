# Task schema details

## Root structure
- `tasks` (required): array of task objects validated against `#/definitions/task`. The root object rejects any properties besides `tasks`.

## Task object required fields
- `task_id`: non-empty `string` (min length 1).
- `title`: non-empty `string`.
- `status`: `string` limited to `["unstarted", "started", "blocked", "completed"]`.
- `model`: `string` limited to `["gpt-5.1-codex-mini", "gpt-5.1-codex", "gpt-5.2-codex", "human"]`.
- `definition_of_done`: non-empty array (min 1 item) where every entry is a non-empty `string`.
- `recommended`: object that **must** include `approach` (non-empty `string`) and rejects any other properties.

Tasks may include additional data beyond these fields (`additionalProperties` is allowed at the task level), but every task must provide the six fields above.
The `assignee` field has been removed from the schema and should no longer appear on new task entries.

## Observability metadata (optional)
When present, the `observability` object **must** include all of these fields and no others:
- `run_attempts`: `integer` with a minimum value of 0.
- `last_note`: `string`.
- `last_update_utc`: `string` in RFC 3339 / ISO 8601 `date-time` format.
- `last_run_id`: non-empty `string`.

This object is optional for a task, but if any observability metadata is recorded it must include this complete set; partial metadata is not allowed.
