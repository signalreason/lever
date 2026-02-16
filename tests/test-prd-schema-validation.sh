#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

require_cmd cargo

repo_root="$(cd "$TEST_DIR/.." && pwd)"
tmp_dir="$(make_temp_dir)"
trap 'rm -rf "$tmp_dir"' EXIT

cat > "$tmp_dir/valid.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "VALID-1",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "title": "Valid schema task",
      "definition_of_done": [
        "A minimal valid task exists"
      ],
      "recommended": {
        "approach": "Keep the task minimal and schema-compliant."
      }
    }
  ]
}
JSON

cat > "$tmp_dir/invalid.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "INVALID-1",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "title": "Invalid schema task",
      "definition_of_done": [
        "This task is missing recommended.approach"
      ]
    }
  ]
}
JSON

cargo run --quiet --manifest-path "$repo_root/Cargo.toml" --bin validate_prd -- \
  --tasks "$repo_root/prd.json" \
  --schema "$repo_root/prd.schema.json" \
  >/dev/null

cargo run --quiet --manifest-path "$repo_root/Cargo.toml" --bin validate_prd -- \
  --tasks "$tmp_dir/valid.json" \
  --schema "$repo_root/prd.schema.json" \
  >/dev/null

if cargo run --quiet --manifest-path "$repo_root/Cargo.toml" --bin validate_prd -- \
  --tasks "$tmp_dir/invalid.json" \
  --schema "$repo_root/prd.schema.json" \
  >"$tmp_dir/invalid.log" 2>&1; then
  echo "Expected validate_prd to fail for invalid tasks file" >&2
  exit 1
fi

if ! grep -q "Validation failed" "$tmp_dir/invalid.log"; then
  echo "Expected validate_prd failure output to include 'Validation failed'" >&2
  exit 1
fi
