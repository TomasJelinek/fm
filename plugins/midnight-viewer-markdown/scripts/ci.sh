#!/usr/bin/env bash
set -euo pipefail
plugin_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
build_root="$(realpath -ms -- "$2")"
config="$(python3 "$plugin_root/scripts/prepare-sdk.py" "$1" "$build_root")"
# The package build selects the native --target; test and lint with the same
# target so Cargo reuses one dependency build instead of compiling it twice.
native_target="$(rustc -vV | sed -n 's/^host: //p')"
cargo fmt --manifest-path "$plugin_root/Cargo.toml" -- --check
cargo --config "$config" test --manifest-path "$plugin_root/Cargo.toml" \
    --target-dir "$build_root/target" --target "$native_target" --locked --offline --all-targets
cargo clippy --config "$config" --manifest-path "$plugin_root/Cargo.toml" \
    --target-dir "$build_root/target" --target "$native_target" --locked --offline --all-targets -- -D warnings
bash "$plugin_root/scripts/build.sh" "$1" "$build_root"
