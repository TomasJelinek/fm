#!/usr/bin/env bash
set -euo pipefail
plugin_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
sdk_config="$(realpath -- "$1")"
build_root="$(realpath -m -- "$2")"
case "$build_root/" in */_build/*) ;; *) echo 'Generated output must be under _build' >&2; exit 2 ;; esac
# The package build selects the native --target; test and lint with the same
# target so a Debug package build reuses the test dependency build.
native_target="$(rustc -vV | sed -n 's/^host: //p')"
cargo fmt --manifest-path "$plugin_root/Cargo.toml" -- --check
cargo --config "$sdk_config" test --manifest-path "$plugin_root/Cargo.toml" \
    --target-dir "$build_root/target" --target "$native_target" --locked --offline --all-targets
cargo clippy --config "$sdk_config" --manifest-path "$plugin_root/Cargo.toml" \
    --target-dir "$build_root/target" --target "$native_target" --locked --offline --all-targets -- -D warnings
bash "$plugin_root/scripts/build.sh" "$sdk_config" "$build_root"
