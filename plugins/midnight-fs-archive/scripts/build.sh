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
if [[ $profile == release ]]; then options+=(--release); fi
plugin_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
case "$build_root" in /*/_build/*) ;; *) echo 'Build output must be under _build' >&2; exit 1;; esac
export CARGO_TARGET_DIR="$build_root" CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
if [[ $run_tests == 1 ]]; then
    cargo test --manifest-path "$plugin_root/Cargo.toml" --locked --config "$sdk_config" "${options[@]}"
fi
cargo build --manifest-path "$plugin_root/Cargo.toml" --locked --config "$sdk_config" "${options[@]}"
if [[ -n "$package" ]]; then
    case "$package" in /*/_build/*) ;; *) echo 'Package staging must be under _build' >&2; exit 1;; esac
    mkdir -p -- "$package"
    cp -- "$build_root/$profile/midnight-fs-archive" "$package/"
    cp -- "$plugin_root/THIRD-PARTY.md" "$package/"
    target=$(rustc -vV | sed -n 's/^host: //p')
    cat > "$package/plugin.toml" <<EOF
schema_version = 1
id = "midnight.fs.archive"
kind = "filesystem-worker"
version = "0.1.0"
entry_executable = "midnight-fs-archive"
target = "$target"
protocol = 3
stable_entry_ids = true
formats = ["zip", "tar", "gzip", "tar-gzip", "7z", "xz", "tar-xz", "bzip2", "tar-bzip2", "zstd", "tar-zstd"]
EOF
    python3 -B "$plugin_root/scripts/notices.py" --config "$sdk_config" --output "$package/licenses"
fi
