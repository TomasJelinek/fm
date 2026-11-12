#!/usr/bin/env bash
set -euo pipefail
if [[ $# -lt 2 || $# -gt 3 ]]; then
    echo 'Usage: build.sh SDK_CONFIG BUILD_ROOT [OUTPUT_PACKAGE_DIRECTORY]' >&2
    exit 2
fi
plugin_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
sdk_config="$(realpath -- "$1")"
build_root="$(realpath -m -- "$2")"
output="$(realpath -m -- "${3:-$build_root/package/midnight-viewer-image}")"
for path in "$build_root" "$output"; do
    case "$path/" in */_build/*) ;; *) echo 'Generated output must be under _build' >&2; exit 2 ;; esac
done
native_target="$(rustc -vV | sed -n 's/^host: //p')"
case "$native_target" in
    x86_64-unknown-linux-gnu) ;;
    *) echo "Image packages currently require Linux x86-64; unqualified target: $native_target" >&2; exit 2 ;;
esac
mkdir -p -- "$build_root" "$output"
native_offline=()
[[ ${MIDNIGHT_IMAGE_NATIVE_OFFLINE:-0} != 1 ]] || native_offline+=(--offline)
native_jobs=${CARGO_BUILD_JOBS:-2}
source "$plugin_root/../image-decoder/native/bootstrap-linux.sh" "$build_root/native-tools" "$native_jobs" "${native_offline[@]}"
source "$plugin_root/../image-decoder/native/build-linux.sh" "$build_root/native-dav1d" "$native_jobs" "${native_offline[@]}"
# The native .pc prefix is absolute; an unrelated GUI sysroot must not rewrite it.
unset PKG_CONFIG_SYSROOT_DIR
profile=${MIDNIGHT_IMAGE_VIEWER_PROFILE:-release}
profile_options=()
case "$profile" in debug) ;; release) profile_options+=(--release) ;; *) echo 'Expected debug or release' >&2; exit 2 ;; esac
cargo --config "$sdk_config" build --manifest-path "$plugin_root/Cargo.toml" \
    --target-dir "$build_root/target" --target "$native_target" "${profile_options[@]}" --locked --offline
cp -- "$build_root/target/$native_target/$profile/midnight-viewer-image" "$output/midnight-viewer-image"
sed -e "s/\"target\": \"[^\"]*\"/\"target\": \"$native_target\"/" \
    -e 's/"executable": "[^"]*"/"executable": "midnight-viewer-image"/' \
    "$plugin_root/image-viewer.json" > "$output/image-viewer.json"
cp -- "$plugin_root/LICENSE" "$plugin_root/README.md" "$plugin_root/THIRD-PARTY.md" "$output/"
mkdir -p -- "$output/notices"
cp -R -- "$plugin_root/../image-decoder/notices/." "$output/notices/"
echo "Image viewer package: $output"
