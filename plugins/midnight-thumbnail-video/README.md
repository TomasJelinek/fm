# FFmpeg thumbnail and playback workers

The normal package installs the thumbnail executable and the video playback
executable together. Both use the public worker SDK and a host-owned read-only
capture. F3 opens the Video viewer using the configured filename associations.

The shared runtime enables **VP8, VP9, Theora, FFV1 and raw video** in
Matroska/WebM, AVI, Ogg and MOV/MP4 containers. Playback also supports Opus audio.
Container labels indicate which files can be tried. Playback still requires
an eligible video stream. Contained files with an unavailable audio codec play
with a visible audio notice.

Native target: **FFmpeg 9.0.2**, dynamically linked under LGPL-2.1-or-later.
Rust wrapper target: **ffmpeg-next 9.0.0**, WTFPL, default features disabled;
it enables `codec`, `format`, `software-scaling` and `software-resampling`. Do not substitute a
generic full FFmpeg distribution or enable GPL/nonfree features.

Build the verified source with `scripts/build-media-ffmpeg.sh`. It deliberately
does not download a compiler, execute a downloaded installer or silently fetch
another FFmpeg version. Acquisition must record the official release archive's
SHA-256 and verify its upstream signature before compiling. All compiler caches
belong under `.tools/`; object files, installed build outputs and test fixtures
belong under `_build/`. Windows needs an MSVC-compatible import library and
libclang for ffmpeg-sys bindgen (or a verified MinGW cross build with converted
import libraries). The native artifacts must be tested on Windows and Linux.

Set `FFMPEG_DIR` to the resulting native prefix and `LIBCLANG_PATH` to the pinned
toolchain's libclang directory when building the Rust provider. Package the
required shared libraries adjacent to the helper, with corresponding source,
build options, LGPL license and notices. Users must remain able to replace these
LGPL libraries. Do not package source files merely containing download URLs as a
substitute for the applicable source-distribution obligations.

Static thumbnail requests use one representative frame. Hovered or selected
visible videos request up to five additional samples. The host queues all visible
selected videos fairly, gives hover priority, cancels offscreen work and persists
each successfully generated frame in its ordinary bounded disk cache.

Samples sit at 10% to 85% of the container duration, counted from the container
start time. When no frame at or after a sample target decodes (a header overstating
a partial download, or sparse keyframes exhausting the 4096-packet budget), the
latest decoded frame is used instead. Packets larger than one 256 KiB brokered read
(uncompressed and FFV1 frames, large VP8/VP9 keyframes) are read in several reads.
BT.709, unspecified and SD (BT.470BG, SMPTE 170M) primaries are shown without gamut
mapping. BT.470M/BG transfer tags, which PAL and NTSC-M SD streams carry for the
BT.601 camera curve, are decoded like that curve; wide-gamut and HDR streams are
rejected.

Primary sources checked 6 November 2026:

- [FFmpeg release versions](https://www.ffmpeg.org/download.html)
- [FFmpeg security fixes](https://www.ffmpeg.org/security.html)
- [FFmpeg license terms](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/LICENSE.md)
- [ffmpeg-next 9.0.0 manifest and features](https://github.com/zmwangx/rust-ffmpeg/blob/v9.0.0/Cargo.toml)

The earlier thumbnail profile passed sampled AVI decoding through restricted
workers on Windows MSVC and Linux x86-64. The playback profile requires fresh
native qualification on both targets. Test an eligible VP9 and Opus MP4 through
both installed executables, then check playback, pause, seek, source revocation,
EOF and retirement through F3. Bind the run to the single installed FFmpeg
library set and the package manifest.

[FFmpeg license and configuration guidance](https://ffmpeg.org/legal.html)
