#!/usr/bin/env bash
set -euo pipefail
if [[ $# -ne 3 ]]; then
    printf '%s\n' 'Usage: test-linux.sh PLUGIN_SO API_INCLUDE BUILD_ROOT' >&2
    exit 2
fi
library=$(realpath -- "$1")
include=$(realpath -- "$2")
build_root=$(realpath -m -- "$3")
case "$build_root/" in
    */_build/*) ;;
    *) printf '%s\n' 'Place test outputs under the common _build directory.' >&2; exit 2 ;;
esac
test_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
mkdir -p -- "$build_root"
cc -std=c11 -Wall -Wextra -Werror -pedantic -I"$include" \
    "$test_root/conformance.c" "$test_root/loader_linux.c" -ldl \
    -o "$build_root/viewer-conformance"
"$build_root/viewer-conformance" "$library"
