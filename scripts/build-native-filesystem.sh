#!/usr/bin/env bash
set -euo pipefail
profile=${1:-debug}
if [[ $# -gt 2 ]]; then
    printf '%s\n' 'Usage: build-native-filesystem.sh [debug|release] [APP_DIRECTORY]' >&2
    exit 2
fi
case "$profile" in debug|release) ;; *) printf '%s\n' 'Expected debug or release' >&2; exit 2;; esac
case "$(uname -s)" in Linux) ;; *) printf '%s\n' 'Native filesystem package requires Linux' >&2; exit 2;; esac
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$project_root"
app_directory=${2:-"${CARGO_TARGET_DIR:-$project_root/_build}/$profile"}
plugin_root="$project_root/plugins/midnight-filesystem-native-linux"
export CARGO_TARGET_DIR=${MIDNIGHT_NATIVE_FILESYSTEM_BUILD_ROOT:-"$project_root/_build/native-filesystems/linux"}
export TMPDIR="$project_root/_build/tmp"
mkdir -p -- "$TMPDIR"
options=(--locked --manifest-path "$plugin_root/Cargo.toml")
[[ "${MIDNIGHT_NATIVE_FILESYSTEM_OFFLINE:-0}" == 1 ]] && options+=(--offline)
[[ "$profile" == release ]] && options+=(--release)
if [[ "${MIDNIGHT_TEST_NATIVE_FILESYSTEM:-0}" == 1 ]]; then
    cargo test "${options[@]}"
fi
cargo build "${options[@]}"
package="$app_directory/filesystems/native.linux"
mkdir -p -- "$package"
native_library=libmidnight_filesystem_native_linux.so
native_files=("$native_library" plugin.toml README.md THIRD-PARTY.md)
native_stage=$(mktemp -d -- "$package/.native-stage.XXXXXXXX")
cleanup_native_stage() {
    # Remove only our known staging files; unexpected entries are never recursed.
    for name in "${native_files[@]}"; do rm -f -- "$native_stage/$name"; done
    rmdir -- "$native_stage"
}
trap cleanup_native_stage EXIT
# Stage the entire input before publishing any entry. A running host may have
# the old .so mapped: rename a completed new inode, never truncate that inode.
cp -- "$CARGO_TARGET_DIR/$profile/$native_library" "$native_stage/$native_library"
for name in plugin.toml README.md THIRD-PARTY.md; do
    cp -- "$plugin_root/$name" "$native_stage/$name"
done
for name in "${native_files[@]}"; do
    mv -fT -- "$native_stage/$name" "$package/$name"
done
printf 'Native filesystem package: %s\n' "$package"
