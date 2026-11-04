#!/usr/bin/env bash
set -euo pipefail

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
maximum_source_bytes=100000
checked=0
oversized=0
while IFS= read -r -d '' source; do
    size=$(wc -c < "$source")
    checked=$((checked + 1))
    if ((size > maximum_source_bytes)); then
        printf '%s: %s bytes exceeds %s\n' "$source" "$size" "$maximum_source_bytes" >&2
        oversized=$((oversized + 1))
    fi
done < <(find "$project_root/crates" "$project_root/scripts" -type f \
    \( -name '*.rs' -o -name '*.slint' -o -name '*.ps1' -o -name '*.sh' \) -print0)
if ((oversized > 0)); then
    exit 1
fi
printf 'Source size: %s files checked, all <= %s bytes\n' "$checked" "$maximum_source_bytes"
