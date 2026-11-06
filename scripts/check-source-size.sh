#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
maximum_source_bytes=100000
checked=0
oversized=0
while IFS= read -r -d '' source; do
    case "$source" in
        *.rs|*.slint|*.ps1|*.sh|*.bat|*.py|*.c|*.h) ;;
        *) continue ;;
    esac
    size=$(wc -c < "$source")
    checked=$((checked + 1))
    if ((size > maximum_source_bytes)); then
        printf '%s: %s bytes exceeds %s\n' "$source" "$size" "$maximum_source_bytes" >&2
        oversized=$((oversized + 1))
    fi
done < <(
    find "$project_root" -maxdepth 1 -type f -print0
    for directory in crates scripts api plugins; do
        if [[ -d "$project_root/$directory" ]]; then
            find "$project_root/$directory" -type f -print0
        fi
    done
)
if ((oversized > 0)); then
    exit 1
fi
printf 'Source size: %s files checked, all <= %s bytes\n' "$checked" "$maximum_source_bytes"
