#!/bin/sh
# Runs as root inside the temporary dependency container, never on the host.
set -eu
workspace=${MIDNIGHT_DOCKER_WORKSPACE:-/workspace}
scratch=${TMPDIR:-/tmp}

apt-get update --error-on=any
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    build-essential pkg-config python3 curl ca-certificates gnupg xz-utils perl libclang-dev cmake ninja-build \
    libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libxkbcommon-x11-0 libx11-dev \
    libxcb1-dev libwayland-dev libudev-dev
rm -rf /var/lib/apt/lists/*

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o "$scratch/rustup-init.sh"
sh "$scratch/rustup-init.sh" -y --profile minimal --default-toolchain none
rm "$scratch/rustup-init.sh"
# The build later runs as the invoking user with this RUSTUP_HOME read-only, so
# the pinned toolchain and its components must be installed here explicitly:
# current rustup no longer installs the active toolchain from `rustup show`.
python3 - "$workspace/rust-toolchain.toml" > "$scratch/toolchain-arguments" <<'PY'
import sys
import tomllib

with open(sys.argv[1], "rb") as stream:
    toolchain = tomllib.load(stream)["toolchain"]
arguments = [toolchain["channel"], "--profile", toolchain.get("profile", "minimal")]
for component in toolchain.get("components", []):
    arguments += ["--component", component]
print("\n".join(arguments))
PY
set --
while IFS= read -r argument; do set -- "$@" "$argument"; done < "$scratch/toolchain-arguments"
rm "$scratch/toolchain-arguments"
rustup toolchain install "$@"
chmod -R a+rX /opt/rustup /opt/cargo
