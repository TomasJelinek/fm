#!/bin/sh
# Install the toolchain that rust-toolchain.toml pins, with its profile and components.
# Current rustup no longer installs a pinned toolchain through `rustup show`, and a
# Cargo or rustc proxy installs nothing when auto-install is disabled
# (RUSTUP_AUTO_INSTALL=0 or `rustup set auto-install disable`).
# Used by build.sh after it bootstraps rustup and by the Docker image setup.
set -eu
if [ "$#" -ne 1 ]; then
    printf '%s\n' 'Usage: sh scripts/install-rust-toolchain.sh RUST_TOOLCHAIN_TOML' >&2
    exit 2
fi
if ! command -v python3 >/dev/null 2>&1; then
    printf '%s\n' 'Python 3.11 or newer (python3) is required to read rust-toolchain.toml.' >&2
    exit 1
fi
arguments=$(python3 - "$1" <<'PY'
import re
import sys
import tomllib

with open(sys.argv[1], "rb") as stream:
    toolchain = tomllib.load(stream)["toolchain"]
names = [toolchain["channel"], toolchain.get("profile", "minimal"), *toolchain.get("components", [])]
if not all(isinstance(name, str) and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", name) for name in names):
    raise SystemExit("rust-toolchain.toml names an invalid channel, profile or component")
arguments = [names[0], "--profile", names[1]]
for component in names[2:]:
    arguments += ["--component", component]
print("\n".join(arguments))
PY
)
set --
while IFS= read -r argument; do set -- "$@" "$argument"; done <<EOF
$arguments
EOF
exec rustup toolchain install "$@"
