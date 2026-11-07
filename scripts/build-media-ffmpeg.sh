#!/usr/bin/env bash
# Build only the approved free baseline. Acquisition/signature verification is a separate step.
set -euo pipefail
if (( $# < 4 )); then
    echo 'usage: build-media-ffmpeg.sh VERIFIED_SOURCE BUILD_DIRECTORY PREFIX linux|mingw64|msvc' >&2
    exit 2
fi
source_root=$(realpath "$1")
build_root=$(realpath -m "$2")
install_root=$(realpath -m "$3")
platform=$4
[[ $(cat "$source_root/RELEASE") == '9.0.2' ]] || { echo 'Expected verified FFmpeg 9.0.2 source' >&2; exit 2; }
[[ -f "$source_root/COPYING.LGPLv2.1" ]] || { echo 'Missing LGPL source license' >&2; exit 2; }
[[ $build_root == */_build/* && $install_root == */_build/* ]] || {
    echo 'Build and install output must be under an explicit _build directory' >&2; exit 2;
}
mkdir -p "$build_root" "$install_root"
cd "$build_root"
arguments=(
    "--prefix=$install_root"
    --disable-everything --disable-autodetect --disable-programs --disable-doc
    --disable-debug --enable-shared --disable-static --enable-pic
    --disable-gpl --disable-nonfree --disable-version3
    --disable-network --disable-protocols --disable-avdevice --disable-avfilter
    --disable-swresample --disable-hwaccels --disable-pthreads --disable-w32threads
    --disable-x86asm --enable-avcodec --enable-avformat --enable-avutil --enable-swscale
    --enable-decoder=vp8,vp9,theora,ffv1,rawvideo
    --enable-demuxer=matroska,avi,ogg --enable-parser=vp8,vp9
)
case "$platform" in
    linux) ;;
    mingw64) arguments+=(--target-os=mingw32 --arch=x86_64 --enable-cross-compile --cross-prefix=x86_64-w64-mingw32-) ;;
    msvc) arguments+=(--toolchain=msvc --target-os=win64 --arch=x86_64) ;;
    *) echo 'Unsupported FFmpeg target' >&2; exit 2 ;;
esac
"$source_root/configure" "${arguments[@]}"
for feature in GPL NONFREE VERSION3 NETWORK; do
    grep -q "^#define CONFIG_${feature} 0$" config.h || { echo "Unexpected enabled feature $feature" >&2; exit 1; }
done
make -j "${MIDNIGHT_NATIVE_JOBS:-2}"
make install
if [[ $platform == msvc ]]; then
    # FFmpeg installs MSVC import libraries beside DLLs; Rust's FFMPEG_DIR uses lib/.
    cp "$install_root"/bin/*.lib "$install_root/lib/"
fi
mkdir -p "$install_root/share/midnight-ffmpeg"
cp "$source_root/COPYING.LGPLv2.1" "$source_root/LICENSE.md" "$install_root/share/midnight-ffmpeg/"
cp config.h config_components.h ffbuild/config.mak "$install_root/share/midnight-ffmpeg/"
printf '%s\n' 'FFmpeg 9.0.2' "${arguments[@]}" > "$install_root/share/midnight-ffmpeg/build-options.txt"
printf '%s\n' "Built restricted FFmpeg runtime: $install_root"
