#!/usr/bin/env bash
# Full local validation on Linux: the tests that CI (.github/workflows/ci.yml) runs for
# this host, with CI's own test commands and the builds and packages those tests need,
# in CI's job order. ./build.sh --validate runs it after its build steps. Each step
# runs in its own subshell and logs to _build/validate/logs/<run>/; a step whose
# prerequisite is missing is SKIPPED with the reason. Exit codes: 0 no step failed
# (with --strict: also none skipped for a missing prerequisite), 1 a step failed,
# 2 invalid arguments, 127 cargo, rustc or Python 3.12+ is missing.
# See docs/developer/TEST_EXECUTION.md.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/validate.sh [debug|release] [--strict] [--offline] [--dry-run] [--jobs N]
           [--only ID[,ID...]] [--plugin-sdk CONFIG] [--worker-sdk CONFIG] [--web-sdk CONFIG]
Runs the tests that CI runs for this platform; debug|release selects the
profile of the host build and smoke test (default debug). --strict fails when a step is
skipped for a missing prerequisite. --offline (or CARGO_NET_OFFLINE=true) forbids
downloads. --dry-run prints the plan and its commands. --jobs N limits every build and
test to N parallel jobs and test threads (default: all logical CPUs). --only runs the
named steps and the steps they need. The --*-sdk options use an already packaged SDK
bundle instead of packaging one, as CI's plugin jobs download theirs.
EOF
}

stop_usage() {
    printf 'validate.sh: %s\n' "$1" >&2
    usage >&2
    exit 2
}

build_profile=
strict=0
dry_run=0
offline=0
cargo_jobs=
selection=()
supplied_plugin_sdk=
supplied_worker_sdk=
supplied_web_sdk=
while [[ $# -gt 0 ]]; do
    argument=$1
    shift
    name=$argument
    value=
    has_value=0
    if [[ $argument == --*=* ]]; then name=${argument%%=*}; value=${argument#*=}; has_value=1; fi
    case "$name" in
        debug|release|--profile)
            if [[ $name == --profile ]]; then
                if [[ $has_value == 0 ]]; then
                    [[ $# -gt 0 ]] || stop_usage '--profile needs a value'
                    value=$1
                    shift
                fi
                case "$value" in debug|release) ;; *) stop_usage "unknown profile '$value'" ;; esac
            else
                value=$name
            fi
            [[ -z $build_profile ]] || stop_usage 'choose one profile'
            build_profile=$value
            ;;
        --strict|--dry-run|--offline|--help)
            [[ $has_value == 0 ]] || stop_usage "$name takes no value"
            case "$name" in
                --strict) strict=1 ;;
                --dry-run) dry_run=1 ;;
                --offline) offline=1 ;;
                --help) usage; exit 0 ;;
            esac
            ;;
        --only|--plugin-sdk|--worker-sdk|--web-sdk|--jobs)
            if [[ $has_value == 0 ]]; then
                [[ $# -gt 0 ]] || stop_usage "$name needs a value"
                value=$1
                shift
            fi
            [[ -n $value ]] || stop_usage "$name needs a value"
            case "$name" in
                --jobs)
                    [[ $value =~ ^[1-9][0-9]*$ ]] || stop_usage "$name needs a positive whole number"
                    [[ -z $cargo_jobs ]] || stop_usage "$name was given twice"
                    cargo_jobs=$value
                    ;;
                --only)
                    ids_found=0
                    IFS=, read -r -a ids <<< "$value"
                    for id in "${ids[@]}"; do
                        id=${id//[[:space:]]/}
                        [[ -z $id ]] || { selection+=("$id"); ids_found=1; }
                    done
                    [[ $ids_found == 1 ]] || stop_usage "$name needs at least one step id"
                    ;;
                *)
                    variable=supplied_${name#--}
                    variable=${variable//-/_}
                    [[ -z ${!variable} ]] || stop_usage "$name was given twice"
                    [[ -f $value ]] || stop_usage "SDK configuration not found: $value"
                    printf -v "$variable" '%s' "$(realpath -- "$value")"
                    ;;
            esac
            ;;
        -h) usage; exit 0 ;;
        *) stop_usage "unknown argument '$argument'" ;;
    esac
done
build_profile=${build_profile:-debug}
[[ ${CARGO_NET_OFFLINE:-} != true ]] || offline=1
case "$(uname -s)" in
    Linux) ;;
    *) printf '%s\n' 'validate.sh requires Linux; use scripts\validate.ps1 on Windows.' >&2; exit 2 ;;
esac

root=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
cd -- "$root"
out="$root/_build/validate"
temporary_root="$root/_build/tmp"

# --- Tools: the Cargo lookup of build.sh; rustc must be on PATH for the probe tests. ---
if ! command -v cargo >/dev/null 2>&1; then
    if [[ -x ${CARGO_HOME:-${HOME:-}/.cargo}/bin/cargo ]]; then
        PATH="${CARGO_HOME:-${HOME:-}/.cargo}/bin:$PATH"
    elif [[ -x $root/.tools/cargo/bin/cargo ]]; then
        export CARGO_HOME="$root/.tools/cargo" RUSTUP_HOME=${RUSTUP_HOME:-"$root/.tools/rustup"}
        PATH="$CARGO_HOME/bin:$PATH"
    fi
    export PATH
fi
missing=()
command -v cargo >/dev/null 2>&1 || missing+=('cargo (install Rust with rustup)')
command -v rustc >/dev/null 2>&1 || missing+=('rustc on PATH (the user-command probe tests compile with it)')
if ! command -v python3 >/dev/null 2>&1 ||
    ! python3 -c 'import sys; sys.exit(sys.version_info < (3, 12))' >/dev/null 2>&1; then
    missing+=('Python 3.12+ (python3)')
fi
native_target=
if command -v rustc >/dev/null 2>&1; then native_target=$(rustc -vV | sed -n 's/^host: //p'); fi
cargo_target=${CARGO_TARGET_DIR:-_build}
[[ $cargo_target == /* ]] || cargo_target="$root/$cargo_target"
run_id=$(date +%Y%m%d-%H%M%S)
run_directory="$out/logs/$run_id"
suffix=2
while [[ -e $run_directory ]]; do run_directory="$out/logs/$run_id-$suffix"; suffix=$((suffix + 1)); done
run_id=${run_directory##*/}
offline_flag=()
[[ $offline == 0 ]] || offline_flag=(--offline)
plugin_sdk=$supplied_plugin_sdk
worker_sdk=$supplied_worker_sdk
web_sdk=$supplied_web_sdk
web_sdk_output="$out/web-sdk/release-$run_id"
# SDK bundles this run packages (60-80 MB each run); removed when it ends.
packaged_bundles=()

# --- Prerequisites: each prints the reason a step cannot run, or nothing. ---
prerequisite_tools() {
    local tool missing_tools=()
    for tool in "$@"; do command -v "$tool" >/dev/null 2>&1 || missing_tools+=("$tool"); done
    [[ ${#missing_tools[@]} == 0 ]] || printf 'missing %s\n' "${missing_tools[*]}"
}
prerequisite_c_compiler() { prerequisite_tools cc; }
# The pinned web SDK archives are classic-zlib deflate output; zlib-ng output differs.
prerequisite_classic_zlib() {
    python3 -c 'import sys,zlib; sys.exit("ng" in zlib.ZLIB_RUNTIME_VERSION.lower())' 2>/dev/null ||
        printf '%s\n' 'python3 uses zlib-ng, whose output differs from the pinned web SDK archives; build with FM_BUILD_BACKEND=docker or pass --web-sdk'
}
prerequisite_dav1d() {
    local tools
    tools=$(prerequisite_tools cc make perl pkg-config)
    if [[ -n $tools ]]; then printf '%s (the pinned dav1d build)\n' "$tools"; return; fi
    [[ $(uname -m) == x86_64 ]] || { printf '%s\n' 'the pinned dav1d build is qualified on x86-64 only'; return; }
    # Offline runs can only reuse the pinned tool and source downloads of an earlier run.
    if [[ $offline == 1 ]] && { [[ ! -f $out/raster-ci/native-tools/tool-provenance.json ]] ||
        [[ ! -f $out/raster-ci/native-dav1d/source.json ]]; }; then
        printf 'offline, and the pinned dav1d tools and source are not cached below %s (run once online)\n' "$out/raster-ci"
    fi
}
prerequisite_ffmpeg() {
    if [[ -n ${MIDNIGHT_FFMPEG_PREFIX:-} && -n ${MIDNIGHT_FFMPEG_SOURCE_ARCHIVE:-} && -n ${LIBCLANG_PATH:-} ]]; then return; fi
    [[ -f $root/_build/thumbnailers/ffmpeg/verified-build.json ]] ||
        printf '%s\n' 'no prepared FFmpeg 9.0.2: run ./build.sh first (it builds the pinned FFmpeg) or set MIDNIGHT_FFMPEG_PREFIX, MIDNIGHT_FFMPEG_SOURCE_ARCHIVE and LIBCLANG_PATH'
}
prerequisite_gio() {
    local libraries directory
    libraries=$({ ldconfig -p || /sbin/ldconfig -p; } 2>/dev/null || :)
    [[ $libraries != *libgio-2.0.so.0* ]] || return 0
    for directory in /usr/lib/x86_64-linux-gnu /usr/lib/aarch64-linux-gnu /usr/lib64 /usr/lib; do
        [[ ! -e $directory/libgio-2.0.so.0 ]] || return 0
    done
    printf '%s\n' 'the GIO runtime (libgio-2.0.so.0, Ubuntu package libglib2.0-0t64) is missing'
}
prerequisite_xvfb() { prerequisite_tools xvfb-run xauth; }
# Offline, Cargo resolves the HTML viewer's whole lockfile from its cache, including the
# optional CEF bindings that a build without native CEF does not compile. The dry run
# and a run before the web SDK exists leave the decision to the step.
prerequisite_html_crates() {
    [[ $offline == 1 && $dry_run == 0 && -n $web_sdk ]] || return 0
    cargo --config "$web_sdk" fetch --manifest-path plugins/midnight-viewer-html/Cargo.toml --locked --offline \
        >/dev/null 2>&1 ||
        printf '%s\n' "offline, and the HTML viewer's locked crates (with the optional CEF bindings) are not in the Cargo cache (run once online)"
}

# --- Steps, in CI's job order: SDK jobs, plugin jobs, check, release. ---
step_ids=()
declare -A step_job step_title step_needs step_platform step_prerequisite step_body step_variables step_supplied
add_step() {
    local id=$1 pair
    step_ids+=("$id")
    step_job[$id]=$2
    step_title[$id]=$3
    step_body[$id]="step_${id//-/_}"
    shift 3
    for pair in "$@"; do
        case "$pair" in
            needs=*) step_needs[$id]=${pair#needs=} ;;
            platform=*) step_platform[$id]=${pair#platform=} ;;
            prerequisite=*) step_prerequisite[$id]=${pair#prerequisite=} ;;
            body=*) step_body[$id]=${pair#body=} ;;
            variables=*) step_variables[$id]=${pair#variables=} ;;
            supplied=*) step_supplied[$id]=${pair#supplied=} ;;
            *) printf 'validate.sh: bad step field %s\n' "$pair" >&2; exit 70 ;;
        esac
    done
}

step_sdk_api() {
    export CARGO_TARGET_DIR="$out/sdk-target"
    cargo fetch --manifest-path api/Cargo.toml --locked "${offline_flag[@]}"
    cargo test --manifest-path api/Cargo.toml --all-targets --locked --offline
}
step_sdk_abi() {
    export CARGO_TARGET_DIR="$out/sdk-target"
    bash api/scripts/check-abi.sh "$out/api-abi/linux"
}
step_sdk_package() {
    export CARGO_TARGET_DIR="$out/sdk-target"
    cargo fetch --manifest-path api/Cargo.toml --locked "${offline_flag[@]}"
    bash api/scripts/package.sh
}
step_sdk_protocols() {
    export CARGO_TARGET_DIR="$out/sdk-target"
    for api in api worker-api thumbnail-api filesystem-api image-viewer-api web-viewer-api; do
        cargo fetch --manifest-path "$api/Cargo.toml" --locked "${offline_flag[@]}"
    done
    for api in worker-api thumbnail-api filesystem-api image-viewer-api web-viewer-api; do
        cargo test --manifest-path "$api/Cargo.toml" --workspace --all-targets --locked --offline
    done
}
step_sdk_web_package() {
    cargo fetch --manifest-path web-viewer-api/Cargo.toml --locked "${offline_flag[@]}"
    python3 -B web-viewer-api/scripts/package.py "$web_sdk_output"
}
step_sdk_worker_package() {
    export CARGO_TARGET_DIR="$out/sdk-target"
    for api in api worker-api thumbnail-api filesystem-api image-viewer-api; do
        cargo fetch --manifest-path "$api/Cargo.toml" --locked "${offline_flag[@]}"
    done
    cargo run --manifest-path api/Cargo.toml -p midnight-api-package --bin worker-packages --locked --offline -- "$root" "$out/releases"
}
# The test command of the plugin's scripts/ci.sh (CI's viewers job), then the package
# build that the private test installation needs.
viewer_step() {
    local plugin_root="plugins/midnight-viewer-$plugin" build="$out/viewer-ci" config="$plugin_sdk"
    if [[ -f "$plugin_root/scripts/prepare-sdk.py" ]]; then
        config=$(python3 -B "$plugin_root/scripts/prepare-sdk.py" "$plugin_sdk" "$build")
    fi
    cargo --config "$config" fetch --manifest-path "$plugin_root/Cargo.toml" --locked "${offline_flag[@]}"
    cargo --config "$config" test --manifest-path "$plugin_root/Cargo.toml" \
        --target-dir "$build/target" --target "$native_target" --locked --offline --all-targets
    bash "$plugin_root/scripts/build.sh" "$plugin_sdk" "$build"
}
step_viewer_conformance() {
    local include manifest=plugins/viewer-conformance/Cargo.toml
    include="$(dirname -- "$(dirname -- "$plugin_sdk")")/include"
    export CARGO_TARGET_DIR="$out/conformance-target"
    cargo --config "$plugin_sdk" fetch --manifest-path "$manifest" --locked "${offline_flag[@]}"
    cargo test --config "$plugin_sdk" --manifest-path "$manifest" --locked --offline
    cargo build --config "$plugin_sdk" --manifest-path "$manifest" --release --locked --offline
    bash plugins/viewer-conformance/tests/test-linux.sh \
        "$CARGO_TARGET_DIR/release/libmidnight_viewer_conformance.so" "$include" "$out/conformance/linux"
}
# The image viewer, PDF viewer and image thumbnailer share one build root, so the
# pinned dav1d tools and library are prepared once. Each runs the test commands of
# its scripts/ci.sh (the image viewer also the shared decoder's); the two viewers
# then build the package that the private test installation needs.
raster_step() {
    local plugin_root="plugins/midnight-$plugin" build="$out/raster-ci" jobs=${CARGO_BUILD_JOBS:-$(nproc)} project
    local -a native_option=() projects=("plugins/midnight-$plugin") target_option=(--target "$native_target")
    cargo --config "$worker_sdk" fetch --manifest-path "$plugin_root/Cargo.toml" --locked "${offline_flag[@]}"
    if [[ $offline == 1 ]]; then export MIDNIGHT_IMAGE_NATIVE_OFFLINE=1; native_option=(--offline); fi
    if [[ $plugin != viewer-pdf ]]; then
        source plugins/image-decoder/native/bootstrap-linux.sh "$build/native-tools" "$jobs" "${native_option[@]}"
        source plugins/image-decoder/native/build-linux.sh "$build/native-dav1d" "$jobs" "${native_option[@]}"
        # The native .pc prefix is absolute; an unrelated GUI sysroot must not rewrite it.
        unset PKG_CONFIG_SYSROOT_DIR
    fi
    [[ $plugin != viewer-image ]] || projects=(plugins/image-decoder "$plugin_root")
    [[ $plugin != thumbnail-image ]] || target_option=()
    for project in "${projects[@]}"; do
        cargo --config "$worker_sdk" test --manifest-path "$project/Cargo.toml" \
            --target-dir "$build/target" "${target_option[@]}" --locked --offline --all-targets
    done
    [[ $plugin == thumbnail-image ]] || bash "$plugin_root/scripts/build.sh" "$worker_sdk" "$build"
}
step_thumbnail_video() {
    local manifest=plugins/midnight-thumbnail-video/Cargo.toml state prefix libclang
    # The build's own preparation reuses (or verifies) the pinned FFmpeg the build made.
    state=$(python3 -B scripts/prepare-thumbnail-ffmpeg.py --build-root "$root/_build/thumbnailers/ffmpeg" "${offline_flag[@]}")
    prefix=$(python3 -c 'import json, sys; print(json.load(sys.stdin)["prefix"])' <<< "$state")
    libclang=$(python3 -c 'import json, sys; print(json.load(sys.stdin)["libclang_path"])' <<< "$state")
    export FFMPEG_DIR="$prefix" LIBCLANG_PATH="$libclang"
    export LD_LIBRARY_PATH="$prefix/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    export CARGO_TARGET_DIR="$out/thumbnail-video-target"
    cargo test --config "$worker_sdk" --manifest-path "$manifest" --all-targets --locked --offline
}
step_html_viewer() {
    local manifest=plugins/midnight-viewer-html/Cargo.toml
    export CARGO_TARGET_DIR="$out/html-viewer-target"
    cargo --config "$web_sdk" fetch --manifest-path "$manifest" --locked "${offline_flag[@]}"
    cargo test --config "$web_sdk" --manifest-path "$manifest" --no-default-features --all-targets --locked --offline
}
step_archive() {
    local build="$out/archive-ci" plugin=plugins/midnight-fs-archive
    export CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUST_TEST_THREADS=1
    export CARGO_TARGET_DIR="$build"
    cargo --config "$worker_sdk" fetch --manifest-path "$plugin/Cargo.toml" --locked "${offline_flag[@]}"
    bash "$plugin/scripts/build.sh" "$worker_sdk" "$build" "$build/package/archive" release --test --offline
    MIDNIGHT_ARCHIVE_FIXTURES="$out/archive-fixtures" cargo test --config "$worker_sdk" \
        --manifest-path "$plugin/Cargo.toml" --test worker_fixtures --locked --offline -- --ignored
}
step_filesystem_api() {
    export CARGO_TARGET_DIR="$out/filesystem-library"
    cargo test --manifest-path filesystem-api/Cargo.toml --workspace --locked
}
filesystem_step() {
    local build="$out/filesystem-library" plugin_root="plugins/$plugin_name" package name
    export CARGO_TARGET_DIR="$build"
    if [[ $provider == network ]]; then export GIO_USE_VFS=local; fi
    cargo test --manifest-path "$plugin_root/Cargo.toml" --locked
    if [[ $provider == network ]]; then
        cargo test --manifest-path "$plugin_root/Cargo.toml" --locked --offline \
            local_only_runtime_reports_network_backend_unavailable -- --ignored
    fi
    cargo build --manifest-path "$plugin_root/Cargo.toml" --release --locked --offline
    package="$build/package/$provider.linux"
    rm -rf -- "$package"
    mkdir -p -- "$package"
    cp -- "$build/release/$library" "$package/"
    cp -- "$plugin_root/$manifest" "$package/plugin.toml"
    for name in README.md THIRD-PARTY.md; do cp -- "$plugin_root/$name" "$package/"; done
}
# Mirrors .github/actions/private-test-installation and plugin-packages with the
# packages this run built.
step_test_install() {
    local viewer_packages="$out/viewer-packages" filesystem_packages="$out/filesystem-packages"
    local test_root entry id source destination name
    rm -rf -- "$viewer_packages" "$filesystem_packages"
    mkdir -p -- "$viewer_packages" "$filesystem_packages" "$temporary_root"
    for entry in viewer-basic:viewer-ci/package/midnight-viewer-basic:viewers \
        viewer-source:viewer-ci/package/midnight-viewer-source:viewers \
        viewer-markdown:viewer-ci/package/midnight-viewer-markdown:viewers \
        viewer-image:raster-ci/package/midnight-viewer-image:viewers \
        viewer-pdf:raster-ci/package/midnight-viewer-pdf:viewers \
        filesystem-native:filesystem-library/package/native.linux:filesystems \
        filesystem-network:filesystem-library/package/network.linux:filesystems \
        archive:archive-ci/package/archive:filesystems; do
        IFS=: read -r id source destination <<< "$entry"
        # Only packages built by this run; a skipped step leaves no stale package behind.
        [[ ${step_status[$id]:-} == PASS ]] || continue
        if [[ $destination == viewers ]]; then destination=$viewer_packages; else destination=$filesystem_packages; fi
        cp -R -- "$out/$source" "$destination/"
        printf 'Installed the package of %s: %s\n' "$id" "${source##*/}"
    done
    test_root=$(mktemp -d "$temporary_root/viewer-tests-XXXXXXXX")
    printf '%s\n' "$test_root" > "$run_directory/test-root"
    chmod 700 -- "$test_root"
    cp -R -- "$viewer_packages" "$test_root/viewers"
    cp -R -- "$filesystem_packages" "$test_root/filesystems"
    # Private modes: directories and worker executables 700, other files 600.
    find "$test_root" -type d -exec chmod 700 '{}' '+'
    find "$test_root" -type f -exec chmod 600 '{}' '+'
    for name in midnight-viewer-image midnight-viewer-pdf midnight-fs-archive; do
        find "$test_root" -type f -name "$name" -exec chmod 700 '{}' '+'
    done
    if [[ -f $filesystem_packages/archive/midnight-fs-archive ]]; then chmod 700 -- "$filesystem_packages/archive/midnight-fs-archive"; fi
    {
        printf 'export HOST_TEST_INSTALL_ROOT=%q\n' "$test_root"
        printf 'export HOST_TEST_PLUGIN=%q\n' "$viewer_packages/midnight-viewer-basic/libmidnight_viewer_basic.so"
        printf 'export MIDNIGHT_VIEWER_TEST_INSTALL=%q\n' "$test_root"
        printf 'export MIDNIGHT_NATIVE_TEST_INSTALLATION=%q\n' "$test_root"
        printf 'export MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION=%q\n' "$test_root/midnight-explorer.exe"
        printf 'export MIDNIGHT_PDF_VIEWER_TEST_APPLICATION=%q\n' "$test_root/midnight-explorer.exe"
        printf 'export ARCHIVE_WORKER=%q\n' "$filesystem_packages/archive/midnight-fs-archive"
        printf 'export ARCHIVE_TEST_INSTALL_ROOT=%q\n' "$test_root"
    } > "$run_directory/test-install.env"
    printf 'HOST_TEST_INSTALL_ROOT=%s (mode 700)\n' "$test_root"
}
step_script_tests() { python3 -B -m unittest discover -s scripts/tests -v; }
step_decoder_native_tests() { python3 -B -m unittest discover -s plugins/image-decoder/native/tests -v; }
step_workspace_tests() {
    # CI's gates without its disk-saving artifact retirement; every gate runs.
    python3 -B scripts/ci-workspace-tests.py --keep-test-artifacts --no-fail-fast
}
step_trash_qualification() {
    python3 -B scripts/ci-workspace-tests.py run midnight-platform-linux lib \
        mutation::tests::cross_filesystem_trash_returns_unsupported_and_keeps_source \
        --exact --nocapture 2>&1 | tee "$out/cross-filesystem-trash.log"
    grep -Fq 'QUALIFIED cross_filesystem_trash:' "$out/cross-filesystem-trash.log"
}
step_bad_descriptor() {
    cargo build --manifest-path crates/midnight-plugin-host/tests/fixtures/bad-descriptor/Cargo.toml \
        --locked --offline --target-dir "$out/bad-viewer-descriptor"
}
ignored_step() {
    local arguments
    read -r -a arguments <<< "$selection_words"
    python3 -B scripts/ci-workspace-tests.py run "${arguments[@]}"
}
step_network_metadata_ignored() {
    export MIDNIGHT_NETWORK_TEST_INSTALLATION=$MIDNIGHT_NATIVE_TEST_INSTALLATION
    python3 -B scripts/ci-workspace-tests.py run midnight-native-filesystem-host lib installed_network_metadata --ignored --test-threads=1
}
step_app_native_viewers() {
    local executable probe fixture
    executable=$(python3 -B scripts/ci-workspace-tests.py artifact midnight-explorer bin:me | tail -n 1)
    [[ -n $executable ]] || { printf '%s\n' 'Application test executable not reported' >&2; return 1; }
    probe="$HOST_TEST_INSTALL_ROOT/native-app-test"
    cp -- "$executable" "$probe"
    chmod 700 -- "$probe"
    fixture="$out/native-app-fixture"
    mkdir -p -- "$fixture"
    printf '%s\n' 'First native provider content' > "$fixture/first "$'\xe6\x96\x87\xe6\xa1\xa3'".txt"
    printf '%s\n' 'Second native provider content' > "$fixture/second.txt"
    MIDNIGHT_APP_NATIVE_TEST_FILES=$fixture "$probe" viewers::tests::native_plugin_ --ignored --nocapture --test-threads=1
}
step_archive_streaming_ignored() {
    export ARCHIVE_CAPTURE_INSTALLATION=$HOST_TEST_INSTALL_ROOT
    export MIDNIGHT_ARCHIVE_FIXTURES="$out/archive-fixtures"
    export MIDNIGHT_ARCHIVE_TEST_ZIP="$out/archive-test.zip"
    # The fixture script never replaces a file; this one is regenerated identically.
    rm -f -- "$MIDNIGHT_ARCHIVE_TEST_ZIP"
    python3 -B scripts/archive-test-zip.py "$MIDNIGHT_ARCHIVE_TEST_ZIP"
    # Decoded members are staged on the spool volume (the user's cache first on Linux),
    # which must keep max(1 GiB, 2%) + 64 MiB free.
    df -h -- "$HOME" "$TMPDIR"
    python3 -B scripts/ci-workspace-tests.py run midnight-filesystem-host test:streaming_worker --ignored --test-threads=1
    python3 -B scripts/ci-workspace-tests.py run midnight-filesystem-host lib captured_source_tests:: --ignored --test-threads=1
    python3 -B scripts/ci-workspace-tests.py run midnight-filesystem-host test:discovery_kinds --ignored --test-threads=1
}
# -noreset: an X server resets when its last client disconnects and refuses
# connections during the reset, such as the first one of the next test.
step_x11_dnd() {
    local executable
    executable=$(python3 -B scripts/ci-workspace-tests.py artifact midnight-platform-linux lib)
    cd crates/midnight-platform-linux
    xvfb-run -a -s "-screen 0 1280x1024x24 -nolisten tcp -noreset" \
        env MIDNIGHT_DND_TEST_ISOLATED=1 "$executable" file_drag_drop:: --ignored --test-threads=1
}
step_x11_clipboard() {
    local executable
    executable=$(python3 -B scripts/ci-workspace-tests.py artifact midnight-platform-linux lib)
    cd crates/midnight-platform-linux
    # Without WAYLAND_DISPLAY, arboard cannot reach a real desktop clipboard.
    xvfb-run -a -s "-screen 0 1280x1024x24 -nolisten tcp -noreset" \
        env -u WAYLAND_DISPLAY MIDNIGHT_CLIPBOARD_TEST_ISOLATED=1 "$executable" \
        clipboard::tests::x11_ file_clipboard::x11::tests:: --ignored --test-threads=1
}
step_host_build() {
    if [[ $build_profile == release ]]; then
        cargo build -p midnight-explorer -p midnight-elevation --locked --release
    else
        cargo build -p midnight-explorer -p midnight-elevation --locked
    fi
}
step_smoke_test() {
    local executable="$HOST_TEST_INSTALL_ROOT/me"
    cp -- "$cargo_target/$build_profile/me" "$executable"
    # The root helper is built but never installed in a private test installation.
    [[ -f $cargo_target/$build_profile/midnight-elevated-listing ]] ||
        { printf 'The %s build is missing midnight-elevated-listing\n' "$build_profile" >&2; return 1; }
    chmod 700 -- "$executable"
    "$executable" --smoke-test
    printf '%s\n' 'Installed native filesystem smoke test passed.'
}

# Parent-side hooks: run before a step, or after it passed (they read its log).
before_archive() { rm -rf -- "$out/archive-fixtures"; }
logged_value() {
    local value
    value=$(sed -n "s/^$2//p" "$1" | tail -n 1)
    [[ -n $value ]] || { printf 'the step did not report %s\n' "$2"; return 1; }
    printf '%s\n' "$value"
}
after_sdk_package() {
    local bundle
    bundle=$(logged_value "$1" 'Public API\/SDK release bundle: ') || return 1
    packaged_bundles+=("$bundle")
    plugin_sdk="$bundle/.cargo/config.toml"
    [[ -f $plugin_sdk ]] || { printf 'missing %s\n' "$plugin_sdk"; return 1; }
}
after_sdk_web_package() {
    packaged_bundles+=("$web_sdk_output")
    web_sdk="$web_sdk_output/config.toml"
    [[ -f $web_sdk ]] || { printf 'missing %s\n' "$web_sdk"; return 1; }
}
after_sdk_worker_package() {
    local bundle
    bundle=$(logged_value "$1" 'Public worker API\/SDK release bundle: ') || return 1
    packaged_bundles+=("$bundle")
    worker_sdk="$bundle/.cargo/registry.toml"
    [[ -f $worker_sdk ]] || { printf 'missing %s\n' "$worker_sdk"; return 1; }
}
after_test_install() {
    # shellcheck source=/dev/null
    . "$run_directory/test-install.env"
}
after_bad_descriptor() {
    export HOST_BAD_PLUGIN="$out/bad-viewer-descriptor/debug/libmidnight_host_bad_descriptor.so"
    [[ -f $HOST_BAD_PLUGIN ]] || { printf 'missing %s\n' "$HOST_BAD_PLUGIN"; return 1; }
}

all_viewers='viewer-basic viewer-source viewer-markdown viewer-image viewer-pdf'
add_step sdk-api public-sdk 'Public API and SDK tests'
add_step sdk-abi public-sdk 'C and Rust ABI layout agreement' prerequisite=prerequisite_c_compiler
add_step sdk-package public-sdk 'Build public SDK release bundle' supplied=plugin
add_step sdk-protocols public-worker-sdk 'Test public protocol workspaces'
add_step sdk-web-package public-worker-sdk 'Package the public web SDK' supplied=web prerequisite=prerequisite_classic_zlib
add_step sdk-worker-package public-worker-sdk 'Package public image and worker APIs' supplied=worker
for name in basic source markdown; do
    add_step "viewer-$name" viewers "Independent $name viewer tests and package" needs=sdk-package \
        body=viewer_step "variables=plugin=$name"
done
add_step viewer-conformance viewer-conformance 'Released SDK ABI conformance' needs=sdk-package \
    prerequisite=prerequisite_c_compiler
add_step viewer-image image-viewer 'Independent raster image viewer' needs=sdk-worker-package \
    body=raster_step variables=plugin=viewer-image prerequisite=prerequisite_dav1d
add_step viewer-pdf image-viewer 'Independent PDF viewer' needs=sdk-worker-package \
    body=raster_step variables=plugin=viewer-pdf
add_step thumbnail-image thumbnail-image 'Independent image thumbnailer' needs=sdk-worker-package \
    body=raster_step variables=plugin=thumbnail-image prerequisite=prerequisite_dav1d
add_step thumbnail-video 'not in CI' 'Video thumbnailer (pinned FFmpeg)' needs=sdk-worker-package \
    prerequisite=prerequisite_ffmpeg
add_step html-viewer 'not in CI' 'HTML viewer without native CEF' needs=sdk-web-package \
    prerequisite=prerequisite_html_crates
add_step archive archive-filesystem 'Archive filesystem tests, package and fixtures' needs=sdk-worker-package
add_step filesystem-api filesystem-libraries 'Public filesystem API and SDK'
add_step filesystem-native filesystem-libraries 'Native filesystem plugin and package' body=filesystem_step \
    'variables=provider=native plugin_name=midnight-filesystem-native-linux library=libmidnight_filesystem_native_linux.so manifest=plugin.toml'
add_step filesystem-network filesystem-libraries 'Network filesystem plugin and package' body=filesystem_step \
    'variables=provider=network plugin_name=midnight-filesystem-network library=libmidnight_filesystem_network.so manifest=plugin.linux.toml' \
    prerequisite=prerequisite_gio
add_step test-install scripts-and-lint 'Private test installation'
add_step script-tests scripts-and-lint 'Build script and packaging tests' needs=test-install
add_step decoder-native-tests scripts-and-lint 'Shared image decoder native helper tests'
add_step powershell-tests-7 scripts-and-lint 'Deployment and shortcut tests (PowerShell 7)' platform=windows
add_step powershell-tests-5 scripts-and-lint 'Deployment and shortcut tests (PowerShell 5.1)' platform=windows
add_step workspace-tests check 'Test (workspace gates, doc tests included)' needs=test-install
add_step trash-qualification check 'Qualify Linux cross-filesystem Trash'
add_step bad-descriptor check 'Native viewer ABI rejection fixture'
add_step host-plugin-ignored check 'Native viewer open, viewport, search and close' \
    'needs=test-install bad-descriptor viewer-basic' body=ignored_step \
    'variables=selection_words=midnight-plugin-host lib --ignored'
# As in CI: the native CEF Office/HTML and qualified-product Source acceptance tests are manual.
add_step viewer-engine-ignored check 'Native viewer engine integration' \
    "needs=test-install filesystem-native $all_viewers" body=ignored_step \
    'variables=selection_words=midnight-viewer-engine lib --ignored --nocapture --skip office_preview::native_tests --skip tests::source_native_acceptance'
add_step native-fs-ignored check 'Installed native filesystem browse and reads' \
    'needs=test-install filesystem-native' body=ignored_step \
    'variables=selection_words=midnight-native-filesystem-host test:native --ignored --test-threads=1'
add_step network-metadata-ignored check 'Installed network metadata boundary' 'needs=test-install filesystem-network'
add_step app-native-viewers check 'Application viewers use the native provider' \
    "needs=test-install filesystem-native $all_viewers"
add_step image-process-ignored check 'Restricted image process qualification' \
    'needs=test-install filesystem-native viewer-image' body=ignored_step \
    'variables=selection_words=midnight-image-viewer-host test:native --ignored --test-threads=1'
add_step pdf-process-ignored check 'Restricted PDF process qualification' \
    'needs=test-install filesystem-native viewer-pdf' body=ignored_step \
    'variables=selection_words=midnight-image-viewer-host test:pdf_native --ignored --test-threads=1'
# As in CI: listing archives of millions of members is a manual measurement.
add_step archive-worker-ignored check 'Restricted archive worker listing' 'needs=test-install archive' \
    body=ignored_step 'variables=selection_words=midnight-filesystem-host test:worker --ignored --test-threads=1 --skip real_sandboxed_worker_lists_archives_of_millions_of_members'
add_step archive-nested-ignored check 'Restricted archive workers inside archives' 'needs=test-install archive' \
    body=ignored_step 'variables=selection_words=midnight-filesystem-host test:nested_worker --ignored --test-threads=1'
add_step archive-streaming-ignored check 'Installed archive worker streaming and discovery' 'needs=test-install archive'
add_step x11-dnd check 'Native X11 drag and drop (isolated display)' prerequisite=prerequisite_xvfb
add_step x11-clipboard check 'Native X11 clipboard (isolated display)' prerequisite=prerequisite_xvfb
add_step host-build release "Host build ($build_profile)"
add_step smoke-test release "Installed filesystem smoke test ($build_profile)" 'needs=test-install host-build'

# --- Selection, dependencies and prerequisites ---
declare -A selected=()
if [[ ${#selection[@]} -gt 0 ]]; then
    pending=("${selection[@]}")
    while [[ ${#pending[@]} -gt 0 ]]; do
        id=${pending[-1]}
        unset 'pending[-1]'
        [[ -n ${step_job[$id]:-} ]] || stop_usage "unknown step '$id' (see --dry-run for the step list)"
        [[ -z ${selected[$id]:-} ]] || continue
        selected[$id]=1
        for need in ${step_needs[$id]:-}; do pending+=("$need"); done
    done
fi
is_selected() { [[ ${#selected[@]} == 0 || -n ${selected[$1]:-} ]]; }

declare -A step_status=() step_kind=() step_reason=() step_seconds=() step_counts=()
# Prints "kind<TAB>reason" when the step cannot run; the needs check uses this run's results.
skip_reason() {
    local id=$1 check_needs=$2 need supplied_variable function reason
    if [[ -n ${step_platform[$id]:-} && ${step_platform[$id]} != linux ]]; then
        printf 'platform\tWindows only (CI runs it on windows-latest)\n'; return
    fi
    if [[ -n ${step_supplied[$id]:-} ]]; then
        supplied_variable=supplied_${step_supplied[$id]}_sdk
        if [[ -n ${!supplied_variable} ]]; then
            printf 'supplied\tuses the bundle given with --%s-sdk\n' "${step_supplied[$id]}"; return
        fi
    fi
    if [[ $check_needs == 1 ]]; then
        for need in ${step_needs[$id]:-}; do
            if [[ -n ${step_supplied[$need]:-} ]]; then
                supplied_variable=supplied_${step_supplied[$need]}_sdk
                [[ -z ${!supplied_variable} ]] || continue
            fi
            if [[ ${step_status[$need]:-} != PASS ]]; then
                printf 'needs\tneeds %s (%s)\n' "$need" "${step_status[$need]:-not run}"; return
            fi
        done
    fi
    for function in ${step_prerequisite[$id]:-}; do
        reason=$("$function")
        if [[ -n $reason ]]; then printf 'prerequisite\t%s\n' "$reason"; return; fi
    done
}

apply_step_variables() {
    local assignment
    local -a assignments
    # Each step variable is NAME=VALUE; a value may contain spaces only as the last one.
    case "${step_variables[$1]:-}" in
        selection_words=*) selection_words=${step_variables[$1]#selection_words=}; return ;;
    esac
    read -r -a assignments <<< "${step_variables[$1]:-}"
    for assignment in "${assignments[@]}"; do printf -v "${assignment%%=*}" '%s' "${assignment#*=}"; done
}

# libtest prints one "test result:" line per test executable and doc-test run;
# unittest prints "Ran N tests" and then "OK (skipped=K)" or "FAILED (failures=F, ...)".
count_tests() {
    awk '
        /^test result: [A-Za-z]+\. [0-9]+ passed; [0-9]+ failed; [0-9]+ ignored;/ {
            passed += $4; failed += $6; ignored += $8; found = 1; next
        }
        /^Ran [0-9]+ tests? in / { ran = $2; next }
        ran != "" && /^(OK|FAILED)( \(.*\))?[ \t]*$/ {
            bad = 0; skipped = 0
            if (match($0, /\(.*\)/)) {
                count = split(substr($0, RSTART + 1, RLENGTH - 2), parts, /, */)
                for (part = 1; part <= count; part++) {
                    split(parts[part], pair, "=")
                    if (pair[1] == "failures" || pair[1] == "errors" || pair[1] == "unexpected successes") bad += pair[2]
                    if (pair[1] == "skipped") skipped += pair[2]
                }
            }
            passed += ran - bad - skipped; failed += bad; ignored += skipped; found = 1; ran = ""
        }
        END { if (found) printf "%d %d %d\n", passed, failed, ignored }
    ' "$1" 2>/dev/null || :
}

format_seconds() {
    local seconds=$1
    if ((seconds >= 3600)); then printf '%dh %02dm' $((seconds / 3600)) $((seconds % 3600 / 60))
    elif ((seconds >= 60)); then printf '%dm %02ds' $((seconds / 60)) $((seconds % 60))
    else printf '%ds' "$seconds"; fi
}

# The function's text as written in this script (step functions are [a-z_] names).
print_body() {
    awk -v name="$1" '
        BEGIN { start = "^" name "\\(\\) \\{" }
        !inside && $0 ~ start {
            rest = $0; sub(start, "", rest)
            if (rest ~ /\}[ \t]*$/) { sub(/[ \t]*\}[ \t]*$/, "", rest); sub(/^[ \t]+/, "", rest); print "    " rest; exit }
            inside = 1; next
        }
        inside && /^}/ { exit }
        inside { print }
    ' "${BASH_SOURCE[0]}"
}

if [[ $dry_run == 1 ]]; then
    printf 'Validation plan (dry run) for %s\n' "$root"
    printf 'Platform: linux (%s); profile: %s; offline: %s; strict: %s\n' "${native_target:-unknown}" "$build_profile" "$offline" "$strict"
    printf 'Tools: cargo=%s; rustc=%s; python3=%s\n' "$(command -v cargo || :)" "$(command -v rustc || :)" "$(command -v python3 || :)"
    if [[ ${#missing[@]} -gt 0 ]]; then printf 'Missing tools (a real run stops with exit 127): %s\n' "${missing[*]}"; fi
    printf 'Logs: %s\n' "$run_directory"
    printf 'Environment: TMPDIR=%s; PYTHONUTF8=1; CARGO_REGISTRIES_MIDNIGHT_INDEX=%s%s%s\n' "$temporary_root" \
        "${CARGO_REGISTRIES_MIDNIGHT_INDEX:-https://midnight-explorer.invalid/api-index}" \
        "$([[ $offline == 0 ]] || printf '; CARGO_NET_OFFLINE=true')" \
        "$([[ -z $cargo_jobs ]] || printf '; CARGO_BUILD_JOBS=%s; RUST_TEST_THREADS=%s' "$cargo_jobs" "$cargo_jobs")"
    printf 'Private test installation: a new mode-700 directory below %s (HOST_TEST_INSTALL_ROOT), removed afterwards\n' "$temporary_root"
    printf 'Step variables: root=%s out=%s run_id=%s build_profile=%s cargo_target=%s offline_flag=(%s)\n' \
        "$root" "$out" "$run_id" "$build_profile" "$cargo_target" "${offline_flag[*]:-}"
    printf '  plugin_sdk=%s\n  worker_sdk=%s\n  web_sdk=%s\n' \
        "${plugin_sdk:-<the bundle that sdk-package reports>}" "${worker_sdk:-<the bundle that sdk-worker-package reports>}" \
        "${web_sdk:-$web_sdk_output/config.toml}"
    number=0
    for id in "${step_ids[@]}"; do
        number=$((number + 1))
        is_selected "$id" || continue
        skip=$(skip_reason "$id" 0)
        if [[ -n $skip ]]; then state="skip: ${skip#*$'\t'}"
        elif [[ -n ${step_needs[$id]:-} ]]; then state="run if these pass: ${step_needs[$id]// /, }"
        else state=run; fi
        printf '\n%2d. %s - %s [CI job: %s] (%s)\n' "$number" "$id" "${step_title[$id]}" "${step_job[$id]}" "$state"
        [[ -z ${step_variables[$id]:-} ]] || printf '      variables: %s\n' "${step_variables[$id]}"
        if [[ -z ${step_platform[$id]:-} || ${step_platform[$id]} == linux ]]; then
            if [[ $id == test-install ]]; then
                printf '      %s\n' '# Its exported variables reach every later step.'
            fi
            print_body "${step_body[$id]}" | sed 's/^/  /'
        fi
    done
    exit 0
fi

if [[ ${#missing[@]} -gt 0 ]]; then
    for item in "${missing[@]}"; do printf 'validate.sh: missing %s\n' "$item" >&2; done
    exit 127
fi

# --- Run ---
mkdir -p -- "$temporary_root" "$run_directory"
export TMPDIR="$temporary_root" PYTHONUTF8=1
export CARGO_REGISTRIES_MIDNIGHT_INDEX=${CARGO_REGISTRIES_MIDNIGHT_INDEX:-https://midnight-explorer.invalid/api-index}
[[ $offline == 0 ]] || export CARGO_NET_OFFLINE=true
# Without --jobs, Cargo, libtest and the native builds use every logical CPU.
[[ -z $cargo_jobs ]] || export CARGO_BUILD_JOBS=$cargo_jobs RUST_TEST_THREADS=$cargo_jobs
remove_owned() {
    if [[ -L $1 || ! -d $1 ]]; then return; fi
    if rm -rf -- "$1"; then printf 'Removed %s %s\n' "$2" "$1"
    else printf 'validate.sh: could not remove %s %s\n' "$2" "$1" >&2; fi
}
cleanup() {
    local test_root bundle
    if [[ -f $run_directory/test-root ]]; then
        test_root=$(cat -- "$run_directory/test-root")
        case "$test_root" in
            "$temporary_root"/viewer-tests-*) remove_owned "$test_root" 'the private test installation' ;;
        esac
    fi
    # Only bundles that this run's packagers reported, below this checkout's _build.
    for bundle in "${packaged_bundles[@]}"; do
        case "$bundle" in
            "$root/_build/"*) remove_owned "$bundle" 'the SDK bundle packaged for this run' ;;
        esac
    done
}
trap cleanup EXIT
trap 'exit 130' INT TERM

started=$(date +%s)
printf 'Validating %s on linux (%s); profile %s; logs in %s\n' "$root" "$native_target" "$build_profile" "$run_directory"
number=0
total=${#step_ids[@]}
for id in "${step_ids[@]}"; do
    number=$((number + 1))
    is_selected "$id" || continue
    log=$(printf '%s/%02d-%s.log' "$run_directory" "$number" "$id")
    skip=$(skip_reason "$id" 1)
    if [[ -n $skip ]]; then
        step_status[$id]=SKIPPED
        step_kind[$id]=${skip%%$'\t'*}
        step_reason[$id]=${skip#*$'\t'}
        printf '== [%d/%d] %s: SKIPPED (%s)\n' "$number" "$total" "$id" "${step_reason[$id]}"
        continue
    fi
    printf '\n== [%d/%d] %s: %s [CI job: %s]\n' "$number" "$total" "$id" "${step_title[$id]}" "${step_job[$id]}"
    step_started=$(date +%s)
    status=PASS
    reason=
    if declare -F "before_${id//-/_}" >/dev/null; then "before_${id//-/_}"; fi
    set +e
    (
        set -euo pipefail
        apply_step_variables "$id"
        "${step_body[$id]}"
    ) < /dev/null 2>&1 | tee -- "$log"
    code=${PIPESTATUS[0]}
    set -e
    if [[ $code != 0 ]]; then
        status=FAIL
        reason="exit code $code"
    elif declare -F "after_${id//-/_}" >/dev/null; then
        # Runs in this shell, so the variables it sets reach the later steps.
        if ! "after_${id//-/_}" "$log" > "$run_directory/hook.out" 2>&1; then
            status=FAIL
            reason=$(cat -- "$run_directory/hook.out")
        fi
        rm -f -- "$run_directory/hook.out"
    fi
    step_status[$id]=$status
    step_reason[$id]=$reason
    step_seconds[$id]=$(($(date +%s) - step_started))
    step_counts[$id]=$(count_tests "$log")
    printf -- '-- %s: %s in %s%s\n' "$id" "$status" "$(format_seconds "${step_seconds[$id]}")" "${reason:+ ($reason)}"
done

# --- Summary ---
summary="$run_directory/summary.txt"
{
    printf '\nValidation summary: linux (%s), profile %s, started %s\n' "$native_target" "$build_profile" "$(date -d "@$started" '+%Y-%m-%d %H:%M:%S')"
    printf '%-2s %-26s %-7s %8s %7s %6s %7s  %s\n' '#' Step Result Time Passed Failed Ignored 'CI job / reason'
    passed_steps=0; failed_steps=0; skipped_steps=0; strict_skips=0; ci_skips=; ci_skip_count=0
    tests_passed=0; tests_failed=0; tests_ignored=0
    number=0
    for id in "${step_ids[@]}"; do
        number=$((number + 1))
        status=${step_status[$id]:-}
        [[ -n $status ]] || continue
        case "$status" in
            PASS) passed_steps=$((passed_steps + 1)) ;;
            FAIL) failed_steps=$((failed_steps + 1)) ;;
            SKIPPED)
                skipped_steps=$((skipped_steps + 1))
                case "${step_kind[$id]}" in
                    platform|supplied) ;;
                    *)
                        strict_skips=$((strict_skips + 1))
                        if [[ ${step_job[$id]} != 'not in CI' ]]; then
                            ci_skips+="${ci_skips:+, }$id"
                            ci_skip_count=$((ci_skip_count + 1))
                        fi
                        ;;
                esac
                ;;
        esac
        passed=; failed=; ignored=
        if [[ -n ${step_counts[$id]:-} ]]; then
            read -r passed failed ignored <<< "${step_counts[$id]}"
            tests_passed=$((tests_passed + passed)); tests_failed=$((tests_failed + failed)); tests_ignored=$((tests_ignored + ignored))
        fi
        time=-
        [[ $status == SKIPPED ]] || time=$(format_seconds "${step_seconds[$id]}")
        printf '%2d %-26s %-7s %8s %7s %6s %7s  %s\n' "$number" "$id" "$status" "$time" "$passed" "$failed" "$ignored" \
            "${step_reason[$id]:-${step_job[$id]}}"
    done
    printf 'Steps: %d passed, %d failed, %d skipped in %s. Tests: %d passed, %d failed, %d ignored.\n' \
        "$passed_steps" "$failed_steps" "$skipped_steps" "$(format_seconds $(($(date +%s) - started)))" \
        "$tests_passed" "$tests_failed" "$tests_ignored"
    if [[ ${#selected[@]} -gt 0 ]]; then
        printf 'Only the selected steps and the steps they need ran (--only %s).\n' "$(IFS=,; printf '%s' "${selection[*]}")"
    fi
    # Without --strict such a run still passes; this line keeps the gap in CI coverage visible.
    if [[ $ci_skip_count -gt 0 ]]; then
        printf 'WARNING: %d CI step(s) SKIPPED for a missing prerequisite or needed step: %s (use --strict to fail)\n' \
            "$ci_skip_count" "$ci_skips"
    fi
    if [[ $failed_steps -gt 0 ]]; then printf '%s\n' 'VALIDATION FAILED'
    elif [[ $strict == 1 && $strict_skips -gt 0 ]]; then printf 'VALIDATION FAILED: --strict and %d step(s) skipped\n' "$strict_skips"
    else printf '%s\n' 'VALIDATION PASSED'; fi
    printf 'Logs: %s\n' "$run_directory"
} | tee -- "$summary"
if grep -q '^VALIDATION FAILED' "$summary"; then exit 1; fi
exit 0
