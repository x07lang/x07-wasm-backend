#!/usr/bin/env bash
set -euo pipefail

repo_root() {
  cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd
}

root="$(repo_root)"
cd "$root"

cargo build --locked --release -p x07-wasm
export PATH="${root}/target/release:${PATH}"
bash scripts/ci/check_schema_index.sh
bash scripts/ci/check_phase10.sh
./scripts/ci/check_doss_ga_surface.sh
./target/release/x07-wasm doctor --json
