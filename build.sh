#!/bin/sh
set -eu

usage() {
    printf '%s\n' 'Usage: ./build.sh [debug|release|clean] [--install-deps] [--dry-run] [--validate] [--acceptance]'
    printf '%s\n' 'FM_BUILD_BACKEND selects auto, native or docker. Missing native Rust is bootstrapped with rustup.'
    printf '%s\n' 'Host packages are installed only with --install-deps, which selects the native backend.'
    printf '%s\n' 'Clean removes repository _build output, preserving .tools and the legacy _build/docker cache.'
    printf '%s\n' '--validate then runs scripts/validate.sh: the tests that CI runs on Linux (native backend only).'
    printf '%s\n' '--acceptance then runs scripts/acceptance.sh: the opt-in end-to-end acceptance suite (native or Docker).'
}

print_command() {
    printf 'Command:'
    printf ' [%s]' "$@"
    printf '\n'
}

# The same library locations scripts/prepare-thumbnail-ffmpeg.py searches.
libclang_available() {
    if [ -n "${LIBCLANG_PATH:-}" ]; then return 0; fi
    for directory in /usr/lib/llvm-*/lib /usr/lib/x86_64-linux-gnu /usr/lib/aarch64-linux-gnu \
        /usr/lib64 /usr/local/lib; do
        for library in "$directory"/libclang.so* "$directory"/libclang-*.so*; do
            if [ -e "$library" ]; then return 0; fi
        done
    done
    if command -v llvm-config >/dev/null 2>&1; then
        directory=$(llvm-config --libdir)
        for library in "$directory"/libclang.so* "$directory"/libclang-*.so*; do
            if [ -e "$library" ]; then return 0; fi
        done
    fi
    return 1
}

# Report missing module prerequisites before the host build, not after it.
warn_missing_module_prerequisites() {
    missing=
    for tool in python3 gpg perl make xz cmake; do
        if ! command -v "$tool" >/dev/null 2>&1; then missing="$missing $tool"; fi
    done
    if ! libclang_available; then missing="$missing libclang"; fi
    if [ -n "$missing" ]; then
        printf 'Warning: missing native module build prerequisites:%s. Install them with ./build.sh --install-deps or build with FM_BUILD_BACKEND=docker.\n' "$missing" >&2
    fi
}

build_profile=debug
profile_chosen=0
install_deps=0
dry_run=0
validate=0
acceptance=0
for argument in "$@"; do
    case "$argument" in
        debug|release|clean)
            if [ "$profile_chosen" = 1 ]; then usage >&2; exit 2; fi
            build_profile=$argument
            profile_chosen=1
            ;;
        --install-deps) install_deps=1 ;;
        --dry-run) dry_run=1 ;;
        --validate)
            if [ "$validate" = 1 ]; then usage >&2; exit 2; fi
            validate=1
            ;;
        --acceptance)
            if [ "$acceptance" = 1 ]; then usage >&2; exit 2; fi
            acceptance=1
            ;;
        --help|-h) usage; exit 0 ;;
        *) usage >&2; exit 2 ;;
    esac
done
if [ "$build_profile" = clean ] &&
    { [ "$install_deps" = 1 ] || [ "$validate" = 1 ] || [ "$acceptance" = 1 ]; }; then
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
    # The build image lacks the X11 test display (Xvfb) and mounts the source read-only,
    # and validation inside it is unqualified; validation therefore runs natively only.
    if [ "$validate" = 1 ]; then
        printf '%s\n' '--validate runs the tests natively and cannot use the Docker build backend.' >&2
        if [ "${FM_BUILD_BACKEND:-auto}" = docker ]; then
            printf '%s\n' 'Unset FM_BUILD_BACKEND=docker (or set it to native).' >&2
        else
            printf '%s\n' 'The native compiler or GUI development libraries are missing; install them with ./build.sh --install-deps.' >&2
        fi
        exit 2
    fi
    # The acceptance suite renders with Slint's software renderer and needs no display,
    # so the container runs the whole chain, acceptance included.
    set -- "$build_profile"
    if [ "$acceptance" = 1 ]; then set -- "$@" --acceptance; fi
    if [ "$dry_run" = 1 ]; then exec sh scripts/build-docker.sh "$@" --dry-run; fi
    exec sh scripts/build-docker.sh "$@"
fi

# The explorer records the SHA-256 of the authorization helper that it ships with
# (docs/RELEASE_BUILD.md): the helper is built first, then both packages with the
# variable naming it. The same package selection keeps the helper's features, so the
# second build reuses it and links only the explorer.
cargo_target=${CARGO_TARGET_DIR:-_build}
case $cargo_target in /*) ;; *) cargo_target=$build_root/$cargo_target ;; esac
elevated_listing=$cargo_target/$build_profile/midnight-elevated-listing

if [ "$dry_run" = 1 ]; then
    printf 'Working directory: %s\n' "$build_root"
    if [ "$install_deps" = 1 ]; then
        print_command sh scripts/setup-linux.sh --install --dry-run
        sh scripts/setup-linux.sh --install --dry-run
    fi
    set -- cargo build -p midnight-explorer -p midnight-elevation --bin midnight-elevated-listing --locked
    if [ "$build_profile" = release ]; then set -- "$@" --release; fi
    print_command "$@"
    set -- env "MIDNIGHT_ELEVATED_LISTING=$elevated_listing" cargo build -p midnight-explorer -p midnight-elevation --locked
    if [ "$build_profile" = release ]; then set -- "$@" --release; fi
    print_command "$@"
    print_command bash scripts/build-native-filesystem.sh "$build_profile"
    print_command bash scripts/build-network-filesystem.sh "$build_profile"
    print_command bash scripts/build-viewers.sh "$build_profile"
    print_command bash scripts/build-default-thumbnailers.sh "$build_profile"
    print_command bash scripts/build-archive-filesystem.sh "$build_profile"
    if [ "$validate" = 1 ]; then
        print_command bash scripts/validate.sh "$build_profile"
        if [ "$acceptance" = 0 ]; then exec bash scripts/validate.sh "$build_profile" --dry-run; fi
        bash scripts/validate.sh "$build_profile" --dry-run
    fi
    if [ "$acceptance" = 1 ]; then
        print_command bash scripts/acceptance.sh "$build_profile"
        exec bash scripts/acceptance.sh "$build_profile" --dry-run
    fi
    exit 0
fi

mkdir -p -- "$build_root/_build/tmp"
TMPDIR="$build_root/_build/tmp"
export TMPDIR
if [ "$install_deps" = 1 ]; then sh scripts/setup-linux.sh --install; fi
warn_missing_module_prerequisites

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
    printf '%s\n' 'Installing Rust with rustup and the toolchain that rust-toolchain.toml pins.'
    rustup_installer=$(mktemp "$TMPDIR/rustup-init.XXXXXX")
    trap 'rm -f -- "$rustup_installer"' EXIT
    trap 'exit 1' HUP INT TERM
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o "$rustup_installer"
    sh "$rustup_installer" -y --profile minimal --default-toolchain none
    rm -f -- "$rustup_installer"
    trap - EXIT HUP INT TERM
    # Explicitly, so the build does not depend on proxy auto-install (which
    # RUSTUP_AUTO_INSTALL=0 or `rustup set auto-install disable` turns off).
    sh scripts/install-rust-toolchain.sh rust-toolchain.toml
fi

if [ "$build_profile" = release ]; then
    cargo build -p midnight-explorer -p midnight-elevation --bin midnight-elevated-listing --locked --release
    MIDNIGHT_ELEVATED_LISTING=$elevated_listing cargo build -p midnight-explorer -p midnight-elevation --locked --release
else
    cargo build -p midnight-explorer -p midnight-elevation --bin midnight-elevated-listing --locked
    MIDNIGHT_ELEVATED_LISTING=$elevated_listing cargo build -p midnight-explorer -p midnight-elevation --locked
fi
bash scripts/build-native-filesystem.sh "$build_profile"
bash scripts/build-network-filesystem.sh "$build_profile"
bash scripts/build-viewers.sh "$build_profile"
bash scripts/build-default-thumbnailers.sh "$build_profile"
if [ "$validate" = 0 ] && [ "$acceptance" = 0 ]; then
    exec bash scripts/build-archive-filesystem.sh "$build_profile"
fi
bash scripts/build-archive-filesystem.sh "$build_profile"
if [ "$acceptance" = 0 ]; then exec bash scripts/validate.sh "$build_profile"; fi
if [ "$validate" = 0 ]; then exec bash scripts/acceptance.sh "$build_profile"; fi
# Both: acceptance runs even when validation failed; the first failure decides the status.
validation_status=0
bash scripts/validate.sh "$build_profile" || validation_status=$?
acceptance_status=0
bash scripts/acceptance.sh "$build_profile" || acceptance_status=$?
if [ "$validation_status" != 0 ]; then exit "$validation_status"; fi
exit "$acceptance_status"
