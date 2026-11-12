#!/bin/sh
set -eu

usage() {
    printf '%s\n' 'Usage: ./build.sh [debug|release|clean] [--install-deps] [--dry-run]'
    printf '%s\n' 'Native packages are installed only with --install-deps. Rust must already be installed.'
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

# Reuse an existing installation before asking for manual Rust setup.
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
    printf '%s\n' 'Rust Cargo was not found. Install Rust with rustup, then reopen your terminal.' >&2
    exit 127
fi
if [ "$install_deps" = 1 ]; then sh scripts/setup-linux.sh --install; fi

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
