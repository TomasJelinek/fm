#!/usr/bin/env bash
# Opt-in end-to-end acceptance suite (docs/developer/ACCEPTANCE.md).
#
# Builds and runs the `acceptance` test target of crates/midnight-acceptance, which
# needs the `acceptance` feature: normal `cargo test`, CI and --validate never run it.
# `./build.sh --acceptance` calls this after its build steps (native or Docker backend).
# The suite renders with Slint's software renderer, so it needs no display.
set -euo pipefail

usage() {
    printf '%s\n' 'Usage: scripts/acceptance.sh [debug|release] [--dry-run] [--list] [--scale smoke|full]'
    printf '%s\n' '       [--only NAME[,NAME...]] [--strict-review] [--keep-fixture] [--all-themes]'
    printf '%s\n' 'Tests always use the debug test profile; the profile names the preceding build.'
    printf '%s\n' 'Results: _build/acceptance/<run>/index.html; exit 0 pass, 1 failure, 2 usage, 127 no cargo.'
}

profile=debug
profile_chosen=0
dry_run=0
runner=()
while [ $# -gt 0 ]; do
    case "$1" in
        debug|release)
            if [ "$profile_chosen" = 1 ]; then usage >&2; exit 2; fi
            profile=$1
            profile_chosen=1
            ;;
        --dry-run) dry_run=1 ;;
        --list|--strict-review|--keep-fixture|--all-themes) runner+=("$1") ;;
        --scale)
            if [ $# -lt 2 ]; then usage >&2; exit 2; fi
            case "$2" in smoke|full) ;; *) usage >&2; exit 2 ;; esac
            runner+=(--scale "$2")
            shift
            ;;
        --only)
            if [ $# -lt 2 ] || [ -z "$2" ]; then usage >&2; exit 2; fi
            IFS=',' read -r -a names <<< "$2"
            for name in "${names[@]}"; do
                case "$name" in
                    ''|-*) usage >&2; exit 2 ;;
                esac
                runner+=("$name")
            done
            shift
            ;;
        --help|-h) usage; exit 0 ;;
        *) usage >&2; exit 2 ;;
    esac
    shift
done

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
cd -- "$root"
# `--workspace` resolves features exactly as the application and workspace tests do, so
# the shared crates are reused instead of rebuilt; `--test acceptance` builds only this
# suite's target.
command=(cargo test --workspace --locked --features midnight-acceptance/acceptance --test acceptance --)
command+=("${runner[@]}")
# The viewer scenarios stage the plugin packages of the build that preceded the suite
# (`./build.sh release --acceptance` built release packages), not the test profile's.
# Absolute (relative to the checkout, like Cargo's target folder): Cargo runs the suite
# in its package folder.
plugins=${MIDNIGHT_ACCEPTANCE_PLUGINS:-${CARGO_TARGET_DIR:-$root/_build}/$profile}
case "$plugins" in
    /*) ;;
    *) plugins=$root/$plugins ;;
esac

if [ "$dry_run" = 1 ]; then
    printf 'Working directory: %s\n' "$root"
    printf 'Build profile: %s (the suite uses the debug test profile)\n' "$profile"
    printf 'Plugin packages: %s\n' "$plugins"
    printf 'TMPDIR: %s\n' "${TMPDIR:-$root/_build/tmp}"
    printf 'Command:'
    printf ' [%s]' "${command[@]}"
    printf '\n'
    exit 0
fi

if ! command -v cargo >/dev/null 2>&1; then
    if [ -x "$root/.tools/cargo/bin/cargo" ]; then
        export CARGO_HOME="$root/.tools/cargo" RUSTUP_HOME=${RUSTUP_HOME:-"$root/.tools/rustup"}
        export PATH="$CARGO_HOME/bin:$PATH"
    else
        printf '%s\n' 'cargo is not on PATH; run ./build.sh first.' >&2
        exit 127
    fi
fi
mkdir -p -- "$root/_build/tmp"
export TMPDIR=${TMPDIR:-"$root/_build/tmp"}
export MIDNIGHT_ACCEPTANCE_PLUGINS="$plugins"
printf 'Acceptance suite (build profile %s; tests use the debug test profile; plugins from %s)\n' \
    "$profile" "$plugins"
exec "${command[@]}"
