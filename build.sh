#!/bin/sh
set -eu

if [ "$#" -gt 1 ]; then
    printf '%s\n' 'Usage: ./build.sh [debug|release|clean]' >&2
    exit 2
fi

case "${1-debug}" in
    debug) build_profile=debug ;;
    release) build_profile=release ;;
    clean) build_profile=clean ;;
    *)
        printf '%s\n' 'Usage: ./build.sh [debug|release|clean]' >&2
        exit 2
        ;;
esac

build_root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
cd -- "$build_root"
if [ "$build_profile" = clean ]; then
    if [ -L "$build_root/_build" ]; then
        printf '%s\n' 'Cannot clean a symlinked _build directory.' >&2
        exit 1
    fi
    if [ -d "$build_root/_build" ]; then
        find "$build_root/_build" -mindepth 1 -maxdepth 1 ! -name docker -exec rm -rf -- {} +
    fi
    printf '%s\n' 'Cleaned _build artifacts; Docker dependencies and download cache preserved.'
    exit 0
fi

mkdir -p -- "$build_root/_build/tmp"
TMPDIR="$build_root/_build/tmp"
export TMPDIR

# Use an isolated dependency image when the host lacks native build libraries.
# Neither automatic setup path needs host root privileges.
case "${FM_BUILD_BACKEND:-auto}" in
    docker) exec sh "$build_root/scripts/build-docker.sh" "$build_profile" ;;
    native) ;;
    auto)
        if ! command -v cc >/dev/null 2>&1 || \
            ! command -v pkg-config >/dev/null 2>&1 || \
            ! pkg-config --exists fontconfig freetype2 xkbcommon x11 xcb wayland-client libudev; then
            exec sh "$build_root/scripts/build-docker.sh" "$build_profile"
        fi
        ;;
    *)
        printf '%s\n' 'FM_BUILD_BACKEND must be auto, native, or docker.' >&2
        exit 2
        ;;
esac

# A prior rustup installation may not yet be on this shell's PATH.
if ! command -v cargo >/dev/null 2>&1; then
    cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"
    PATH="$cargo_bin:$PATH"
    export PATH
fi
if ! command -v cargo >/dev/null 2>&1; then
    if ! command -v curl >/dev/null 2>&1; then
        printf '%s\n' 'Install curl and CA certificates with your package manager, then retry.' >&2
        exit 1
    fi
    printf '%s\n' 'Installing Rust with rustup; Cargo will select rust-toolchain.toml.'
    rustup_installer=$(mktemp "$TMPDIR/rustup-init.XXXXXX")
    trap 'rm -f -- "$rustup_installer"' EXIT
    trap 'exit 1' HUP INT TERM
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o "$rustup_installer"
    sh "$rustup_installer" -y --profile minimal --default-toolchain none
    rm -f -- "$rustup_installer"
    trap - EXIT HUP INT TERM
fi

if [ "$build_profile" = release ]; then
    cargo build -p midnight-explorer -p midnight-elevation --locked --release
else
    cargo build -p midnight-explorer -p midnight-elevation --locked
fi
bash scripts/build-native-filesystem.sh "$build_profile"
bash scripts/build-network-filesystem.sh "$build_profile"
exec bash scripts/build-viewers.sh "$build_profile"
