#!/usr/bin/env bash
# HTML is a required independent runtime; missing SDK/native inputs fail the build.
set -euo pipefail
if [[ $# -gt 3 ]]; then
    echo 'Usage: build-html-viewer.sh [debug|release] [WEB_SDK_CONFIG] [APP_DIRECTORY]' >&2
    exit 2
fi
profile=${1:-debug}
sdk_config=${2:-${MIDNIGHT_HTML_SDK_CONFIG:-}}
case "$profile" in debug|release) ;; *) echo 'Expected debug or release' >&2; exit 2 ;; esac
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$project_root"
build_root="$project_root/_build/html-viewer"
target_root=${CARGO_TARGET_DIR:-"$project_root/_build"}
app_directory=$(realpath -ms -- "${3:-$target_root/$profile}")
destination="$app_directory/viewers/midnight-viewer-html"
for target in "$build_root" "$destination"; do
    case "$target" in /*/_build/*) ;; *) echo 'Generated HTML output must be under _build' >&2; exit 2 ;; esac
    parent=$target
    while [[ $parent != / ]]; do
        [[ ! -L $parent ]] || { echo 'Generated HTML output must not traverse links' >&2; exit 2; }
        parent=$(dirname -- "$parent")
    done
done
mkdir -p -- "$build_root" "$project_root/_build/tmp"
export TMPDIR="$project_root/_build/tmp"
options=()
[[ ${MIDNIGHT_VIEWERS_OFFLINE:-0} != 1 ]] || options+=(--offline)
if [[ -z "$sdk_config" ]]; then
    stamp=$(find web-viewer-api -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum | cut -d' ' -f1)
    if [[ -f "$build_root/linux-web-sdk-stamp" && -f "$build_root/linux-web-sdk-config" ]] &&
        [[ $(cat "$build_root/linux-web-sdk-stamp") == "$stamp" ]]; then
        candidate=$(cat "$build_root/linux-web-sdk-config")
        [[ ! -f "$candidate" ]] || sdk_config=$candidate
    fi
    if [[ -z "$sdk_config" ]]; then
        cargo fetch --manifest-path "$project_root/web-viewer-api/Cargo.toml" --locked "${options[@]}"
        sdk_config=$(python3 -B "$project_root/web-viewer-api/scripts/package.py")
        [[ -f "$sdk_config" ]] || { echo 'Public web SDK packager did not report a configuration' >&2; exit 1; }
        printf '%s\n' "$sdk_config" > "$build_root/linux-web-sdk-config"
        printf '%s\n' "$stamp" > "$build_root/linux-web-sdk-stamp"
    fi
fi
sdk_config=$(realpath -- "$sdk_config")
native_state=$(python3 -B "$project_root/scripts/prepare-html-cef.py" \
    --output "$build_root/cef-sdk/linux" "${options[@]}")
IFS= read -r -d '' CEF_PATH < <(python3 -B - "$native_state" <<'PY'
import json, os, sys
path = json.loads(sys.argv[1])["cef_path"]
if not isinstance(path, str) or not path or "\0" in path:
    raise SystemExit("Verified CEF preparation returned an invalid path")
sys.stdout.buffer.write(os.fsencode(path) + b"\0")
PY
)
export CEF_PATH
cargo --config "$sdk_config" fetch --manifest-path "$project_root/plugins/midnight-viewer-html/Cargo.toml" \
    --locked "${options[@]}"
mkdir -p -- "$app_directory/viewers"
staging=$(mktemp -d "$app_directory/.html-build.XXXXXXXX")
clean_staging=1
trap 'if [[ $clean_staging == 1 ]]; then rm -rf -- "$staging"; fi' EXIT
MIDNIGHT_HTML_TARGET_DIR="$build_root/provider-target/linux" \
    bash "$project_root/plugins/midnight-viewer-html/scripts/build-linux.sh" \
    "$sdk_config" "$profile" "$staging/package"
[[ -f "$staging/package/runtime-files.json" && -f "$staging/package/web-viewer.json" ]] || {
    echo 'HTML package is missing its runtime inventory or viewer manifest' >&2; exit 1;
}
if [[ -e "$destination" ]]; then mv -- "$destination" "$staging/previous"; fi
if mv -- "$staging/package" "$destination"; then
    printf 'HTML viewer package: %s\n' "$destination"
else
    result=$?
    if [[ -e "$staging/previous" ]] && ! mv -- "$staging/previous" "$destination"; then
        clean_staging=0
        printf 'Previous HTML module retained at %s; restoration failed\n' "$staging/previous" >&2
    fi
    exit "$result"
fi
