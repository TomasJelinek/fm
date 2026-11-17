#!/usr/bin/env bash
set -euo pipefail
if [[ $# -lt 2 || $# -gt 4 ]]; then
    echo 'Usage: build.sh SDK_CONFIG BUILD_ROOT [OUTPUT_PACKAGE_DIRECTORY] [debug|release]' >&2
    exit 2
fi
profile=${4:-debug}
profile_options=()
case "$profile" in
    debug) ;;
    release) profile_options+=(--release) ;;
    *) echo 'Expected debug or release' >&2; exit 2 ;;
esac
plugin_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
build_root="$(realpath -m -- "$2")"
config="$(python3 "$plugin_root/scripts/prepare-sdk.py" "$1" "$build_root")"
output="${3:-$build_root/package/midnight-viewer-markdown}"
native_target="$(rustc -vV | sed -n 's/^host: //p')"
case "$native_target" in
    *-linux-*) library=libmidnight_viewer_markdown.so ;;
    *) echo "Unsupported Linux target: $native_target" >&2; exit 2 ;;
esac
cargo --config "$config" build --manifest-path "$plugin_root/Cargo.toml" \
    --target-dir "$build_root/target" --target "$native_target" "${profile_options[@]}" --locked --offline
mkdir -p -- "$output"
# A running host may have the old library mapped: rename a complete new inode
# into place instead of truncating the mapped one.
cp -- "$build_root/target/$native_target/$profile/$library" "$output/.$library.new"
mv -f -- "$output/.$library.new" "$output/$library"
sed -e "s/^entry_library = .*/entry_library = \"$library\"/" \
    -e "s/^target = .*/target = \"$native_target\"/" "$plugin_root/plugin.toml" > "$output/plugin.toml"
cp -- "$plugin_root/LICENSE" "$plugin_root/README.md" "$plugin_root/THIRD-PARTY.md" "$output/"
# Replace only notices that this script generated (they carry its marker) or that lie
# in a _build output tree, so a removed dependency's notice cannot survive a rebuild.
# The marker is never removed: an interrupted build is completed by the next one.
notices="$output/notices"
marker_text='midnight-viewer-markdown notices'
if [[ -e $notices || -L $notices ]]; then
    [[ -d $notices && ! -L $notices ]] || { echo 'Package notices must be a directory, not a link' >&2; exit 2; }
    if [[ -n $(find "$notices" -mindepth 1 -type l -print -quit) ]]; then
        echo "Package notices must not contain links: $notices" >&2
        exit 2
    fi
    if [[ ! -f $notices/.midnight-generated || "$(cat -- "$notices/.midnight-generated")" != "$marker_text" ]]; then
        case "$(realpath -- "$output")/" in
            */_build/*) ;;
            *) echo "Refusing to replace notices that this script did not generate: $notices" >&2; exit 2 ;;
        esac
    fi
    find "$notices" -mindepth 1 -maxdepth 1 ! -name .midnight-generated -exec rm -rf -- {} +
else
    mkdir -- "$notices"
fi
printf '%s\n' "$marker_text" > "$notices/.midnight-generated"
cp -R -- "$plugin_root/notices/." "$notices/"
echo "Viewer package: $output"
