#!/usr/bin/env bash
set -euo pipefail
profile=${1:-debug}
sdk_config=${2:-${MIDNIGHT_VIEWER_SDK_CONFIG:-}}
if [[ $# -gt 3 ]]; then printf '%s\n' 'Usage: build-viewers.sh [debug|release] [SDK_CONFIG] [APP_DIRECTORY]' >&2; exit 2; fi
case "$profile" in debug|release) ;; *) printf '%s\n' 'Expected debug or release' >&2; exit 2;; esac
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$project_root"
build_root="$project_root/_build/viewers"
mkdir -p -- "$build_root" "$project_root/_build/tmp"
export TMPDIR="$project_root/_build/tmp"
target_root=${CARGO_TARGET_DIR:-"$project_root/_build"}
app_directory=${3:-"$target_root/$profile"}
fetch_options=(--locked)
[[ "${MIDNIGHT_VIEWERS_OFFLINE:-0}" == 1 ]] && fetch_options+=(--offline)
if [[ -z "$sdk_config" ]]; then
    stamp=$(find api -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum | cut -d' ' -f1)
    if [[ -f "$build_root/sdk-stamp" && -f "$build_root/sdk-config" ]] &&
        [[ "$(cat "$build_root/sdk-stamp")" == "$stamp" ]]; then
        sdk_config=$(cat "$build_root/sdk-config")
        [[ -f "$sdk_config" ]] || sdk_config=''
    fi
    if [[ -z "$sdk_config" ]]; then
        cargo fetch --manifest-path api/Cargo.toml "${fetch_options[@]}"
        output=$(CARGO_TARGET_DIR="$build_root/sdk-target" bash api/scripts/package.sh)
        bundle=$(printf '%s\n' "$output" | sed -n 's/^Public API\/SDK release bundle: //p' | tail -n 1)
        [[ -n "$bundle" ]] || { printf '%s\n' 'SDK packager did not report output' >&2; exit 1; }
        sdk_config="$bundle/.cargo/config.toml"
        printf '%s\n' "$sdk_config" > "$build_root/sdk-config"
        printf '%s\n' "$stamp" > "$build_root/sdk-stamp"
    fi
fi
sdk_config=$(realpath -- "$sdk_config")
native_target=$(rustc -vV | sed -n 's/^host: //p')
[[ -n "$native_target" ]] || { printf '%s\n' 'Cannot determine native plugin target' >&2; exit 1; }
for name in basic source markdown; do
    plugin_root="$project_root/plugins/midnight-viewer-$name"
    plugin_build="$build_root/$native_target/$name"
    config="$sdk_config"
    if [[ -f "$plugin_root/scripts/prepare-sdk.sh" ]]; then
        config=$(bash "$plugin_root/scripts/prepare-sdk.sh" "$sdk_config" "$plugin_build")
    elif [[ -f "$plugin_root/scripts/prepare-sdk.py" ]]; then
        config=$(python3 -B "$plugin_root/scripts/prepare-sdk.py" "$sdk_config" "$plugin_build")
    fi
    cargo --config "$config" fetch --manifest-path "$plugin_root/Cargo.toml" "${fetch_options[@]}"
    if [[ "${MIDNIGHT_TEST_VIEWERS:-0}" == 1 ]]; then
        bash "$plugin_root/scripts/ci.sh" "$sdk_config" "$plugin_build"
    fi
    bash "$plugin_root/scripts/build.sh" "$sdk_config" "$plugin_build" "$app_directory/viewers/midnight-viewer-$name"
done
bash "$project_root/scripts/build-image-viewer.sh" "$profile" "${MIDNIGHT_IMAGE_VIEWER_SDK_CONFIG:-}" "$app_directory"
