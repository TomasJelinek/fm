#!/usr/bin/env bash
# Normal builds always install both independently built thumbnail providers.
set -euo pipefail
if [[ $# -gt 3 ]]; then
    echo 'Usage: build-default-thumbnailers.sh [debug|release] [WORKER_SDK_CONFIG] [APP_DIRECTORY]' >&2
    exit 2
fi
profile=${1:-debug}
sdk_config=${2:-${MIDNIGHT_THUMBNAIL_SDK_CONFIG:-}}
case "$profile" in debug|release) ;; *) echo 'Expected debug or release' >&2; exit 2 ;; esac
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$project_root"
build_root="$project_root/_build/thumbnailers"
mkdir -p -- "$build_root" "$project_root/_build/tmp"
export TMPDIR="$project_root/_build/tmp"
target_root=${CARGO_TARGET_DIR:-"$project_root/_build"}
app_directory=${3:-"$target_root/$profile"}
fetch_options=(--locked)
native_options=()
if [[ ${MIDNIGHT_THUMBNAILERS_OFFLINE:-${MIDNIGHT_VIEWERS_OFFLINE:-0}} == 1 ]]; then
    fetch_options+=(--offline)
    native_options+=(--offline)
    export MIDNIGHT_IMAGE_NATIVE_OFFLINE=1
fi
if [[ -z "$sdk_config" ]]; then
    stamp=$(find api/tools/package worker-api thumbnail-api filesystem-api image-viewer-api -type f -print0 |
        sort -z | xargs -0 sha256sum | sha256sum | cut -d' ' -f1)
    # Image viewers already package this same public worker SDK in normal builds.
    for cache in "$build_root" "$project_root/_build/image-viewer"; do
        if [[ -f "$cache/linux-sdk-stamp" && -f "$cache/linux-sdk-config" ]] &&
            [[ $(cat "$cache/linux-sdk-stamp") == "$stamp" ]]; then
            candidate=$(cat "$cache/linux-sdk-config")
            if [[ -f "$candidate" ]]; then sdk_config=$candidate; break; fi
        fi
    done
    if [[ -z "$sdk_config" ]]; then
        for workspace in api worker-api thumbnail-api filesystem-api image-viewer-api; do
            cargo fetch --manifest-path "$workspace/Cargo.toml" "${fetch_options[@]}"
        done
        output=$(CARGO_TARGET_DIR="$build_root/linux-sdk-target" cargo run --manifest-path api/Cargo.toml \
            -p midnight-api-package --bin worker-packages --locked --offline -- "$project_root" "$build_root/releases")
        bundle=$(printf '%s\n' "$output" | sed -n 's/^Public worker API\/SDK release bundle: //p' | tail -n 1)
        [[ -n "$bundle" ]] || { echo 'Worker SDK packager did not report output' >&2; exit 1; }
        sdk_config="$bundle/.cargo/registry.toml"
        [[ -f "$sdk_config" ]] || { echo 'Worker SDK configuration is missing' >&2; exit 1; }
        printf '%s\n' "$sdk_config" > "$build_root/linux-sdk-config"
        printf '%s\n' "$stamp" > "$build_root/linux-sdk-stamp"
    fi
fi
sdk_config=$(realpath -- "$sdk_config")
for name in image video; do
    cargo --config "$sdk_config" fetch \
        --manifest-path "$project_root/plugins/midnight-thumbnail-$name/Cargo.toml" "${fetch_options[@]}"
done
python3 -B "$project_root/scripts/prepare-thumbnail-ffmpeg.py" \
    --build-root "$build_root/ffmpeg" "${native_options[@]}" > "$build_root/native-state.json"
# Decode data only; never evaluate paths as shell commands.
native_paths=$(python3 -B - "$build_root/native-state.json" <<'PY'
import json
from pathlib import Path
import sys
state = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
for key in ("prefix", "source_archive", "libclang_path"):
    value = state.get(key)
    if not isinstance(value, str) or not value or "\n" in value or "\r" in value:
        raise SystemExit(f"Missing or invalid native thumbnail build path: {key}")
    print(value)
PY
)
mapfile -t native_paths <<< "$native_paths"
python3 -B "$project_root/scripts/thumbnail-notices.py" --config "$sdk_config" --output "$build_root/notices"
exec bash "$project_root/scripts/build-thumbnailers.sh" "$sdk_config" "$app_directory/thumbnailers" \
    "$build_root/plugins" "$build_root/notices" "${native_paths[@]}" "$profile"
