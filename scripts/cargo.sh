#!/bin/sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
mkdir -p -- "$project_root/_build/tmp"
TMPDIR="$project_root/_build/tmp"
export TMPDIR

exec cargo "$@"
