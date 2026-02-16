#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

require_cmd cargo

repo_root="$(cd "$TEST_DIR/.." && pwd)"
stub_dir="$(make_temp_dir)"
trap 'rm -rf "$stub_dir"' EXIT

cat > "$stub_dir/assembly" <<'EOF'
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
chmod +x "$stub_dir/assembly"

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

cargo build --quiet --manifest-path "$repo_root/Cargo.toml" --bin validate_assembly_contract
validator_bin="$repo_root/target/debug/validate_assembly_contract"

"$validator_bin" --assembly "$stub_dir/assembly"

output="$("$validator_bin" --assembly "$stub_dir/assembly-bad" 2>&1 || true)"
if [[ -z "$output" ]]; then
  echo "Expected validation error output for missing flags" >&2
  exit 1
fi
if [[ "$output" != *"missing required build flags"* ]]; then
  echo "Expected missing flag error, got: $output" >&2
  exit 1
fi
