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
sdk_config="$(realpath -- "$1")"
build_root="$(realpath -ms -- "$2")"
output="$(realpath -ms -- "${3:-$build_root/package/midnight-viewer-basic}")"
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
native_target="$(rustc -vV | sed -n 's/^host: //p')"
case "$native_target" in
    *-linux-*) library=libmidnight_viewer_basic.so ;;
    *) echo "Unsupported Linux target: $native_target" >&2; exit 2 ;;
esac
mkdir -p -- "$build_root" "$output"
cargo --config "$sdk_config" build --manifest-path "$plugin_root/Cargo.toml" \
    --target-dir "$build_root/target" --target "$native_target" "${profile_options[@]}" --locked --offline
# A running host may have the old library mapped: rename a complete new inode
# into place instead of truncating the mapped one.
cp -- "$build_root/target/$native_target/$profile/$library" "$output/.$library.new"
mv -f -- "$output/.$library.new" "$output/$library"
sed -e "s/^entry_library = .*/entry_library = \"$library\"/" \
    -e "s/^target = .*/target = \"$native_target\"/" "$plugin_root/plugin.toml" > "$output/plugin.toml"
cp -- "$plugin_root/LICENSE" "$plugin_root/UNICODE-LICENSE.txt" \
    "$plugin_root/README.md" "$plugin_root/THIRD-PARTY.md" "$output/"
# Replace only notices that this script generated (they carry its marker) or that lie
# in a _build output tree, so a removed dependency's notice cannot survive a rebuild.
# The marker is never removed: an interrupted build is completed by the next one.
notices="$output/notices"
marker_text='midnight-viewer-basic notices'
if [[ -e $notices || -L $notices ]]; then
    [[ -d $notices && ! -L $notices ]] || { echo 'Package notices must be a directory, not a link' >&2; exit 2; }
    if [[ -n $(find "$notices" -mindepth 1 -type l -print -quit) ]]; then
        echo "Package notices must not contain links: $notices" >&2
        exit 2
    fi
    if [[ ! -f $notices/.midnight-generated || "$(cat -- "$notices/.midnight-generated")" != "$marker_text" ]]; then
        if [[ -n $(generated_output_problem "$output") ]]; then
            echo "Refusing to replace notices that this script did not generate: $notices" >&2
            exit 2
        fi
    fi
    find "$notices" -mindepth 1 -maxdepth 1 ! -name .midnight-generated -exec rm -rf -- {} +
else
    mkdir -- "$notices"
fi
printf '%s\n' "$marker_text" > "$notices/.midnight-generated"
cp -R -- "$plugin_root/notices/." "$notices/"
# Markdown names in a package are lowercase, with '-' for '_' (README.md is readme.md);
# the fresh copy replaces an earlier build's.
while IFS= read -r -d '' file; do
    name=${file##*/}; name=${name,,}; name=${name//_/-}
    [[ $name == "${file##*/}" ]] || mv -f -- "$file" "${file%/*}/$name"
done < <(find "$output" -type f -iname '*.md' -print0)
echo "Basic viewer package: $output"
