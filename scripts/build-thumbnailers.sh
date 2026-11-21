#!/usr/bin/env bash
set -euo pipefail
if (( $# < 7 || $# > 8 )); then
    echo 'usage: build-thumbnailers.sh API_CONFIG DESTINATION BUILD_ROOT NOTICES_ROOT FFMPEG_PREFIX FFMPEG_SOURCE_ARCHIVE LIBCLANG_PATH [debug|release]' >&2
    exit 2
fi
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
api_config=$(realpath "$1")
destination=$(realpath -ms "$2")
build_root=$(realpath -ms "$3")
notices_root=$(realpath "$4")
# Generated output lies below _build: the checkout's own for a path in the checkout,
# otherwise the first _build of a custom target directory. That _build and everything
# below it must not be a symlink. The checkout and the folders above it may be reached
# through links (Fedora Silverblue's /home -> /var/home, a linked parent folder); the
# checkout is recognised as the same directory however the path spells it.
for target in "$destination" "$build_root"; do
    ancestors=() parent=$target
    while [[ $parent != / ]]; do ancestors+=("$parent"); parent=$(dirname -- "$parent"); done
    top=${#ancestors[@]}
    for (( index=top-1; index>=0; index-- )); do
        if [[ ${ancestors[index]} -ef $repository ]]; then top=$index; break; fi
    done
    anchor=0
    for (( index=top-1; index>0; index-- )); do
        if [[ ${ancestors[index]##*/} == _build ]]; then anchor=$index; break; fi
        (( top == ${#ancestors[@]} )) || break
    done
    (( anchor > 0 )) || { echo "Generated output must be under _build: $target" >&2; exit 2; }
    for (( index=0; index<=anchor; index++ )); do
        [[ ! -L ${ancestors[index]} ]] || { echo "Generated output must not traverse symlinks: ${ancestors[index]}" >&2; exit 2; }
    done
done
profile=${8:-debug}
[[ $profile == debug || $profile == release ]] || exit 2
plugins=(midnight-thumbnail-image midnight-thumbnail-video)
[[ -n $5 && -n $6 && -n $7 ]] || { echo 'Both thumbnailers require FFMPEG_PREFIX, FFMPEG_SOURCE_ARCHIVE and LIBCLANG_PATH' >&2; exit 2; }
FFMPEG_DIR=$(realpath "$5")
LIBCLANG_PATH=$(realpath "$7")
export FFMPEG_DIR LIBCLANG_PATH
source_archive=$(realpath "$6")
echo "8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e  $source_archive" | sha256sum --check --status
mkdir -p "$(dirname -- "$destination")" "$build_root"
# Assemble a complete package beside the destination and swap directories: files
# from an earlier FFmpeg or dependency set never survive, and a running worker
# keeps its mapped libraries (they are renamed away, never truncated in place).
staging=$(mktemp -d -- "$(dirname -- "$destination")/.thumbnailers-build.XXXXXXXX")
clean_staging=1
trap 'if [[ $clean_staging == 1 ]]; then rm -rf -- "$staging"; fi' EXIT
package="$staging/package"
mkdir -- "$package"
native_offline=()
[[ ${MIDNIGHT_IMAGE_NATIVE_OFFLINE:-0} != 1 ]] || native_offline+=(--offline)
native_jobs=${CARGO_BUILD_JOBS:-$(nproc)}
source "$repository/plugins/image-decoder/native/bootstrap-linux.sh" "$build_root/native-tools" "$native_jobs" "${native_offline[@]}"
source "$repository/plugins/image-decoder/native/build-linux.sh" "$build_root/native-dav1d" "$native_jobs" "${native_offline[@]}"
for name in "${plugins[@]}"; do
    export CARGO_TARGET_DIR="$build_root/$name"
    arguments=(build --manifest-path "$repository/plugins/$name/Cargo.toml" --config "$api_config" --locked --offline)
    [[ $profile == debug ]] || arguments+=(--release)
    if [[ $name == midnight-thumbnail-video ]]; then
        arguments+=(--features playback-worker)
        RUSTFLAGS='-C link-arg=-Wl,--disable-new-dtags,-rpath,$ORIGIN' cargo "${arguments[@]}"
        cp "$CARGO_TARGET_DIR/$profile/midnight-video-playback" "$package/midnight-video-playback"
    else
        env -u PKG_CONFIG_SYSROOT_DIR cargo "${arguments[@]}"
    fi
    cp "$CARGO_TARGET_DIR/$profile/$name" "$package/$name"
    cp "$repository/plugins/$name/thumbnailer.json" "$package/$name.json"
    if [[ -d $notices_root/$name ]]; then
        mkdir -p "$package/notices/$name"
        cp -R "$notices_root/$name/." "$package/notices/$name/"
        if [[ $name == midnight-thumbnail-image ]]; then
            cp -R "$repository/plugins/image-decoder/notices/." "$package/notices/$name/"
        fi
    else
        echo "Missing distribution notices for $name; stage pinned dependency license notices first" >&2
        exit 2
    fi
done
for name in libavcodec.so.63 libavformat.so.63 libavutil.so.61 libswscale.so.10 libswresample.so.7; do
    cp -L "$FFMPEG_DIR/lib/$name" "$package/$name"
done
mkdir -p "$package/notices/ffmpeg-9.0.2"
cp -R "$FFMPEG_DIR/share/midnight-ffmpeg/." "$package/notices/ffmpeg-9.0.2/"
cp "$source_archive" "$package/notices/ffmpeg-9.0.2/ffmpeg-9.0.2.tar.xz"
if [[ -e $destination ]]; then mv -T -- "$destination" "$staging/previous"; fi
if ! mv -T -- "$package" "$destination"; then
    if [[ -e $staging/previous ]] && ! mv -T -- "$staging/previous" "$destination"; then
        clean_staging=0
        printf 'Previous thumbnail package retained at %s; restoration failed\n' "$staging/previous" >&2
    fi
    exit 1
fi
printf 'Built installed thumbnail providers: %s\n' "$destination"
