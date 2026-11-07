# Video thumbnail provider

Independent executable provider. It consumes only the versioned public thumbnail
SDK and the host's read-only broker stream; it never receives a media pathname.

The initial supported codec set is **VP8, VP9, Theora, FFV1 and raw video** in
Matroska/WebM, AVI and Ogg containers. This is intentionally an explicit deployment
allowlist. H.264/H.265, MPEG-4, AV1, audio, network URLs and playlists are not enabled
by this baseline. A different codec set requires its own licensing/security review.

Native target: **FFmpeg 9.0.2**, dynamically linked under LGPL-2.1-or-later.
Rust wrapper target: **ffmpeg-next 9.0.0**, WTFPL, default features disabled;
only `codec`, `format` and `software-scaling` are enabled. Do not substitute a
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

Primary sources checked 6 November 2026:

- [FFmpeg release versions](https://www.ffmpeg.org/download.html)
- [FFmpeg security fixes](https://www.ffmpeg.org/security.html)
- [FFmpeg license terms](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/LICENSE.md)
- [ffmpeg-next 9.0.0 manifest and features](https://github.com/zmwangx/rust-ffmpeg/blob/v9.0.0/Cargo.toml)

FFmpeg 9.0.2 was built natively for Windows MSVC and Linux x86-64 from the verified
source. Both targets pass real sampled AVI decoding through their restricted
helper process, plus malformed/playlist rejection tests. Windows private
installation additionally passes provider discovery and decoding with only the
four explicit runtime-file ReadAndExecute grants. See the repository's
`docs/verification/2026-11-06-thumbnail-progress.md` for exact qualification and
the remaining main-application release checks.
