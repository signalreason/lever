# Task schema details

This document translates the README's task schema guidance and `prd.schema.json` into a precise checklist of fields, types, and constraints so every task entry stays valid.

## Root structure
- `tasks` (required): the only top-level property. It is an array whose items must match `#/definitions/task`.
- `additionalProperties: false` on the root object, so the file may not introduce any other top-level keys.

## Task object field requirements
Every task object must include the fields below. Task entries may include extra metadata (because `additionalProperties: true` is allowed at the task level), but omitting a required field causes validation to fail.

- `task_id`: non-empty `string` (min length 1).
- `title`: non-empty `string`.
- `status`: `string` limited to `"unstarted"`, `"started"`, `"blocked"`, or `"completed"`.
- `model`: `string` limited to `"gpt-5.1-codex-mini"`, `"gpt-5.1-codex"`, `"gpt-5.2-codex"`, or `"human"`.
- `definition_of_done`: array with `minItems: 1`; each entry must be a non-empty `string` (`minLength: 1`).
- `recommended`: object whose only allowed property is `approach`. That property is a non-empty `string`, and the object rejects any additional keys.

The `assignee` property has been removed, so tasks should no longer include it.

## Observability metadata (optional)
When present, the `observability` object must include all of the following fields and may not contain anything else (`additionalProperties: false`). Partial metadata is rejected, so omit the object entirely until you have a complete set of values.

- `run_attempts`: `integer` with a `minimum` of `0`.
- `last_note`: `string`.
- `last_update_utc`: `string` whose format is RFC 3339 / ISO 8601 `date-time`.
- `last_run_id`: non-empty `string` (min length 1).

Only add this object when you have real observability data from a run.
