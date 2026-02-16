#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

require_cmd cargo
require_cmd git

repo_root="$(cd "$TEST_DIR/.." && pwd)"
repo_dir="$(make_temp_dir)"
stub_dir="$(make_temp_dir)"
args_dir="$(make_temp_dir)"
trap 'rm -rf "$repo_dir" "$stub_dir" "$args_dir"' EXIT

cat > "$repo_dir/prd.json" <<'JSON'
{
  "tasks": [
    {
      "task_id": "T1",
      "title": "Assembly path override",
      "status": "unstarted",
      "model": "gpt-5.1-codex-mini",
      "definition_of_done": [
        "Ensure assembly path overrides are forwarded"
      ],
      "recommended": {
        "approach": "Keep this test minimal"
      }
    }
  ]
}
JSON

ensure_workspace_prompt "$repo_dir"
init_git_repo "$repo_dir"

mkdir -p "$stub_dir"
cat > "$stub_dir/assembly-ok" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  printf '%s\n' "assembly 1.2.3"
  exit 0
fi

if [[ "${1:-}" == "build" && "${2:-}" == "--help" ]]; then
  cat <<'HELP'
Usage: assembly build [OPTIONS]

Options:
  --repo <PATH>           Repository root
  --task <PATH>           Task input file (supports @file)
  --task-id <ID>          Task identifier
  --out <DIR>             Output pack directory
  --token-budget <TOKENS> Token budget for context
  --exclude <GLOB>        Additive exclude glob (repeatable)
  --exclude-runtime <GLOB> Runtime artifact exclusion glob (repeatable)
  --summary-json <PATH>   Write machine-readable summary JSON
HELP
  exit 0
fi

echo "unexpected args: $*" >&2
exit 1
EOF
chmod +x "$stub_dir/assembly-ok"

cat > "$stub_dir/assembly-bad" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "--version" ]]; then
  printf '%s\n' "assembly 1.2.3"
  exit 0
fi

if [[ "${1:-}" == "build" && "${2:-}" == "--help" ]]; then
  cat <<'HELP'
Usage: assembly build [OPTIONS]

Options:
  --repo <PATH>           Repository root
  --task <PATH>           Task input file (supports @file)
  --task-id <ID>          Task identifier
  --out <DIR>             Output pack directory
  --token-budget <TOKENS> Token budget for context
  --exclude <GLOB>        Additive exclude glob (repeatable)
  --exclude-runtime <GLOB> Runtime artifact exclusion glob (repeatable)
HELP
  exit 0
fi

echo "unexpected args: $*" >&2
exit 1
EOF
chmod +x "$stub_dir/assembly-bad"

cat > "$stub_dir/flag-stub" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$@" > "${ARGS_FILE}"
exit 0
EOF
chmod +x "$stub_dir/flag-stub"

(
  cd "$repo_root"
  cargo build --quiet
)
lever_bin="$repo_root/target/debug/lever"

args_file="$args_dir/args.txt"
ARGS_FILE="$args_file" "$lever_bin" \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --command-path "$stub_dir/flag-stub" \
  --task-id T1 \
  --assembly-path "$stub_dir/assembly-ok" \
  >/dev/null

if ! grep -Fxq -- "--assembly-path" "$args_file"; then
  echo "Expected --assembly-path to be passed to task agent" >&2
  exit 1
fi
assembly_real="$(cd "$stub_dir" && pwd -P)/assembly-ok"
if ! grep -Fxq -- "$assembly_real" "$args_file"; then
  echo "Expected assembly path value to be passed to task agent" >&2
  exit 1
fi

set +e
bad_output="$($lever_bin \
  --workspace "$repo_dir" \
  --tasks prd.json \
  --command-path "$stub_dir/flag-stub" \
  --task-id T1 \
  --assembly-path "$stub_dir/assembly-bad" \
  2>&1)"
bad_status=$?
set -e

if [[ $bad_status -eq 0 ]]; then
  echo "Expected invalid assembly contract to fail" >&2
  exit 1
fi

if [[ "$bad_output" != *"missing required build flags"* ]]; then
  echo "Expected assembly validation error, got: $bad_output" >&2
  exit 1
fi
