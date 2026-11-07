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
    x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu) ;;
    *) echo "Unsupported Linux target: $native_target" >&2; exit 2 ;;
esac
mkdir -p -- "$build_root" "$output"
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
cp -R -- "$plugin_root/notices/." "$output/notices/"
echo "Image viewer package: $output"
