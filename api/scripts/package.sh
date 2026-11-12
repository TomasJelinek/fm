#!/usr/bin/env bash
set -euo pipefail
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
cd -- "$project_root"
mkdir -p -- "$project_root/_build/tmp"
export TMPDIR="$project_root/_build/tmp"
cargo run --manifest-path api/Cargo.toml -p midnight-api-package --locked --offline -- "$project_root"
