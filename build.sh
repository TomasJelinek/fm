#!/bin/sh
set -eu

usage() {
    printf '%s\n' 'Usage: ./build.sh [debug|release|clean] [--install-deps] [--dry-run]'
    printf '%s\n' 'FM_BUILD_BACKEND selects auto, native or docker. Missing native Rust is bootstrapped with rustup.'
    printf '%s\n' 'Host packages are installed only with --install-deps, which selects the native backend.'
    printf '%s\n' 'Clean removes repository _build output, preserving .tools and the legacy _build/docker cache.'
}

print_command() {
    printf 'Command:'
    printf ' [%s]' "$@"
    printf '\n'
}

build_profile=debug
profile_chosen=0
install_deps=0
dry_run=0
for argument in "$@"; do
    case "$argument" in
        debug|release|clean)
            if [ "$profile_chosen" = 1 ]; then usage >&2; exit 2; fi
            build_profile=$argument
            profile_chosen=1
            ;;
        --install-deps) install_deps=1 ;;
        --dry-run) dry_run=1 ;;
        --help|-h) usage; exit 0 ;;
        *) usage >&2; exit 2 ;;
    esac
done
if [ "$build_profile" = clean ] && [ "$install_deps" = 1 ]; then
    usage >&2
    exit 2
fi
case "$(uname -s)" in
    Linux) ;;
    *) printf '%s\n' 'build.sh requires Linux; macOS support is pending. Use build.bat on Windows.' >&2; exit 2 ;;
esac

build_root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
cd -- "$build_root"
if [ "$build_profile" = clean ]; then
    if [ "$dry_run" = 1 ]; then exec sh scripts/clean-build.sh --dry-run; fi
    exec sh scripts/clean-build.sh
fi

# Preserve main's automatic isolated build when native libraries are absent.
build_backend=${FM_BUILD_BACKEND:-auto}
case "$build_backend" in
    auto|native|docker) ;;
    *) printf '%s\n' 'FM_BUILD_BACKEND must be auto, native, or docker.' >&2; exit 2 ;;
esac
if [ "$install_deps" = 1 ]; then
    if [ "$build_backend" = docker ]; then
        printf '%s\n' '--install-deps changes the native host and cannot be combined with FM_BUILD_BACKEND=docker.' >&2
        exit 2
    fi
    build_backend=native
fi
if [ "$build_backend" = auto ]; then
    if ! command -v cc >/dev/null 2>&1 ||
        ! command -v pkg-config >/dev/null 2>&1 ||
        ! pkg-config --exists fontconfig freetype2 xkbcommon x11 xcb wayland-client libudev; then
        build_backend=docker
    else
        build_backend=native
    fi
fi
if [ "$build_backend" = docker ]; then
    if [ "$dry_run" = 1 ]; then exec sh scripts/build-docker.sh "$build_profile" --dry-run; fi
    exec sh scripts/build-docker.sh "$build_profile"
fi

if [ "$dry_run" = 1 ]; then
    printf 'Working directory: %s\n' "$build_root"
    if [ "$install_deps" = 1 ]; then
        print_command sh scripts/setup-linux.sh --install --dry-run
        sh scripts/setup-linux.sh --install --dry-run
    fi
    set -- cargo build -p midnight-explorer -p midnight-elevation --locked
    if [ "$build_profile" = release ]; then set -- "$@" --release; fi
    print_command "$@"
    print_command bash scripts/build-native-filesystem.sh "$build_profile"
    print_command bash scripts/build-network-filesystem.sh "$build_profile"
    print_command bash scripts/build-viewers.sh "$build_profile"
    exit 0
fi

mkdir -p -- "$build_root/_build/tmp"
TMPDIR="$build_root/_build/tmp"
export TMPDIR
if [ "$install_deps" = 1 ]; then sh scripts/setup-linux.sh --install; fi

# Reuse existing Rust before preserving main's native bootstrap fallback.
if ! command -v cargo >/dev/null 2>&1; then
    cargo_bin=${CARGO_HOME:-${HOME:-}/.cargo}/bin
    if [ -x "$cargo_bin/cargo" ]; then
        PATH="$cargo_bin:$PATH"
        export PATH
    elif [ -x "$build_root/.tools/cargo/bin/cargo" ]; then
        CARGO_HOME="$build_root/.tools/cargo"
        RUSTUP_HOME=${RUSTUP_HOME:-"$build_root/.tools/rustup"}
        PATH="$CARGO_HOME/bin:$PATH"
        export CARGO_HOME RUSTUP_HOME PATH
    fi
fi
if ! command -v cargo >/dev/null 2>&1; then
    if ! command -v curl >/dev/null 2>&1; then
        printf '%s\n' 'Install curl and CA certificates with your package manager, then retry.' >&2
        exit 1
    fi
    CARGO_HOME=${CARGO_HOME:-"$build_root/.tools/cargo"}
    RUSTUP_HOME=${RUSTUP_HOME:-"$build_root/.tools/rustup"}
    PATH="$CARGO_HOME/bin:$PATH"
    export CARGO_HOME RUSTUP_HOME PATH
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
