#!/usr/bin/env bash
set -euo pipefail
plugin_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
sdk_config="$(realpath -- "$1")"
build_root="$(realpath -ms -- "$2")"
# Generated output lies strictly below _build: for a path inside this plugin, or inside
# the repository that holds it in plugins/NAME, their own _build; otherwise the first
# _build of the path (a custom directory). That _build and every folder below it must
# not be a symlink. The checkout and the folders above it may be links (Fedora
# Silverblue's /home -> /var/home, a linked parent folder): a checkout is recognised
# however the path spells it, at its shallowest place, so a link back to it is inspected.
generated_output_problem() {
    local path=$1 ancestors=() index top anchor=0 repository=
    [[ $path == /* ]] || { printf 'must be an absolute path: %s\n' "$1"; return; }
    [[ ${plugin_root%/*} != */plugins ]] || repository=${plugin_root%/*/*}
    while [[ $path != / ]]; do ancestors+=("$path"); path=${path%/*}; path=${path:-/}; done
    top=${#ancestors[@]}
    for (( index=top-1; index>=0; index-- )); do
        if [[ ${ancestors[index]} -ef $plugin_root || ( -n $repository && ${ancestors[index]} -ef $repository ) ]]; then
            top=$index
            break
        fi
    done
    if (( top < ${#ancestors[@]} )); then
        if (( top >= 2 )) && [[ ${ancestors[top-1]##*/} == _build ]]; then
            anchor=$((top - 1))
        elif (( top >= 4 )) && [[ -n $repository && ${ancestors[top]} -ef $repository && ${ancestors[top-1]##*/} == plugins &&
                ${ancestors[top-2]} -ef $plugin_root && ${ancestors[top-3]##*/} == _build ]]; then
            anchor=$((top - 3))
        fi
    else
        for (( index=${#ancestors[@]}-1; index>0; index-- )); do
            if [[ ${ancestors[index]##*/} == _build ]]; then anchor=$index; break; fi
        done
    fi
    if (( anchor == 0 )); then printf 'must be under _build: %s\n' "$1"; return; fi
    for (( index=0; index<=anchor; index++ )); do
        if [[ -L ${ancestors[index]} ]]; then printf 'must not traverse symlinks: %s\n' "${ancestors[index]}"; return; fi
    done
}
problem=$(generated_output_problem "$build_root")
[[ -z $problem ]] || { echo "Generated output $problem" >&2; exit 2; }
decoder_root="$(cd -- "$plugin_root/../image-decoder" && pwd)"
native_offline=()
[[ ${MIDNIGHT_IMAGE_NATIVE_OFFLINE:-0} != 1 ]] || native_offline+=(--offline)
native_jobs=${CARGO_BUILD_JOBS:-$(nproc)}
source "$plugin_root/../image-decoder/native/bootstrap-linux.sh" "$build_root/native-tools" "$native_jobs" "${native_offline[@]}"
source "$plugin_root/../image-decoder/native/build-linux.sh" "$build_root/native-dav1d" "$native_jobs" "${native_offline[@]}"
# The native .pc prefix is absolute; an unrelated GUI sysroot must not rewrite it.
unset PKG_CONFIG_SYSROOT_DIR
# The package build selects the native --target; test and lint with the same
# target so a Debug package build reuses the test dependency build.
native_target="$(rustc -vV | sed -n 's/^host: //p')"
for project in "$decoder_root" "$plugin_root"; do
    cargo fmt --manifest-path "$project/Cargo.toml" -- --check
    cargo --config "$sdk_config" test --manifest-path "$project/Cargo.toml" \
        --target-dir "$build_root/target" --target "$native_target" --locked --offline --all-targets
    cargo clippy --config "$sdk_config" --manifest-path "$project/Cargo.toml" \
        --target-dir "$build_root/target" --target "$native_target" --locked --offline --all-targets -- -D warnings
done
bash "$plugin_root/scripts/build.sh" "$sdk_config" "$build_root"
