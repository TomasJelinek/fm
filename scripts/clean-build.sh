#!/bin/sh
# Scope cleanup to the repository output tree, never a Cargo target override.
set -eu

dry_run=0
case "$#:${1-}" in
    0:) ;;
    1:--dry-run) dry_run=1 ;;
    *) printf '%s\n' 'Usage: sh scripts/clean-build.sh [--dry-run]' >&2; exit 2 ;;
esac
case "$(uname -s)" in
    Linux) ;;
    *) printf '%s\n' 'Build cleanup requires Linux; macOS support is pending.' >&2; exit 2 ;;
esac
build_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
output="$build_root/_build"
if [ -L "$output" ]; then
    printf '%s\n' 'Cannot clean a symlinked _build directory.' >&2
    exit 1
fi
if [ -e "$output" ] && [ ! -d "$output" ]; then
    printf '%s\n' 'Cannot clean: _build is not a directory.' >&2
    exit 1
fi
if [ ! -d "$output" ]; then
    printf '%s\n' 'No repository _build output to clean.'
    exit 0
fi
sh "$build_root/scripts/check-clean-mounts-linux.sh" "$output"
if [ "$dry_run" = 1 ]; then
    find "$output" -mindepth 1 -maxdepth 1 ! -name docker -exec printf 'Would remove: %s\n' {} \;
else
    # Linux mount metadata was checked before any removal. Unlink individual
    # entries and remove only empty directories; never use recursive rm.
    # Select roots before walking: an unreadable retained cache is never opened.
    for cleanup_entry in "$output"/* "$output"/.[!.]* "$output"/..?*; do
        if [ ! -e "$cleanup_entry" ] && [ ! -L "$cleanup_entry" ]; then continue; fi
        if [ "$cleanup_entry" = "$output/docker" ]; then continue; fi
        find "$cleanup_entry" -xdev -depth -exec sh -c '
            status=0
            for entry do
                if [ -d "$entry" ] && [ ! -L "$entry" ]; then
                    rmdir -- "$entry" || status=$?
                else
                    rm -f -- "$entry" || status=$?
                fi
            done
            exit "$status"
        ' clean-build {} +
    done
    printf '%s\n' 'Cleaned repository _build output; .tools and legacy Docker cache preserved.'
fi
