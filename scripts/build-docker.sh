#!/bin/sh
set -eu

case "${1-debug}" in
    debug) build_profile=debug ;;
    release) build_profile=release ;;
    *) printf '%s\n' 'Usage: sh scripts/build-docker.sh [debug|release]' >&2; exit 2 ;;
esac

if ! command -v docker >/dev/null 2>&1; then
    printf '%s\n' 'Native build dependencies are missing. Install/start Docker with access for your user, or install the native prerequisites listed in README.md.' >&2
    exit 1
fi

build_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
mkdir -p "$build_root/_build/tmp" "$build_root/_build/docker/cargo" "$build_root/_build/docker/home"
base_image=ubuntu:24.04
# Cache the image by setup recipe, base image tag, and pinned toolchain.
image_key=$({ printf '%s\n' "$base_image"; cat "$build_root/scripts/docker-install-deps.sh" "$build_root/rust-toolchain.toml"; } | sha256sum)
build_image="midnight-explorer-build:${image_key%% *}"
set --
if [ -n "${FM_DOCKER_DNS:-}" ]; then
    set -- --dns "$FM_DOCKER_DNS"
fi
if ! docker image inspect "$build_image" >/dev/null 2>&1; then
    printf '%s\n' 'Installing Docker build dependencies (no host sudo required).'
    setup_container=$(docker create "$@" \
        --mount "type=bind,src=$build_root,dst=/workspace,readonly" \
        --env RUSTUP_HOME=/opt/rustup --env CARGO_HOME=/opt/cargo \
        --env PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
        "$base_image" sh /workspace/scripts/docker-install-deps.sh)
    trap 'docker rm -f "$setup_container" >/dev/null 2>&1 || :' EXIT
    trap 'exit 1' HUP INT TERM
    docker start --attach "$setup_container"
    # docker start --attach does not reliably propagate the process exit status.
    setup_status=$(docker inspect --format '{{.State.ExitCode}}' "$setup_container")
    if [ "$setup_status" -ne 0 ]; then
        exit "$setup_status"
    fi
    docker commit "$setup_container" "$build_image" >/dev/null
    docker rm "$setup_container" >/dev/null
    trap - EXIT HUP INT TERM
fi

# Keep source read-only and write caches/artifacts as the invoking user.
set -- "$@" "$build_image" cargo build -p midnight-explorer --locked
if [ "$build_profile" = release ]; then
    set -- "$@" --release
fi
exec docker run --rm --user "$(id -u):$(id -g)" \
    --mount "type=bind,src=$build_root,dst=/workspace,readonly" \
    --mount "type=bind,src=$build_root/_build,dst=/workspace/_build" \
    --workdir /workspace \
    --env HOME=/workspace/_build/docker/home \
    --env CARGO_HOME=/workspace/_build/docker/cargo \
    --env CARGO_TARGET_DIR=/workspace/_build \
    --env TMPDIR=/workspace/_build/tmp \
    "$@"
