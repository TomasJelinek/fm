#!/bin/sh
# Runs as root inside the temporary dependency container, never on the host.
set -eu

apt-get update --error-on=any
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    build-essential pkg-config python3 curl ca-certificates gnupg xz-utils perl libclang-dev cmake ninja-build \
    libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libxkbcommon-x11-0 libx11-dev \
    libxcb1-dev libwayland-dev libudev-dev
rm -rf /var/lib/apt/lists/*

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh
sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain none
rm /tmp/rustup-init.sh
mkdir -p /opt/toolchain
cp /workspace/rust-toolchain.toml /opt/toolchain/
cd /opt/toolchain
rustup show
chmod -R a+rX /opt/rustup /opt/cargo
