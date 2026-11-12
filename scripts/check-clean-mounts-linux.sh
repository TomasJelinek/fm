#!/bin/sh
# Read-only guard; an alternate mount table supports offline inspection/tests.
set -eu
if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    printf '%s\n' 'Usage: check-clean-mounts-linux.sh OUTPUT_DIRECTORY [MOUNTINFO_FILE]' >&2
    exit 2
fi
output=${1%/}
mountinfo=${2:-/proc/self/mountinfo}
case "$output" in
    /*) ;;
    *) printf '%s\n' 'Cleanup output path must be absolute.' >&2; exit 2 ;;
esac
if [ ! -r "$mountinfo" ]; then
    printf 'Cannot validate cleanup mounts: %s\n' "$mountinfo" >&2
    exit 1
fi
seen=0
while IFS=' ' read -r mount_id parent_id device mount_root encoded_mount remainder; do
    if [ -z "$encoded_mount" ]; then
        printf '%s\n' 'Cannot validate malformed Linux mount metadata.' >&2
        exit 1
    fi
    seen=1
    # Kernel mountinfo escapes space, tab, newline and backslash as octal.
    # A sentinel prevents command substitution from stripping path newlines.
    mount_path=$(printf '%bX' "$encoded_mount")
    mount_path=${mount_path%X}
    case "$mount_path" in
        "$output/docker"|"$output/docker/"*) ;; # Preserved cache is outside deletion scope.
        "$output"|"$output/"*)
            printf 'Cannot clean mounted output: %s\n' "$mount_path" >&2
            exit 1
            ;;
    esac
done < "$mountinfo"
if [ "$seen" = 0 ]; then
    printf '%s\n' 'Cannot validate an empty Linux mount table.' >&2
    exit 1
fi
