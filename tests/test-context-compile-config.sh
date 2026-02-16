#!/usr/bin/env bash
set -euo pipefail

TEST_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=helpers.sh
source "$TEST_DIR/helpers.sh"

require_cmd cargo

repo_root="$(cd "$TEST_DIR/.." && pwd)"

cargo test --quiet --manifest-path "$repo_root/Cargo.toml" context_compile_config_defaults
