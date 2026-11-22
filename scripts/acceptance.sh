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
    printf '%s\n' 'that MIDNIGHT_ACCEPTANCE_STAGE_ROOT names. A run stops (exit 2) when the packages are'
    printf '%s\n' 'newer than this build or the stages would lie below a folder other accounts can change.'
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
# A target folder shared with checkouts of other commits can link their code: Cargo names
# a path package's artifacts relative to the workspace root and judges freshness by mtime
# (docs/developer/TEST_EXECUTION.md, "Targeted test execution").
case "${CARGO_TARGET_DIR:-}" in
    ''|"$root"/*|[!/]*) ;;
    *) printf 'acceptance.sh: warning: CARGO_TARGET_DIR %s is outside this checkout; a target shared with checkouts of other commits can link their code (docs/developer/TEST_EXECUTION.md)\n' "$CARGO_TARGET_DIR" >&2 ;;
esac
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

# Two traps that make every staged scenario fail for a misleading reason are named before
# anything runs, when there are packages to stage:
# - packages newer than this checkout: the native filesystem host refuses a package whose
#   minimum_host_minor exceeds its ABI_MINOR (midnight-native-filesystem-host's
#   discovery.rs), so the staged scenarios find no filesystem plugin;
# - a stage below a folder that other accounts may change (a copy from /mnt/c keeps
#   drvfs's mode 0777): the plugin host refuses packages from it (midnight-plugin-host's
#   trust_linux.rs: every folder above them owned by root or this account, and writable
#   only by its owner unless it is a sticky folder such as /tmp).
problems=()
if compgen -G "$plugins/viewers/*/" >/dev/null || compgen -G "$plugins/filesystems/*/" >/dev/null; then
    abi_minor=$(sed -n 's/^pub const ABI_MINOR: u16 = \([0-9][0-9]*\);.*/\1/p' \
        "$root/filesystem-api/midnight-filesystem-plugin-api/src/abi.rs" 2>/dev/null)
    if [ -n "$abi_minor" ]; then
        for manifest in "$plugins"/filesystems/*/plugin.toml; do
            [ -f "$manifest" ] || continue
            wanted=$(sed -n 's/^minimum_host_minor *= *\([0-9][0-9]*\).*/\1/p' "$manifest" | head -n 1)
            if [ -n "$wanted" ] && [ "$wanted" -gt "$abi_minor" ]; then
                problems+=("packages newer than this build: $(dirname -- "$manifest") needs filesystem host ABI minor $wanted and this checkout has $abi_minor; build the packages from this checkout or name matching ones in MIDNIGHT_ACCEPTANCE_PLUGINS")
            fi
        done
    fi
    stages=$stage_root
    if [ -z "$stages" ]; then
        case "$(basename -- "$(dirname -- "$plugins")")" in
            _build) stages=$(dirname -- "$plugins") ;;
            *) stages=${CARGO_TARGET_DIR:-$root/_build} ;;
        esac
    fi
    folder=$stages
    while [ ! -e "$folder" ] && [ "$folder" != / ]; do folder=$(dirname -- "$folder"); done
    # The account this run's processes have, as the plugin host compares it (geteuid):
    # bash's own EUID, not an `id` found on PATH, which a wrapper or a test may replace.
    user=$EUID
    while :; do
        mode=$(stat -c %a -- "$folder" 2>/dev/null || printf 0)
        owner=$(stat -c %u -- "$folder" 2>/dev/null || printf 0)
        if [ -L "$folder" ] || { [ "$owner" != 0 ] && [ "$owner" != "$user" ]; }; then
            problems+=("$folder is a link or owned by another account: the plugin host refuses packages staged below it")
        elif [ $(( 8#$mode & 8#022 )) -ne 0 ] && [ $(( 8#$mode & 8#1000 )) -eq 0 ]; then
            problems+=("$folder is writable by other accounts (mode $mode): the plugin host refuses packages staged below it; run chmod go-w on it (a copy from /mnt/c keeps mode 0777, rsync --chmod=go-w does not)")
        fi
        [ "$folder" = / ] && break
        folder=$(dirname -- "$folder")
    done
fi
# A run under a machine-wide lease: a wrapper names the lease's lock file in
# MIDNIGHT_ACCEPTANCE_REQUIRE_LEASE, so that a wrapper whose lease failed cannot run beside
# another one. The lock must exist and, with MIDNIGHT_ACCEPTANCE_LEASE_OWNER set, name that
# owner first. (The lease of a run in WSL is held by a Windows process, which this side
# cannot see; scripts\acceptance.ps1 also checks the lock's process.)
if [ -n "${MIDNIGHT_ACCEPTANCE_REQUIRE_LEASE:-}" ]; then
    lease=$MIDNIGHT_ACCEPTANCE_REQUIRE_LEASE
    if [ ! -f "$lease" ]; then
        problems+=("the lease $lease (MIDNIGHT_ACCEPTANCE_REQUIRE_LEASE) was not taken for this run: it is not held")
    elif [ -n "${MIDNIGHT_ACCEPTANCE_LEASE_OWNER:-}" ]; then
        holder=$(head -c 512 -- "$lease" | tr -d '\r' | awk 'NR == 1 { print $1 }')
        if [ "$holder" != "$MIDNIGHT_ACCEPTANCE_LEASE_OWNER" ]; then
            problems+=("the lease $lease (MIDNIGHT_ACCEPTANCE_REQUIRE_LEASE) was not taken for this run: it is held by ${holder:-nobody}, not $MIDNIGHT_ACCEPTANCE_LEASE_OWNER")
        fi
    fi
fi

if [ "$dry_run" = 1 ]; then
    printf 'Working directory: %s\n' "$root"
    printf 'Build profile: %s (the suite uses the debug test profile)\n' "$profile"
    printf 'Plugin packages: %s\n' "$plugins"
    if [ -n "$stage_root" ]; then printf 'Stages: %s\n' "$stage_root"; fi
    for problem in "${problems[@]}"; do printf 'A run stops: %s\n' "$problem"; done
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
# --list runs no scenario and stages nothing.
listing=0
for argument in "${runner[@]}"; do
    if [ "$argument" = --list ]; then listing=1; fi
done
if [ "$listing" = 0 ] && [ "${#problems[@]}" -gt 0 ]; then
    for problem in "${problems[@]}"; do printf 'acceptance.sh: %s\n' "$problem" >&2; done
    exit 2
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
