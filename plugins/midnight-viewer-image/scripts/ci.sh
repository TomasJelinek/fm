#!/usr/bin/env bash
set -euo pipefail
plugin_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
sdk_config="$(realpath -- "$1")"
build_root="$(realpath -m -- "$2")"
case "$build_root/" in */_build/*) ;; *) echo 'Generated output must be under _build' >&2; exit 2 ;; esac
decoder_root="$(cd -- "$plugin_root/../image-decoder" && pwd)"
native_offline=()
[[ ${MIDNIGHT_IMAGE_NATIVE_OFFLINE:-0} != 1 ]] || native_offline+=(--offline)
native_jobs=${CARGO_BUILD_JOBS:-2}
source "$plugin_root/../image-decoder/native/bootstrap-linux.sh" "$build_root/native-tools" "$native_jobs" "${native_offline[@]}"
source "$plugin_root/../image-decoder/native/build-linux.sh" "$build_root/native-dav1d" "$native_jobs" "${native_offline[@]}"
# The native .pc prefix is absolute; an unrelated GUI sysroot must not rewrite it.
unset PKG_CONFIG_SYSROOT_DIR
for project in "$decoder_root" "$plugin_root"; do
    cargo fmt --manifest-path "$project/Cargo.toml" -- --check
    cargo --config "$sdk_config" test --manifest-path "$project/Cargo.toml" \
        --target-dir "$build_root/target" --locked --offline --all-targets
    cargo clippy --config "$sdk_config" --manifest-path "$project/Cargo.toml" \
        --target-dir "$build_root/target" --locked --offline --all-targets -- -D warnings
done
bash "$plugin_root/scripts/build.sh" "$sdk_config" "$build_root"
