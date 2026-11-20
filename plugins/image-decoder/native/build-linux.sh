#!/usr/bin/env bash
# Source this script so the following Cargo command inherits the static library paths.
# Usage: source build-linux.sh /absolute/path/_build/dav1d [jobs] [--offline]
set -euo pipefail
native_script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd) || return $?
native_root=${1:?Expected an absolute _build directory}
native_jobs=${2:-4}
case "$native_jobs" in ''|*[!0-9]*) echo 'Expected a positive job count' >&2; return 2 ;; esac
if ((native_jobs < 1 || native_jobs > 32)); then echo 'Jobs must be 1..32' >&2; return 2; fi
native_offline=()
case "${3:-}" in --offline) native_offline+=(--offline) ;; '') ;; *) echo 'Expected --offline' >&2; return 2 ;; esac
native_source=$(python3 -I -S -B "$native_script_dir/prepare-source.py" "$native_root" "${native_offline[@]}") || return $?
# prepare-source.py refused links from _build down, so this resolves only the links above
# _build (a linked checkout): Meson sees one spelling of the source and build folders.
native_source=$(realpath -- "$native_source") || return $?
native_root=$(realpath -- "$native_root") || return $?
native_build="$native_root/linux-$(uname -m)"
native_prefix="$native_root/prefix-linux-$(uname -m)"
native_setup=(setup "$native_build" "$native_source" --prefix "$native_prefix" --libdir lib
    --buildtype release --default-library static --wrap-mode=nodownload
    -Denable_tools=false -Denable_tests=false -Denable_examples=false -Denable_docs=false)
# A build configured before prepare-source.py made vcs_version.h a configure-time file lists
# the header as a ninja output, and reconfiguring it lets ninja delete the regenerated
# header: configure such a build afresh.
if [[ -f "$native_build/.ninja_log" ]] && grep -Fq vcs_version.h "$native_build/.ninja_log"; then
    rm -rf -- "${native_build:?}" || return $?
fi
[[ ! -f "$native_build/meson-private/coredata.dat" ]] || native_setup+=(--reconfigure)
python3 -I -S -B "$native_script_dir/private-meson.py" "${native_setup[@]}" || return $?
python3 -I -S -B "$native_script_dir/private-meson.py" compile -C "$native_build" -j "$native_jobs" || return $?
python3 -I -S -B "$native_script_dir/private-meson.py" install -C "$native_build" --no-rebuild || return $?
# dav1d's private system dependencies are supplied by the native .pc metadata.
export PKG_CONFIG_PATH="$native_prefix/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export SYSTEM_DEPS_DAV1D_LINK=static SYSTEM_DEPS_DAV1D_BUILD_INTERNAL=never
printf 'Pinned static dav1d: %s\n' "$native_prefix"
