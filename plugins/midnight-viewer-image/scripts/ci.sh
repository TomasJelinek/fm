#!/usr/bin/env bash
set -euo pipefail
plugin_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
sdk_config="$(realpath -- "$1")"
build_root="$(realpath -m -- "$2")"
case "$build_root/" in */_build/*) ;; *) echo 'Generated output must be under _build' >&2; exit 2 ;; esac
cargo fmt --manifest-path "$plugin_root/Cargo.toml" -- --check
cargo --config "$sdk_config" test --manifest-path "$plugin_root/Cargo.toml" \
    --target-dir "$build_root/target" --locked --offline --all-targets
cargo clippy --config "$sdk_config" --manifest-path "$plugin_root/Cargo.toml" \
    --target-dir "$build_root/target" --locked --offline --all-targets -- -D warnings
bash "$plugin_root/scripts/build.sh" "$sdk_config" "$build_root"
