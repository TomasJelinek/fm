#!/usr/bin/env bash
set -euo pipefail
if [[ $# -lt 2 || $# -gt 3 ]]; then
    echo 'Usage: build.sh SDK_CONFIG BUILD_ROOT [OUTPUT_PACKAGE_DIRECTORY]' >&2
    exit 2
fi
plugin_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
sdk_config="$(realpath -- "$1")"
build_root="$(realpath -ms -- "$2")"
output="$(realpath -ms -- "${3:-$build_root/package/midnight-viewer-pdf}")"
# Generated output lies strictly below _build: for a path inside this plugin, or inside
# the repository that holds it in plugins/NAME, their own _build; otherwise the first
# _build of the path (a custom directory). That _build and every folder below it must
# not be a symlink. The checkout and the folders above it may be links (Fedora
# Silverblue's /home -> /var/home, a linked parent folder): a checkout is recognised
# however the path spells it, at its shallowest place, so a link back to it is inspected.
generated_output_problem() {
    local path=$1 ancestors=() index top anchor=0 repository=
    [[ $path == /* ]] || { printf 'must be an absolute path: %s\n' "$1"; return; }
    [[ ${plugin_root%/*} != */plugins ]] || repository=${plugin_root%/*/*}
    while [[ $path != / ]]; do ancestors+=("$path"); path=${path%/*}; path=${path:-/}; done
    top=${#ancestors[@]}
    for (( index=top-1; index>=0; index-- )); do
        if [[ ${ancestors[index]} -ef $plugin_root || ( -n $repository && ${ancestors[index]} -ef $repository ) ]]; then
            top=$index
            break
        fi
    done
    if (( top < ${#ancestors[@]} )); then
        if (( top >= 2 )) && [[ ${ancestors[top-1]##*/} == _build ]]; then
            anchor=$((top - 1))
        elif (( top >= 4 )) && [[ -n $repository && ${ancestors[top]} -ef $repository && ${ancestors[top-1]##*/} == plugins &&
                ${ancestors[top-2]} -ef $plugin_root && ${ancestors[top-3]##*/} == _build ]]; then
            anchor=$((top - 3))
        fi
    else
        for (( index=${#ancestors[@]}-1; index>0; index-- )); do
            if [[ ${ancestors[index]##*/} == _build ]]; then anchor=$index; break; fi
        done
    fi
    if (( anchor == 0 )); then printf 'must be under _build: %s\n' "$1"; return; fi
    for (( index=0; index<=anchor; index++ )); do
        if [[ -L ${ancestors[index]} ]]; then printf 'must not traverse symlinks: %s\n' "${ancestors[index]}"; return; fi
    done
}
for path in "$build_root" "$output"; do
    problem=$(generated_output_problem "$path")
    [[ -z $problem ]] || { echo "Generated output $problem" >&2; exit 2; }
done
native_target="$(rustc -vV | sed -n 's/^host: //p')"
case "$native_target" in
    x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu) ;;
    *) echo "Unsupported Linux target: $native_target" >&2; exit 2 ;;
esac
mkdir -p -- "$build_root" "$output"
profile=${MIDNIGHT_PDF_VIEWER_PROFILE:-release}
profile_options=()
case "$profile" in debug) ;; release) profile_options+=(--release) ;; *) echo 'Expected debug or release' >&2; exit 2 ;; esac
cargo --config "$sdk_config" build --manifest-path "$plugin_root/Cargo.toml" \
    --target-dir "$build_root/target" --target "$native_target" "${profile_options[@]}" --locked --offline
# A running worker keeps its executable busy: rename a complete new inode into place.
cp -- "$build_root/target/$native_target/$profile/midnight-viewer-pdf" "$output/.midnight-viewer-pdf.new"
mv -f -- "$output/.midnight-viewer-pdf.new" "$output/midnight-viewer-pdf"
sed -e "s/\"target\": \"[^\"]*\"/\"target\": \"$native_target\"/" \
    -e 's/"executable": "[^"]*"/"executable": "midnight-viewer-pdf"/' \
    "$plugin_root/image-viewer.json" > "$output/image-viewer.json"
cp -- "$plugin_root/LICENSE" "$plugin_root/README.md" "$plugin_root/THIRD-PARTY.md" "$output/"
# Replace the generated notices so a removed dependency's notice cannot survive.
[[ ! -L $output/notices ]] || { echo 'Package notices must not be a link' >&2; exit 2; }
rm -rf -- "$output/notices"
mkdir -- "$output/notices"
cp -R -- "$plugin_root/notices/." "$output/notices/"
echo "PDF viewer package: $output"
