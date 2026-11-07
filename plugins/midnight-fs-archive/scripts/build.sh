#!/usr/bin/env bash
set -euo pipefail
sdk_config=${1:?public SDK config required}
build_root=${2:?absolute build directory under _build required}
package=${3:-}
plugin_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
case "$build_root" in /*/_build/*) ;; *) echo 'Build output must be under _build' >&2; exit 1;; esac
export CARGO_TARGET_DIR="$build_root" CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
cargo test --manifest-path "$plugin_root/Cargo.toml" --locked --config "$sdk_config"
cargo build --release --manifest-path "$plugin_root/Cargo.toml" --locked --config "$sdk_config"
if [[ -n "$package" ]]; then
    case "$package" in /*/_build/*) ;; *) echo 'Package staging must be under _build' >&2; exit 1;; esac
    mkdir -p -- "$package"
    cp -- "$build_root/release/midnight-fs-archive" "$package/"
    cp -- "$plugin_root/THIRD-PARTY.md" "$package/"
    target=$(rustc -vV | sed -n 's/^host: //p')
    cat > "$package/plugin.toml" <<EOF
schema_version = 1
id = "midnight.fs.archive"
kind = "filesystem-worker"
version = "0.1.0"
entry_executable = "midnight-fs-archive"
target = "$target"
protocol = 1
formats = ["zip", "tar", "gzip", "tar-gzip", "7z", "xz", "tar-xz", "bzip2", "tar-bzip2", "zstd", "tar-zstd"]
EOF
    python3 "$plugin_root/scripts/notices.py" --config "$sdk_config" --output "$package/licenses"
fi
