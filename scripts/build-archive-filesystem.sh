#!/usr/bin/env bash
# Normal builds install the independent archive worker beside the native providers.
set -euo pipefail
if [[ $# -gt 3 ]]; then
    echo 'Usage: build-archive-filesystem.sh [debug|release] [WORKER_SDK_CONFIG] [APP_DIRECTORY]' >&2
    exit 2
fi
profile=${1:-debug}
sdk_config=${2:-${MIDNIGHT_ARCHIVE_SDK_CONFIG:-}}
case "$profile" in debug|release) ;; *) echo 'Expected debug or release' >&2; exit 2 ;; esac
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$project_root"
build_root="$project_root/_build/archive-filesystems/linux"
target_root=${CARGO_TARGET_DIR:-"$project_root/_build"}
app_directory=${3:-"$target_root/$profile"}
if [[ -z "$sdk_config" ]]; then
    stamp=$(find api/tools/package worker-api thumbnail-api filesystem-api image-viewer-api -type f -print0 |
        sort -z | xargs -0 sha256sum | sha256sum | cut -d' ' -f1)
    for cache in "$project_root/_build/thumbnailers" "$project_root/_build/image-viewer"; do
        if [[ -f "$cache/linux-sdk-stamp" && -f "$cache/linux-sdk-config" ]] &&
            [[ $(cat "$cache/linux-sdk-stamp") == "$stamp" ]]; then
            candidate=$(cat "$cache/linux-sdk-config")
            if [[ -f "$candidate" ]]; then sdk_config=$candidate; break; fi
        fi
    done
    if [[ -z "$sdk_config" ]]; then
        echo 'A current public worker SDK is required. Run ./build.sh debug to prepare it through the viewer/thumbnail builds, or set MIDNIGHT_ARCHIVE_SDK_CONFIG to its .cargo/registry.toml.' >&2
        exit 2
    fi
fi
sdk_config=$(realpath -- "$sdk_config")
options=()
if [[ ${MIDNIGHT_ARCHIVE_OFFLINE:-${MIDNIGHT_VIEWERS_OFFLINE:-0}} == 1 ]]; then
    options+=(--offline)
fi
mkdir -p -- "$build_root" "$project_root/_build/tmp"
export TMPDIR="$project_root/_build/tmp"
cargo --config "$sdk_config" fetch --manifest-path "$project_root/plugins/midnight-fs-archive/Cargo.toml" \
    --locked "${options[@]}"
exec bash "$project_root/plugins/midnight-fs-archive/scripts/build.sh" "$sdk_config" "$build_root" \
    "$app_directory/filesystems/midnight.fs.archive" "$profile" "${options[@]}"
