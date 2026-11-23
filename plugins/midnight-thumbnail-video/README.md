# Video thumbnail and playback workers

The normal package installs two pairs of restricted helpers built from this crate.
Both use the public worker SDK and a host-owned read-only capture. F3 opens the Video
viewer using the configured filename associations.

- `midnight-thumbnail-video` (MTH1) and `midnight-video-playback` (MMV2) decode with
  the bundled FFmpeg profile (feature `bundled`, the default).
- `midnight-thumbnail-video-system` and `midnight-video-playback-system` decode with
  the platform's own decoders (feature `system`) and link no FFmpeg: Media Foundation
  on Windows; on Linux the distribution's FFmpeg, then GStreamer, loaded at run time.

The host tries the bundled helper first. When it reports that it has no decoder for
the video (MMV2 failure cause 5, `NoDecoder`, naming the codec as a number from
`media::Codec` and the decoders it tried), the viewer starts the platform helper on
the same pinned capture; after an MTH1 `Failed` from the bundled video provider, the
thumbnail host asks the platform helper, under the bundled provider's identity and
cache. The host words the final message, for example "This video uses HEVC; Windows
has no decoder for it installed (HEVC Video Extensions)"; helpers never send text.

## Bundled FFmpeg profile

The shared runtime enables **VP8, VP9, Theora, FFV1 and raw video** in
Matroska/WebM, AVI, Ogg and MOV/MP4 containers. Playback also supports Opus audio.
Container labels indicate which files can be tried. Playback still requires
an eligible video stream. Contained files with an unavailable audio codec play
with a visible audio notice. Content no bundled demuxer recognises, and video in a
codec outside the profile, ends in `NoDecoder`.

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

Samples sit at 10%, 25%, 40%, 55%, 70% and 85% of the container duration, counted
from the container start time; the panel shows them in that order, each for 0.8 s
([video cycles](../../docs/developer/NATIVE_ICONS.md#video-cycles)). A seek lands on
the key frame at or before the target and decoding continues forward from there, so
key-frame spacing never moves a sample: the bundled profile takes the frame displayed
at the target, the Media Foundation helper the first frame at or after it. When no
frame at or after a sample target decodes (a header overstating
a partial download, or sparse keyframes exhausting the 4096-packet budget), the
latest decoded frame is used instead. Packets larger than one 256 KiB brokered read
(uncompressed and FFV1 frames, large VP8/VP9 keyframes) are read in several reads.
BT.709, unspecified and SD (BT.470BG, SMPTE 170M) primaries are shown without gamut
mapping. BT.470M/BG transfer tags, which PAL and NTSC-M SD streams carry for the
BT.601 camera curve, are decoded like that curve; wide-gamut and HDR streams are
rejected.

## Media Foundation (Windows platform helpers)

The Windows platform helpers decode with the Media Foundation components that
Windows itself includes or that the user installed from the Microsoft Store:
H.264, MPEG-4 Part 2, VC-1 and Windows Media Video, Motion JPEG and H.263 video;
AAC, MP3 and Windows Media Audio; and with the Store extensions HEVC (HEVC Video
Extensions, which may be a paid Store item or come with the PC), AV1 (AV1 Video
Extension), VP9 and MPEG-2. Containers are those
Media Foundation reads: MP4/M4V/MOV/3GP, ASF/WMV, AVI, Matroska and others. Nothing
here encodes, and no codec is bundled; Microsoft licenses these decoders with Windows
or the extension.

- Source: Media Foundation reads only an `IMFByteStream` over the host-brokered
  capture: read-only, seekable, with no URL or path, so no source can reopen the file,
  follow a reference or use the network. Each read is a positioned broker request,
  served by the helper's pipe thread while Media Foundation decodes on its own thread
  (`system/media_foundation/remote.rs`).
- Decoding: software only. The source readers get no Direct3D manager, hardware
  transforms stay off and DXVA is disabled, so no GPU driver code loads into the
  helper. One reader decodes the first video stream, a second one the first audio
  stream, so PCM demand stays independent of the video.
- Pictures: the decoder's NV12, P010, YUY2, I420 or YV12 output is converted here with
  its stated BT.601 or BT.709 matrix and range, each output pixel averaging its whole
  box of source samples, then turned upright from the track's rotation; BT.709 and
  BT.601 transfers are re-encoded as sRGB as in the bundled profile, and HDR or
  BT.2020 video is refused like there.
- Audio: Media Foundation converts to 48 kHz stereo 16-bit PCM; `system::pcm` drops
  the preroll before a seek target and fills gaps and the tail with silence.
- Images: before reading any content the helpers switch on Windows' binary signature
  policy for Microsoft-signed images only (`SetProcessMitigationPolicy`), so a codec
  pack's decoder or byte-stream handler registered system-wide cannot load into
  them. Store codec extensions are signed by Microsoft and still load.
- A codec without a Windows decoder ends in `NoDecoder` with that codec; content that
  Media Foundation does not recognise ends in `NoDecoder` without one.

Build the Windows platform helpers without FFmpeg:

    cargo build --config <worker SDK registry.toml> --no-default-features --features system,playback-worker

Tests generate their H.264 + AAC (and, where a GPU encoder exists, HEVC) MP4 fixtures
at test time with Media Foundation's own encoders (`tests/fixtures/mf_writer.rs`); no
encoded fixture is committed. Run them with the default features plus
`--features system,playback-worker`.

## Distribution FFmpeg and GStreamer (Linux platform helpers)

The Linux platform helpers decode with libraries the user's distribution installed.
Nothing is bundled, and the helpers link neither family: both are loaded at run time
(`src/system/linux/`), before the helper enters the worker sandbox.

- First the distribution's FFmpeg: libavformat, libavcodec, libavutil, libswscale and
  libswresample of one series, libavcodec 58 to 63 (FFmpeg 4 to 9), found by soname,
  newest series first. The first library loads with its dependencies or not at all;
  after that the series is committed, the other four must match it, and every
  library's reported version must have the soname's major and be no older than the
  oldest release of that series. A process never maps two series: loading refuses
  when any FFmpeg is already mapped.
- Struct layouts differ between series. `src/system/linux/ffmpeg/layouts.rs` holds the
  field offsets per series, generated by `scripts/generate-ffmpeg-system-layouts.py`
  from the public headers of the first and last release of each series (4.0 and 4.4.8,
  5.0 and 5.1.10, 6.0 and 6.1.6, 7.0 and 7.1.5, 8.0 and 8.1.3, 9.0 and 9.1); a field is
  used only when every probed release of the series declares it at the same offset.
  The generated file also records each called function's declaration per series.
  Channel layouts (a 64-bit mask before FFmpeg 5.1, `AVChannelLayout` after) and the
  display matrix (stream side data before FFmpeg 7, codec parameters after) use the API
  generation the series has. `scripts/tests/test_ffmpeg_system_layouts.py` checks the
  tables against the headers again.
- FFmpeg reads only through a custom AVIO context over the brokered capture, with an
  empty protocol whitelist, a refusing `io_open`, the container whitelist of the
  registered extensions (MP4/MOV, Matroska/WebM, AVI, Ogg and ASF for `.wmv`/`.asf`),
  one decoder thread and the bundled profile's pixel bound.
- Then GStreamer 1.x, declared by hand against its stable 1.x C ABI. A fixed list of
  plugins is loaded from the folder beside libgstreamer: base (typefind, converters,
  Ogg, Theora, Vorbis, Opus), good (MP4/MOV, Matroska/WebM, AVI, audio parsers, VP8/VP9,
  FLAC, mpg123), bad (video parsers, OpenH264, libde265, dav1d, AOM AV1, faad, fdk-aac,
  DTS), ugly (a52dec, and the ASF demuxer `asf` for `.wmv`/`.asf`), and gst-libav only
  when no distribution FFmpeg was loaded; Windows Media Video (WMV 1/2/3, VC-1) and
  Windows Media Audio have no GStreamer decoder but gst-libav's. The `asf` plugin also
  registers an RTSP extension and an RTP depayloader, which no graph here selects (they
  take no demuxed caps, and no network source is loaded). A test checks that every
  extension in `thumbnailer.json` has its container in both lists. Device,
  hardware-decoder, network, display and capture plugins are never loaded, and neither
  are the user's plugin folders: there is no plugin registry at all.
- The GStreamer graph runs on the helper's one thread with no pipeline, bus, queue,
  decodebin or appsrc (each starts threads). The helper owns a source pad and pushes
  the brokered bytes into the demuxer; demuxing, parsing, decoding and conversion run
  inside each push and the output reaches the helper's sink pad before the push
  returns. Elements are chosen like decodebin chooses them: typefind, the best-ranked
  demuxer, a parser when one exists, the best-ranked decoder, each with one thread.
  Seeking uses the demuxers' push-mode seek: a demuxer turns a time seek into a byte
  seek on the helper's pad, which flushes and restarts the pushes there (this is also
  how an MP4 whose index follows the media is read). Push sizes shrink while frames
  flow, so one push completes only a few frames.
- Pictures are converted by libswscale or by videoconvert and videoscale with the
  stream's matrix and range; the size bound, rotation (display matrix, or
  `image-orientation` tags), the BT.709-to-sRGB transfer and the HDR/BT.2020 refusal
  follow the bundled profile. Audio is mixed down and resampled to 48 kHz stereo 16-bit
  by libswresample or audioconvert and audioresample; `system::pcm` assembles blocks.
- A video codec that neither family decodes ends in `NoDecoder` naming the codec (from
  FFmpeg's demuxer, which names codecs it cannot decode, or GStreamer's caps) and what
  was tried (`Tried` bits 2 and 4); a container no family recognises is invalid data,
  and one without video is unsupported. Audio no family decodes plays as an
  unavailable track, as in the bundled profile.

Build the Linux platform helpers without FFmpeg and without an rpath, so no library
beside them can be found by soname (the packaging script unsets `RUSTFLAGS`):

    cargo build --config <worker SDK registry.toml> --no-default-features --features system,playback-worker

Test them the same way, with `--test-threads=1`. The tests encode their fixtures at
test time with the distribution's `ffmpeg` command (H.264 + AAC MP4, also with the
index after the media, HEVC + AAC Matroska, MPEG-4 Part 2 + MP3 AVI, Windows Media
Video 8 + Windows Media Audio 2 in `.wmv` and version 7 + 1 in `.asf`, a rotated MP4, a
4K H.264 MP4); none is committed. FFmpeg encodes no WMV 9 or VC-1, so those decoders
are not exercised. Each test that loads a family runs in a forked
child, because a process maps one FFmpeg series; some children enter the real worker
sandbox first. `tests/system_linux.rs` runs the built helpers as the host does and
checks their dynamic section. Optional coverage: `MIDNIGHT_TEST_FFMPEG_DIRS` (folders
of private FFmpeg builds of other series, each with the H.264 and AAC decoders),
`MIDNIGHT_TEST_FFMPEG_NO_HEVC` (a private build without the HEVC and MPEG-4 Part 2
decoders, like a distribution build without them) and `MIDNIGHT_FFMPEG_HEADERS`
(extracted release headers, for the layout check). Tests whose library, plugin or
encoder is missing report it and skip that part.

Primary sources checked 6 November 2026:

- [FFmpeg release versions](https://www.ffmpeg.org/download.html)
- [FFmpeg security fixes](https://www.ffmpeg.org/security.html)
- [FFmpeg license terms](https://github.com/FFmpeg/FFmpeg/blob/n9.0.2/LICENSE.md)
- [ffmpeg-next 9.0.0 manifest and features](https://github.com/zmwangx/rust-ffmpeg/blob/v9.0.0/Cargo.toml)

After a change to the bundled profile, check an eligible VP9 and Opus MP4 on Windows
and Linux through both installed bundled helpers: its thumbnail, then playback, pause,
seek, source revocation, end of file and closing through F3.

[FFmpeg license and configuration guidance](https://ffmpeg.org/legal.html)
