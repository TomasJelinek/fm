#!/bin/sh
set -eu

if [ "$#" -gt 1 ]; then
    printf '%s\n' 'Usage: ./build.sh [debug|release]' >&2
    exit 2
fi

case "${1-debug}" in
    debug) build_profile=debug ;;
    release) build_profile=release ;;
    *)
        printf '%s\n' 'Usage: ./build.sh [debug|release]' >&2
        exit 2
        ;;
esac

build_root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
cd -- "$build_root"
mkdir -p -- "$build_root/_build/tmp"
TMPDIR="$build_root/_build/tmp"
export TMPDIR

if [ "$build_profile" = release ]; then
    cargo build -p midnight-explorer -p midnight-elevation --locked --release
else
    cargo build -p midnight-explorer -p midnight-elevation --locked
fi
bash scripts/build-native-filesystem.sh "$build_profile"
bash scripts/build-network-filesystem.sh "$build_profile"
exec bash scripts/build-viewers.sh "$build_profile"
