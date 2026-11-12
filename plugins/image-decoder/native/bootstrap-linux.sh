#!/usr/bin/env bash
# Source before build-linux.sh so the following command sees only private tools.
# Usage: source bootstrap-linux.sh /absolute/path/_build/native-tools [jobs] [--offline]
set -euo pipefail
native_bootstrap_script=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
native_bootstrap_root=${1:?Expected an absolute _build directory}
native_bootstrap_jobs=${2:-2}
case "$native_bootstrap_jobs" in ''|*[!0-9]*) echo 'Expected a positive job count' >&2; return 2 ;; esac
if ((native_bootstrap_jobs < 1 || native_bootstrap_jobs > 32)); then echo 'Jobs must be 1..32' >&2; return 2; fi
native_bootstrap_offline=()
case "${3:-}" in --offline) native_bootstrap_offline+=(--offline) ;; '') ;; *) echo 'Expected --offline' >&2; return 2 ;; esac
for native_bootstrap_tool in python3 cc make perl; do
    command -v "$native_bootstrap_tool" >/dev/null || { echo "Missing prerequisite: $native_bootstrap_tool" >&2; return 2; }
done
native_bootstrap_root=$(python3 -I -S -B "$native_bootstrap_script/bootstrap-tools.py" "$native_bootstrap_root" --platform linux "${native_bootstrap_offline[@]}") || return $?
export PYTHONDONTWRITEBYTECODE=1
export TMPDIR="$native_bootstrap_root/tmp"
export MIDNIGHT_IMAGE_NATIVE_TOOLS="$native_bootstrap_root"
export PATH="$native_bootstrap_root/bin:$native_bootstrap_root/tools/ninja-1.13.0-linux/ninja-1.13.0.data/scripts:$PATH"
native_bootstrap_source="$native_bootstrap_root/tools/nasm-2.16.03-linux/nasm-2.16.03"
native_bootstrap_build="$native_bootstrap_root/nasm-build"
# Regenerate recipes from verified source; cached objects may still be reused.
(cd -- "$native_bootstrap_build" && bash "$native_bootstrap_source/configure" --srcdir="$native_bootstrap_source" --prefix="$native_bootstrap_root") || return $?
(cd -- "$native_bootstrap_build" && make dirs && make -j "$native_bootstrap_jobs" nasm) || return $?
install -m 755 -- "$native_bootstrap_build/nasm" "$native_bootstrap_root/bin/nasm" || return $?
python3 -I -S -B "$native_bootstrap_script/private-meson.py" --version || return $?
ninja --version || return $?
nasm --version || return $?
