#!/usr/bin/env bash
set -euo pipefail
if [[ $# -gt 4 ]]; then
    echo 'Usage: build-image-viewer.sh [debug|release] [WORKER_SDK_CONFIG] [APP_DIRECTORY] [image|pdf]' >&2
    exit 2
fi
profile=${1:-debug}
sdk_config=${2:-${MIDNIGHT_IMAGE_VIEWER_SDK_CONFIG:-}}
viewer=${4:-image}
case "$viewer" in image|pdf) ;; *) echo 'Expected image or pdf' >&2; exit 2 ;; esac
case "$profile" in debug|release) ;; *) echo 'Expected debug or release' >&2; exit 2 ;; esac
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$project_root"
build_root="$project_root/_build/$viewer-viewer"
mkdir -p -- "$build_root" "$project_root/_build/tmp"
export TMPDIR="$project_root/_build/tmp"
target_root=${CARGO_TARGET_DIR:-"$project_root/_build"}
app_directory=${3:-"$target_root/$profile"}
fetch_options=(--locked)
[[ "${MIDNIGHT_VIEWERS_OFFLINE:-0}" == 1 ]] && fetch_options+=(--offline)
if [[ -z "$sdk_config" ]]; then
    stamp=$(find api/tools/package worker-api thumbnail-api filesystem-api image-viewer-api -type f -print0 |
        sort -z | xargs -0 sha256sum | sha256sum | cut -d' ' -f1)
    if [[ -f "$build_root/linux-sdk-stamp" && -f "$build_root/linux-sdk-config" ]] &&
        [[ "$(cat "$build_root/linux-sdk-stamp")" == "$stamp" ]]; then
        sdk_config=$(cat "$build_root/linux-sdk-config")
        [[ -f "$sdk_config" ]] || sdk_config=''
    fi
    if [[ -z "$sdk_config" ]]; then
        for workspace in api worker-api thumbnail-api filesystem-api image-viewer-api; do
            cargo fetch --manifest-path "$workspace/Cargo.toml" "${fetch_options[@]}"
        done
        output=$(CARGO_TARGET_DIR="$build_root/linux-sdk-target" cargo run --manifest-path api/Cargo.toml \
            -p midnight-api-package --bin worker-packages --locked --offline -- "$project_root" "$build_root/releases")
        bundle=$(printf '%s\n' "$output" | sed -n 's/^Public worker API\/SDK release bundle: //p' | tail -n 1)
        [[ -n "$bundle" ]] || { echo 'Worker SDK packager did not report output' >&2; exit 1; }
        sdk_config="$bundle/.cargo/registry.toml"
        printf '%s\n' "$sdk_config" > "$build_root/linux-sdk-config"
        printf '%s\n' "$stamp" > "$build_root/linux-sdk-stamp"
    fi
fi
sdk_config=$(realpath -- "$sdk_config")
native_target=$(rustc -vV | sed -n 's/^host: //p')
plugin_root="$project_root/plugins/midnight-viewer-$viewer"
plugin_build="$build_root/$native_target/plugin"
cargo --config "$sdk_config" fetch --manifest-path "$plugin_root/Cargo.toml" "${fetch_options[@]}"
if [[ "${MIDNIGHT_TEST_VIEWERS:-0}" == 1 ]]; then
    bash "$plugin_root/scripts/ci.sh" "$sdk_config" "$plugin_build"
fi
MIDNIGHT_IMAGE_VIEWER_PROFILE="$profile" MIDNIGHT_PDF_VIEWER_PROFILE="$profile" \
    bash "$plugin_root/scripts/build.sh" "$sdk_config" \
    "$plugin_build" "$app_directory/viewers/midnight-viewer-$viewer"
