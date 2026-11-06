#!/usr/bin/env bash
set -euo pipefail
plugin_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
build_root="$(realpath -m -- "$2")"
config="$(python3 "$plugin_root/scripts/prepare-sdk.py" "$1" "$build_root")"
cargo fmt --manifest-path "$plugin_root/Cargo.toml" -- --check
cargo --config "$config" test --manifest-path "$plugin_root/Cargo.toml" \
    --target-dir "$build_root/target" --locked --offline --all-targets
cargo clippy --config "$config" --manifest-path "$plugin_root/Cargo.toml" \
    --target-dir "$build_root/target" --locked --offline --all-targets -- -D warnings
bash "$plugin_root/scripts/build.sh" "$1" "$build_root"
