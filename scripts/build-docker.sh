#!/bin/sh
set -eu

usage() {
    printf '%s\n' 'Usage: sh scripts/build-docker.sh [debug|release] [--dry-run]'
}

print_command() {
    printf 'Command:'
    printf ' [%s]' "$@"
    printf '\n'
}

build_profile=debug
profile_chosen=0
dry_run=0
for argument in "$@"; do
    case "$argument" in
        debug|release)
            if [ "$profile_chosen" = 1 ]; then usage >&2; exit 2; fi
            build_profile=$argument
            profile_chosen=1
            ;;
        --dry-run) dry_run=1 ;;
        --help|-h) usage; exit 0 ;;
        *) usage >&2; exit 2 ;;
    esac
done

if [ "$dry_run" = 0 ] && ! command -v docker >/dev/null 2>&1; then
    printf '%s\n' 'Native build dependencies are missing. Install/start Docker with access for your user, or install the native prerequisites listed in README.md.' >&2
    exit 1
fi

build_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
# Docker parses each --mount argument as CSV after the shell preserves its argv.
mount_root=$(printf '%s' "$build_root" | sed 's/"/""/g')
base_image=ubuntu:24.04
tool_path=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
# Cache the image by setup recipe, base image tag, and pinned toolchain.
image_key=$({ printf '%s\n' "$base_image"; cat "$build_root/scripts/docker-install-deps.sh" "$build_root/rust-toolchain.toml"; } | sha256sum)
build_image="midnight-explorer-build:${image_key%% *}"
build_user="$(id -u):$(id -g)"

create_setup_container() {
    set -- docker create
    if [ -n "${FM_DOCKER_DNS:-}" ]; then
        set -- "$@" --dns "$FM_DOCKER_DNS"
    fi
    set -- "$@" \
        --mount "type=bind,\"src=$mount_root\",dst=/workspace,readonly" \
        --env RUSTUP_HOME=/opt/rustup --env CARGO_HOME=/opt/cargo \
        --env "PATH=$tool_path" \
        "$base_image" sh /workspace/scripts/docker-install-deps.sh
    if [ "$dry_run" = 1 ]; then
        print_command "$@"
    else
        "$@"
    fi
}

if [ "$dry_run" = 1 ]; then
    printf '%s\n' 'Backend: Docker runs the full build.sh chain with FM_BUILD_BACKEND=native.'
    print_command mkdir -p "$build_root/_build/tmp" \
        "$build_root/.tools/docker/cargo" "$build_root/.tools/docker/home"
    print_command docker image inspect "$build_image"
    printf '%s\n' 'If the cached image is absent, set up and check a temporary container:'
    create_setup_container
    print_command docker start --attach '<setup-container>'
    print_command docker inspect --format '{{.State.ExitCode}}' '<setup-container>'
    printf '%s\n' 'Commit only after setup exits successfully; force-remove the container on failure.'
    print_command docker commit '<setup-container>' "$build_image"
    print_command docker rm '<setup-container>'
else
    mkdir -p "$build_root/_build/tmp" \
        "$build_root/.tools/docker/cargo" "$build_root/.tools/docker/home"
    if ! docker image inspect "$build_image" >/dev/null 2>&1; then
        printf '%s\n' 'Installing Docker build dependencies (no host sudo required).'
        setup_container=$(create_setup_container)
        trap 'docker rm -f "$setup_container" >/dev/null 2>&1 || :' EXIT
        trap 'exit 1' HUP INT TERM
        docker start --attach "$setup_container"
        # docker start --attach does not reliably propagate the process exit status.
        setup_status=$(docker inspect --format '{{.State.ExitCode}}' "$setup_container")
        case "$setup_status" in
            0) ;;
            [1-9]|[1-9][0-9]|1[0-9][0-9]|2[0-4][0-9]|25[0-5]) exit "$setup_status" ;;
            *) printf '%s\n' 'Docker returned an invalid dependency setup exit status.' >&2; exit 1 ;;
        esac
        docker commit "$setup_container" "$build_image" >/dev/null
        docker rm "$setup_container" >/dev/null
        trap - EXIT HUP INT TERM
    fi
fi

# Keep source read-only and write caches/artifacts as the invoking user.
set -- docker run --rm --user "$build_user" \
    --mount "type=bind,\"src=$mount_root\",dst=/workspace,readonly" \
    --mount "type=bind,\"src=$mount_root/_build\",dst=/workspace/_build" \
    --mount "type=bind,\"src=$mount_root/.tools\",dst=/workspace/.tools" \
    --workdir /workspace \
    --env FM_BUILD_BACKEND=native \
    --env HOME=/workspace/.tools/docker/home \
    --env CARGO_HOME=/workspace/.tools/docker/cargo \
    --env RUSTUP_HOME=/opt/rustup \
    --env "PATH=$tool_path" \
    --env CARGO_TARGET_DIR=/workspace/_build \
    --env TMPDIR=/workspace/_build/tmp
if [ -n "${FM_DOCKER_DNS:-}" ]; then
    set -- "$@" --dns "$FM_DOCKER_DNS"
fi
set -- "$@" "$build_image" sh /workspace/build.sh "$build_profile"
if [ "$dry_run" = 1 ]; then
    print_command "$@"
    exit 0
fi
exec "$@"
