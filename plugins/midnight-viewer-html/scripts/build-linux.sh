#!/usr/bin/env bash
set -euo pipefail
project=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
repository=$(cd -- "$project/../.." && pwd)
sdk_config=${1:?Usage: build-linux.sh /absolute/public-sdk-config.toml}
: "${CEF_PATH:?Set CEF_PATH to the verified flattened Linux CEF 156.0.26 SDK}"
target=${MIDNIGHT_HTML_TARGET_DIR:-"$repository/_build/html-viewer/provider-target/linux"}
package="$target/package"
IFS= read -r -d '' CEF_PATH < <(
    python3 "$project/scripts/package-linux.py" validate "$CEF_PATH"
)
export CEF_PATH
[[ -f "$sdk_config" ]] || { printf '%s\n' "Public SDK config is missing" >&2; exit 1; }
[[ ! -e "$package" ]] || {
    printf '%s\n' "Package already exists: retain or explicitly relocate $package before rebuilding" >&2
    exit 1
}
mkdir -p -- "$target"
# DT_RPATH applies to transitive CEF dependencies after the supervisor clears LD_LIBRARY_PATH.
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C link-arg=-Wl,--disable-new-dtags,-rpath,\$ORIGIN"
cargo build --manifest-path "$project/Cargo.toml" --config "$sdk_config" \
    --target-dir "$target" --locked --offline --release --bin midnight-html-linux-launcher
cargo metadata --manifest-path "$project/Cargo.toml" --config "$sdk_config" \
    --filter-platform x86_64-unknown-linux-gnu --locked --offline --format-version 1 \
    > "$target/dependency-metadata.json"
python3 "$project/scripts/package-linux.py" package "$CEF_PATH" "$package" \
    "$target/release/midnight-html-linux-launcher" "$target/dependency-metadata.json"
# No installation, setuid chmod, policy relaxation or sandbox disabling is performed.
printf '%s\n' "Unqualified Linux package staged at $package; audit notices and run native qualification before distribution."
