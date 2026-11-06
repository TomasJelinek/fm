#!/usr/bin/env bash
set -euo pipefail
if [[ $# -lt 2 || $# -gt 3 ]]; then
    echo 'Usage: build.sh SDK_CONFIG BUILD_ROOT [OUTPUT_PACKAGE_DIRECTORY]' >&2
    exit 2
fi
plugin_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
sdk_config="$(realpath -- "$1")"
build_root="$(realpath -m -- "$2")"
output="${3:-$build_root/package/midnight-viewer-basic}"
native_target="$(rustc -vV | sed -n 's/^host: //p')"
case "$native_target" in
    *-linux-*) library=libmidnight_viewer_basic.so ;;
    *) echo "Unsupported Linux target: $native_target" >&2; exit 2 ;;
esac
mkdir -p -- "$build_root" "$output"
cargo --config "$sdk_config" build --manifest-path "$plugin_root/Cargo.toml" \
    --target-dir "$build_root/target" --target "$native_target" --release --locked --offline
cp -- "$build_root/target/$native_target/release/$library" "$output/$library"
sed -e "s/^entry_library = .*/entry_library = \"$library\"/" \
    -e "s/^target = .*/target = \"$native_target\"/" "$plugin_root/plugin.toml" > "$output/plugin.toml"
cp -- "$plugin_root/LICENSE" "$plugin_root/UNICODE-LICENSE.txt" \
    "$plugin_root/README.md" "$plugin_root/THIRD-PARTY.md" "$output/"
mkdir -p -- "$output/notices"
cp -R -- "$plugin_root/notices/." "$output/notices/"
echo "Basic viewer package: $output"
