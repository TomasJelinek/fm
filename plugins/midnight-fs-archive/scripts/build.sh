#!/usr/bin/env bash
set -euo pipefail
if [[ $# -lt 2 ]]; then
    echo 'Usage: build.sh SDK_CONFIG BUILD_ROOT [OUTPUT_DIRECTORY] [debug|release] [--test] [--offline]' >&2
    exit 2
fi
sdk_config=$(realpath -- "$1")
build_root=$(realpath -ms -- "$2")
package=${3:-}
profile=${4:-debug}
case "$profile" in debug|release) ;; *) echo 'Expected debug or release' >&2; exit 2 ;; esac
options=()
run_tests=${MIDNIGHT_TEST_ARCHIVE_FILESYSTEM:-0}
if [[ $# -gt 4 ]]; then
    for argument in "${@:5}"; do
        case "$argument" in
            --test) run_tests=1 ;;
            --offline) options+=(--offline) ;;
            *) echo "Unknown archive build option: $argument" >&2; exit 2 ;;
        esac
    done
fi
build_options=("${options[@]}")
if [[ $profile == release ]]; then build_options+=(--release); fi
plugin_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
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
problem=$(generated_output_problem "$build_root")
[[ -z $problem ]] || { echo "Build output $problem" >&2; exit 1; }
export CARGO_TARGET_DIR="$build_root" CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
if [[ $run_tests == 1 ]]; then
    # Parser tests keep debug assertions and overflow checks for every package profile.
    cargo test --manifest-path "$plugin_root/Cargo.toml" --locked --config "$sdk_config" "${options[@]}"
fi
cargo build --manifest-path "$plugin_root/Cargo.toml" --locked --config "$sdk_config" "${build_options[@]}"
if [[ -n "$package" ]]; then
    package=$(realpath -ms -- "$package")
    problem=$(generated_output_problem "$package")
    [[ -z $problem ]] || { echo "Package staging $problem" >&2; exit 1; }
    mkdir -p -- "$package"
    # A running worker keeps its executable busy: rename a complete new inode into place.
    cp -- "$build_root/$profile/midnight-fs-archive" "$package/.midnight-fs-archive.new"
    mv -f -- "$package/.midnight-fs-archive.new" "$package/midnight-fs-archive"
    cp -- "$plugin_root/THIRD-PARTY.md" "$package/"
    target=$(rustc -vV | sed -n 's/^host: //p')
    cat > "$package/plugin.toml" <<EOF
schema_version = 1
id = "midnight.fs.archive"
kind = "filesystem-worker"
version = "0.2.0"
entry_executable = "midnight-fs-archive"
target = "$target"
protocol = 3
stable_entry_ids = true
formats = ["zip", "tar", "gzip", "tar-gzip", "7z", "xz", "tar-xz", "bzip2", "tar-bzip2", "zstd", "tar-zstd"]
EOF
    python3 -B "$plugin_root/scripts/notices.py" --config "$sdk_config" --output "$package/licenses"
fi
