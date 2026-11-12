#!/usr/bin/env bash
set -euo pipefail
provider=${1:?Expected native or network provider}
shift
case "$provider" in native|network) ;; *) printf '%s\n' 'Expected native or network' >&2; exit 2;; esac
profile=${1:-debug}
if [[ $# -gt 2 ]]; then
    printf 'Usage: build-%s-filesystem.sh [debug|release] [APP_DIRECTORY]\n' "$provider" >&2
    exit 2
fi
case "$profile" in debug|release) ;; *) printf '%s\n' 'Expected debug or release' >&2; exit 2;; esac
case "$(uname -s)" in Linux) ;; *) printf '%s\n' 'Filesystem package requires Linux' >&2; exit 2;; esac
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$project_root"
app_directory=${2:-"${CARGO_TARGET_DIR:-$project_root/_build}/$profile"}
plugin_root="$project_root/plugins/midnight-filesystem-$provider"
manifest=plugin.linux.toml
library="libmidnight_filesystem_$provider.so"
if [[ "$provider" == native ]]; then
    plugin_root+=-linux
    manifest=plugin.toml
    library=libmidnight_filesystem_native_linux.so
fi
build_variable="MIDNIGHT_${provider^^}_FILESYSTEM_BUILD_ROOT"
offline_variable="MIDNIGHT_${provider^^}_FILESYSTEM_OFFLINE"
test_variable="MIDNIGHT_TEST_${provider^^}_FILESYSTEM"
export CARGO_TARGET_DIR=${!build_variable:-"$project_root/_build/$provider-filesystems/linux"}
export TMPDIR="$project_root/_build/tmp"
mkdir -p -- "$TMPDIR"
options=(--locked --manifest-path "$plugin_root/Cargo.toml")
[[ "${!offline_variable:-0}" == 1 ]] && options+=(--offline)
[[ "$profile" == release ]] && options+=(--release)
if [[ "${!test_variable:-0}" == 1 ]]; then
    cargo test "${options[@]}"
fi
cargo build "${options[@]}"
package="$app_directory/filesystems/$provider.linux"
mkdir -p -- "$package"
package_files=("$library" plugin.toml README.md THIRD-PARTY.md)
package_stage=$(mktemp -d -- "$package/.$provider-stage.XXXXXXXX")
cleanup_package_stage() {
    # Remove only our known staging files; unexpected entries are never recursed.
    for name in "${package_files[@]}"; do rm -f -- "$package_stage/$name"; done
    rmdir -- "$package_stage"
}
trap cleanup_package_stage EXIT
# Stage the entire input before publishing any entry. A running host may have
# the old .so mapped: rename a completed new inode, never truncate that inode.
cp -- "$CARGO_TARGET_DIR/$profile/$library" "$package_stage/$library"
cp -- "$plugin_root/$manifest" "$package_stage/plugin.toml"
for name in README.md THIRD-PARTY.md; do
    cp -- "$plugin_root/$name" "$package_stage/$name"
done
for name in "${package_files[@]}"; do
    mv -fT -- "$package_stage/$name" "$package/$name"
done
printf 'Filesystem package: %s\n' "$package"
