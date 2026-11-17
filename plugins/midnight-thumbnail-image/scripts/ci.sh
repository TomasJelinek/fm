#!/usr/bin/env bash
# Independent checks of the image thumbnailer: the pinned native dav1d build, then
# formatting, tests and strict Clippy, and an optimized build.
set -euo pipefail
if [[ $# -ne 2 ]]; then
    echo 'Usage: ci.sh SDK_CONFIG BUILD_ROOT' >&2
    exit 2
fi
plugin_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
sdk_config="$(realpath -- "$1")"
build_root="$(realpath -m -- "$2")"
case "$build_root/" in */_build/*) ;; *) echo 'Generated output must be under _build' >&2; exit 2 ;; esac
native_offline=()
[[ ${MIDNIGHT_IMAGE_NATIVE_OFFLINE:-0} != 1 ]] || native_offline+=(--offline)
native_jobs=${CARGO_BUILD_JOBS:-2}
source "$plugin_root/../image-decoder/native/bootstrap-linux.sh" "$build_root/native-tools" "$native_jobs" "${native_offline[@]}"
source "$plugin_root/../image-decoder/native/build-linux.sh" "$build_root/native-dav1d" "$native_jobs" "${native_offline[@]}"
# The native .pc prefix is absolute; an unrelated GUI sysroot must not rewrite it.
unset PKG_CONFIG_SYSROOT_DIR
manifest="$plugin_root/Cargo.toml"
common=(--config "$sdk_config" --manifest-path "$manifest" --target-dir "$build_root/target" --locked --offline)
cargo fmt --manifest-path "$manifest" -- --check
cargo test "${common[@]}" --all-targets
cargo clippy "${common[@]}" --all-targets -- -D warnings
cargo build "${common[@]}" --release
