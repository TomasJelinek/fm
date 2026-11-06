#!/usr/bin/env bash
set -euo pipefail
api_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
build_root="${1:-$api_root/../_build/api-abi/linux}"
header_directory="${2:-$api_root/midnight-plugin-api/include}"
mkdir -p "$build_root"
cc -std=c11 -Wall -Wextra -Werror -I "$header_directory" \
    "$api_root/midnight-plugin-api/tests/c/abi_layout.c" -o "$build_root/abi-layout"
"$build_root/abi-layout" > "$build_root/c-layout.txt"
cargo run --manifest-path "$api_root/Cargo.toml" -p midnight-plugin-api \
    --example abi_layout --locked --quiet > "$build_root/rust-layout.txt"
diff -u "$build_root/c-layout.txt" "$build_root/rust-layout.txt"
cat "$build_root/c-layout.txt"
printf '%s\n' 'C and Rust match: every public struct size, alignment and field offset.'
