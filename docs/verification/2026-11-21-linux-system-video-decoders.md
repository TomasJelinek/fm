# Linux system video decoders — 2026-11-21

## Scope

The Linux platform helpers `midnight-thumbnail-video-system` and
`midnight-video-playback-system` (plugin `midnight-thumbnail-video`, feature `system`,
code in `src/system/linux/`): video in codecs outside the bundled FFmpeg profile is
decoded by the distribution's own FFmpeg (libavcodec 58 to 63), else by GStreamer 1.x,
both loaded at run time before the worker sandbox, plus the host's Linux wording
(`crates/midnight-media-host/src/system_decoders_linux.rs`). Design, version support
and sandbox rules are in the [video worker guide](../../plugins/midnight-thumbnail-video/README.md),
[SECURITY.md](../../SECURITY.md) and the [dependency policy](../dependency-policy.md).

Not covered here: the host routing (bundled helper, then platform helper) and the
packaging, which the viewer agent owns; an installed-package run through the viewer.

## Environment

WSL 2, Ubuntu 24.04.4 LTS, kernel 6.6.87.2-microsoft-standard-WSL2, x86-64; Rust 1.99.1
from `~/fm-tools`. Clone `~/fm-wt/linmedia` at `1c548c6a` with the shared worktree's
uncommitted `plugins/midnight-thumbnail-video/`, `crates/midnight-media-host/` and
`crates/midnight-worker-platform/` copied in; worker SDK registry bundle built from the
clone with `worker-packages`. Distribution packages (installed by the user for this
run): `ffmpeg` 7:6.1.1-3ubuntu5 (libavcodec60 60.31.102, libavformat60 60.16.100,
libavutil58 58.29.100, libswscale7 7.5.100, libswresample4 4.12.100, GPL build with
libx264/libx265 encoders), GStreamer 1.24.2 with `gstreamer1.0-plugins-base`,
`-good`, `-bad` (OpenH264 2.4.1, libde265), `gstreamer1.0-libav` 1.24.1 and
`gstreamer1.0-tools`. Private FFmpeg builds for the version-dispatch tests (never
installed, inside the clone's `_build`): 4.4.8 (libavcodec 58.134.100) and 8.1.3
(libavcodec 62.28.103), from signature-verified release archives (key
`FCF986EA15E6E293A5644F10B4322F04D67658D8`), shared, no assembly, decoders h264, aac,
mp3, opus, vp8 and vp9 only (no HEVC or MPEG-4 Part 2), demuxers mov, matroska, avi
and ogg. Fixtures are encoded at test time by Ubuntu's `ffmpeg` command; none is
committed.

Release archives whose public headers generated and checked the layout tables, all
verified against their upstream signatures:

| Archive | SHA-256 |
| --- | --- |
| ffmpeg-4.0.tar.xz | `ed945daf40b124e77a685893cc025d086f638bc703183460aff49508edb3a43f` |
| ffmpeg-4.4.8.tar.xz | `c73848c4ae283d9eaee7be3b276affbc3543380483555500d0dd2c9b7e1c39c3` |
| ffmpeg-5.0.tar.xz | `51e919f7d205062c0fd4fae6243a84850391115104ccf1efc451733bc0ac7298` |
| ffmpeg-5.1.10.tar.xz | `392306d6fc45dab0e9e0ea55381e071842e83a2fb31d320aeda40477a7766293` |
| ffmpeg-6.0.tar.xz | `57be87c22d9b49c112b6d24bc67d42508660e6b718b3db89c44e47e289137082` |
| ffmpeg-6.1.6.tar.xz | `d4fcb164028dd3beee5d92c0ac72e46aac6973c75ea12dc14de07bf8f407370a` |
| ffmpeg-7.0.tar.xz | `4426a94dd2c814945456600c8adfc402bee65ec14a70e8c531ec9a2cd651da7b` |
| ffmpeg-7.1.5.tar.xz | `de668509caf9e35e3cd162473441fdb29538c6d96ed080292b3cf9e6fc5d558f` |
| ffmpeg-8.0.tar.xz | `b2751fccb6cc4c77708113cd78b561059b6fa904b24162fa0be2d60273d27b8e` |
| ffmpeg-8.1.3.tar.xz | `7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3` |
| ffmpeg-9.0.tar.xz | `7f607a00dd0d28a729d5a4811205812eef01cf6ef6155025febb6f36a9062d52` |
| ffmpeg-9.1.tar.xz | `1a4d394ae63d190a526cd052eb4415c156b5c5e2df37efab41a7de0b8fde2433` |

## Commands

Environment for every Cargo command: `RUSTUP_HOME=~/fm-tools/rustup
CARGO_HOME=~/fm-tools/cargo CARGO_REGISTRIES_MIDNIGHT_INDEX=https://midnight-explorer.invalid/api-index
CARGO_NET_OFFLINE=true CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
CARGO_INCREMENTAL=0 TMPDIR=<clone>/_build/tmp`, `<sdk>` the bundle's
`.cargo/registry.toml`.

    cargo --config <sdk> test --manifest-path plugins/midnight-thumbnail-video/Cargo.toml \
      --no-default-features --features system,playback-worker --locked --offline \
      --lib -- system::linux --test-threads=1
    # with MIDNIGHT_TEST_FFMPEG_DIRS=<prefix-4.4.8>/lib:<prefix-8.1.3>/lib
    #      MIDNIGHT_TEST_FFMPEG_NO_HEVC=<prefix-8.1.3>/lib
    cargo --config <sdk> test ... --test system_linux -- --test-threads=1
    cargo clippy --config <sdk> ... --all-targets; rustfmt --edition 2024 --check <files>
    python3 scripts/generate-ffmpeg-system-layouts.py <headers> --check \
      plugins/midnight-thumbnail-video/src/system/linux/ffmpeg/layouts.rs
    MIDNIGHT_FFMPEG_HEADERS=<headers> python3 -B -m unittest discover -s scripts/tests \
      -p test_ffmpeg_system_layouts.py
    cargo test -p midnight-media-host --locked --offline --lib -- system_decoders

## Results

| Run | Result |
| --- | --- |
| Plugin unit tests `system::linux` | 26 passed, 0 failed (7.7 s) |
| End-to-end helper tests `tests/system_linux.rs` | 7 passed, 0 failed (5.3 s) |
| Layout tables against the headers of FFmpeg 4.0, 4.4.8, 5.0, 5.1.10, 6.0, 6.1.6, 7.0, 7.1.5, 8.0, 8.1.3, 9.0 and 9.1 | identical (`--check`: "matches 12 FFmpeg releases"); script test 2 passed |
| Host wording `system_decoders` (with the shared tests) | 7 passed |
| Clippy (`--all-targets`, system features) and rustfmt on the new files | no findings |
| The viewer agent's `test_default_thumbnailers.py` / `test_installed_video_package.py` on Linux | 20 run, 6 skipped, OK / 1 OK |

What the passing tests show:

- The distribution's FFmpeg 6.1 is found by soname, its five reported versions match
  the series, exactly one series is mapped, and a second load is refused. It decodes
  H.264 + AAC (MP4), HEVC + AAC (Matroska) and MPEG-4 Part 2 + MP3 (AVI): thumbnails at
  the six bundled-profile sample points have the expected colours, playback returns
  30 frames at exact 40 ms steps, one second of non-silent 48 kHz stereo PCM, and after
  a seek to 2.5 s the first frame is the 2.52 s frame and PCM restarts at frame 0.
- The private 4.4.8 (series 58) and 8.1.3 (series 62) builds load as their own series
  through their generated layouts and pass the same H.264 + AAC checks: three FFmpeg
  majors decoded with real libraries.
- With the FFmpeg libraries hidden from the loader, GStreamer (gst-libav then loaded)
  passes the same thumbnail and playback checks, reads an MP4 whose index follows the
  media (push-mode byte seeks served by the helper) and orients a rotated MP4 like
  FFmpeg does.
- With the 8.1.3 build standing in for a distribution FFmpeg without HEVC and MPEG-4
  Part 2: gst-libav is not loaded (one FFmpeg per process), HEVC is decoded by
  GStreamer's libde265, and MPEG-4 Part 2 ends in `NoDecoder` with codec 3 and tried
  bits FFmpeg and GStreamer. An unknown codec tag ends in `NoDecoder` (codec 0, both
  bits); with neither family it is `NoDecoder` (codec 0, no bits); text is invalid
  data; a failed source read is reported as itself through both families.
- Sandbox: both families decode inside the real worker SDK lockdown (a forked child
  calls `midnight_worker_sdk::enter`; a file open then fails). The 177 objects mapped
  after loading both families pass the read rules. GStreamer loads only listed plugins
  (23 here: souphttpsrc, vah264dec, v4l2src and playbin stay unavailable although
  installed), sets `GST_REGISTRY_DISABLE` and writes nothing below `HOME`. Its event and
  query type values and the GstEvent, GstQuery, GstBuffer, GstMapInfo and GParamSpec
  offsets match the installed 1.24.2.
- End to end, the built helpers spawned like the host (cleared environment, MWS1/MWS2,
  512 MiB address space) return an H.264 thumbnail, serve Opened (320 x 240, 4.0 s,
  qualified audio), frames, PCM, a seek and Close over MMV2, report an unknown codec as
  Failed cause 5 (codec 0, tried 0x06), and decode a 4K H.264 thumbnail. Neither helper
  has a RUNPATH/RPATH or a NEEDED entry for FFmpeg, GStreamer or GLib.

Measured, helpers spawned like the host with distribution FFmpeg and GStreamer present:
ready (libraries loaded and sandbox entered) after 51 to 146 ms over the runs; address
space after loading 285 MB (VmSize), peak 342 MB during loading, resident 35 MB; a
320 x 240 thumbnail in 73 to 109 ms and a 4K one in 290 to 718 ms including startup.
The viewer agent gives the platform helpers a 1 GiB address-space policy for headroom
above the libraries' 285 MB.

## Red runs

Each part was reverted in the clone only (never in the shared worktree) and its guard
test run; every one failed, and passed again after restoring:

| Reverted part | Failing test |
| --- | --- |
| Per-series layouts (series 60's layout and no version check for every series) | `each_private_build_loads_as_its_own_series_and_decodes` |
| Oldest-release version check | `versions_must_match_the_soname_series_and_its_oldest_release` |
| GStreamer attempt when FFmpeg is absent | `without_ffmpeg_gstreamer_decodes_h264_and_aac_thumbnails_and_playback` |
| Fall-through after a missing FFmpeg decoder | `a_codec_missing_from_the_distribution_ffmpeg_falls_through_to_gstreamer` |
| Read rule: folders writable by others | `system_libraries_are_trusted_and_writable_or_foreign_paths_are_not` |
| GStreamer environment isolation (registry) | `gstreamer_loads_only_listed_plugins_and_never_touches_the_registry` |
| Plugin allowlist (one network plugin added) | same test |
| GLib thread-pool priming before lockdown | `both_families_decode_inside_the_worker_sandbox` (child ended by SIGTRAP) |
| gst-libav skipped beside a distribution FFmpeg | `a_codec_missing_from_the_distribution_ffmpeg_falls_through_to_gstreamer` |
| Push-mode byte seek on the source pad | `gstreamer_seeks_an_mp4_whose_index_follows_the_media` |
| Keeping a failed source read | `a_source_read_failure_is_reported_as_itself` |
| `NoDecoder` on the wire | `the_playback_helper_reports_a_missing_decoder_as_cause_five` |
| Helpers built with an `$ORIGIN` RUNPATH | `the_system_helpers_link_no_ffmpeg_or_gstreamer_and_have_no_library_path` |
| Host family detection from `ID_LIKE` | `os_release_names_the_family` |
| One generated offset (series 61 `AVFrame.pts`) | `test_the_tables_match_every_release_header` |

## Not tested, and why

- FFmpeg series 59, 61 and 63 with real libraries: only their headers were checked
  (layout tables), not built, to save the shared disk; 58, 60 and 62 ran for real.
- Distributions other than Ubuntu 24.04 (Fedora's `ffmpeg-free`, openSUSE, Arch,
  Debian), GStreamer releases before 1.24, and AArch64: not available here. The host
  wording for them has unit tests only.
- A desktop session (WSL only), installed packages through the viewer, A/V sync
  through the audio output and long or high-bitrate files: these belong to the
  `viewers_video` acceptance run on installed packages.
- Codecs beyond H.264, HEVC, MPEG-4 Part 2, AAC and MP3 (AV1, VC-1, AC-3, ...), AVI
  seeking through GStreamer, and Ogg/Matroska seeking through GStreamer.
