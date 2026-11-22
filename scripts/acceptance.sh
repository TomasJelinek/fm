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
    printf '%s\n' '       [--stress] [--run-dir DIR]'
    printf '%s\n' 'Tests always use the debug test profile; the profile names the preceding build.'
    printf '%s\n' '--run-dir names an absolute folder for the run folders (default _build/acceptance).'
    printf '%s\n' '--stress runs the opt-in stress sizes (MIDNIGHT_ACCEPTANCE_STRESS=1).'
    printf '%s\n' 'Plugin packages outside this checkout are staged in its _build, or in the folder'
    printf '%s\n' 'that MIDNIGHT_ACCEPTANCE_STAGE_ROOT names.'
    printf '%s\n' 'The gated volumes'"'"' variables (MIDNIGHT_ACCEPTANCE_FAT_DIR, _REMOVABLE_DIR, _NETWORK_DIR,'
    printf '%s\n' '_OTHER_EXT_DIR, _NO_TMPFILE_DIR, _WIN_SYMLINKS) name absolute folders; the suite and its'
    printf '%s\n' 'staged children get them as they are.'
    printf '%s\n' 'Results: _build/acceptance/<run>/index.html; exit 0 pass, 1 failure, 2 usage, 127 no cargo.'
}

# The gated volumes' folders (crates/midnight-acceptance/tests/acceptance/fixtures.rs,
# `GATED`): the scenarios that need such a volume report SKIP without theirs.
gated_variables=(MIDNIGHT_ACCEPTANCE_FAT_DIR MIDNIGHT_ACCEPTANCE_REMOVABLE_DIR
    MIDNIGHT_ACCEPTANCE_NETWORK_DIR MIDNIGHT_ACCEPTANCE_OTHER_EXT_DIR
    MIDNIGHT_ACCEPTANCE_NO_TMPFILE_DIR MIDNIGHT_ACCEPTANCE_WIN_SYMLINKS)

profile=debug
profile_chosen=0
dry_run=0
stress=0
run_dir=
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
        --stress)
            stress=1
            runner+=("$1")
            ;;
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
        --run-dir)
            if [ $# -lt 2 ]; then usage >&2; exit 2; fi
            case "$2" in /?*) ;; *) usage >&2; exit 2 ;; esac
            run_dir=$2
            shift
            ;;
        --help|-h) usage; exit 0 ;;
        *) usage >&2; exit 2 ;;
    esac
    shift
done
# A gated folder that is set must be absolute, as the suite requires.
for variable in "${gated_variables[@]}"; do
    value=${!variable:-}
    case "$value" in
        ''|/*) ;;
        *) printf '%s %s is not an absolute folder\n' "$variable" "$value" >&2; usage >&2; exit 2 ;;
    esac
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
# A staged scenario makes its stage beside the packages' folder (crates/midnight-acceptance/
# tests/acceptance/viewers_stage.rs) unless MIDNIGHT_ACCEPTANCE_STAGE_ROOT names another
# folder. Packages outside this checkout (another checkout's or a shared build) get their
# stages in this checkout's _build instead, one level below it as the HTML viewer's
# AppArmor profile expects, so a run never writes beside packages it does not own.
stage_root=${MIDNIGHT_ACCEPTANCE_STAGE_ROOT:-}
case "$plugins" in
    "$root"/*) ;;
    *) if [ -z "$stage_root" ]; then stage_root=$root/_build; fi ;;
esac

if [ "$dry_run" = 1 ]; then
    printf 'Working directory: %s\n' "$root"
    printf 'Build profile: %s (the suite uses the debug test profile)\n' "$profile"
    printf 'Plugin packages: %s\n' "$plugins"
    if [ -n "$stage_root" ]; then printf 'Stages: %s\n' "$stage_root"; fi
    printf 'TMPDIR: %s\n' "${TMPDIR:-$root/_build/tmp}"
    if [ -n "$run_dir" ]; then printf 'Run folders: %s\n' "$run_dir"; fi
    if [ "$stress" = 1 ]; then printf '%s\n' 'Stress sizes: on (MIDNIGHT_ACCEPTANCE_STRESS=1)'; fi
    for variable in "${gated_variables[@]}"; do
        if [ -n "${!variable:-}" ]; then printf 'Gated folder: %s = %s\n' "$variable" "${!variable}"; fi
    done
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
if [ -n "$stage_root" ]; then
    mkdir -p -- "$stage_root"
    export MIDNIGHT_ACCEPTANCE_STAGE_ROOT="$stage_root"
fi
if [ -n "$run_dir" ]; then export MIDNIGHT_ACCEPTANCE_RUN_DIR="$run_dir"; fi
# Staged children read the stress sizes from the environment, as they inherit the gated
# volumes' variables.
if [ "$stress" = 1 ]; then export MIDNIGHT_ACCEPTANCE_STRESS=1; fi
printf 'Acceptance suite (build profile %s; tests use the debug test profile; plugins from %s)\n' \
    "$profile" "$plugins"
for variable in "${gated_variables[@]}"; do
    if [ -n "${!variable:-}" ]; then printf 'Gated folder: %s = %s\n' "$variable" "${!variable}"; fi
done
exec "${command[@]}"
