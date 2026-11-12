#!/usr/bin/env bash
set -euo pipefail
if (( $# < 4 )); then
    echo 'usage: build-thumbnailers.sh API_CONFIG DESTINATION BUILD_ROOT NOTICES_ROOT [FFMPEG_PREFIX FFMPEG_SOURCE_ARCHIVE LIBCLANG_PATH] [debug|release]' >&2
    exit 2
fi
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
api_config=$(realpath "$1")
destination=$(realpath -ms "$2")
build_root=$(realpath -ms "$3")
notices_root=$(realpath "$4")
[[ $destination == */_build/* && $build_root == */_build/* ]] || { echo 'Generated output must be under _build' >&2; exit 2; }
for target in "$destination" "$build_root"; do
    parent=$target
    while [[ $parent != / ]]; do
        [[ ! -L $parent ]] || { echo 'Generated deployment path must not traverse symlinks' >&2; exit 2; }
        parent=$(dirname "$parent")
    done
done
mkdir -p "$destination" "$build_root"
profile=${8:-release}
[[ $profile == debug || $profile == release ]] || exit 2
plugins=(midnight-thumbnail-image)
if (( $# >= 7 )) && [[ -n $5 ]]; then
    export FFMPEG_DIR=$(realpath "$5") LIBCLANG_PATH=$(realpath "$7")
    source_archive=$(realpath "$6")
    echo "8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e  $source_archive" | sha256sum --check --status
    plugins+=(midnight-thumbnail-video)
fi
native_offline=()
[[ ${MIDNIGHT_IMAGE_NATIVE_OFFLINE:-0} != 1 ]] || native_offline+=(--offline)
native_jobs=${CARGO_BUILD_JOBS:-2}
source "$repository/plugins/image-decoder/native/bootstrap-linux.sh" "$build_root/native-tools" "$native_jobs" "${native_offline[@]}"
source "$repository/plugins/image-decoder/native/build-linux.sh" "$build_root/native-dav1d" "$native_jobs" "${native_offline[@]}"
for name in "${plugins[@]}"; do
    export CARGO_TARGET_DIR="$build_root/$name"
    arguments=(build --manifest-path "$repository/plugins/$name/Cargo.toml" --config "$api_config" --locked --offline)
    [[ $profile == debug ]] || arguments+=(--release)
    if [[ $name == midnight-thumbnail-video ]]; then
        RUSTFLAGS='-C link-arg=-Wl,--disable-new-dtags,-rpath,$ORIGIN' cargo "${arguments[@]}"
    else
        env -u PKG_CONFIG_SYSROOT_DIR cargo "${arguments[@]}"
    fi
    cp "$CARGO_TARGET_DIR/$profile/$name" "$destination/$name"
    cp "$repository/plugins/$name/thumbnailer.json" "$destination/$name.json"
    if [[ -d $notices_root/$name ]]; then
        mkdir -p "$destination/notices/$name"
        cp -R "$notices_root/$name/." "$destination/notices/$name/"
        if [[ $name == midnight-thumbnail-image ]]; then
            cp -R "$repository/plugins/image-decoder/notices/." "$destination/notices/$name/"
        fi
    else
        echo "Missing distribution notices for $name; stage pinned dependency license notices first" >&2
        exit 2
    fi
done
if (( $# >= 7 )) && [[ -n $5 ]]; then
    for name in libavcodec.so.63 libavformat.so.63 libavutil.so.61 libswscale.so.10; do
        cp -L "$FFMPEG_DIR/lib/$name" "$destination/$name"
    done
    mkdir -p "$destination/notices/ffmpeg-9.0.2"
    cp -R "$FFMPEG_DIR/share/midnight-ffmpeg/." "$destination/notices/ffmpeg-9.0.2/"
    cp "$source_archive" "$destination/notices/ffmpeg-9.0.2/ffmpeg-9.0.2.tar.xz"
fi
printf 'Built installed thumbnail providers: %s\n' "$destination"
