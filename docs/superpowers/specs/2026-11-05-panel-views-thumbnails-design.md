# Panel views and media thumbnails

Each panel independently chooses Detailed, Brief or Thumbnails in its top action
toolbar. Detailed retains Name/Size/Modified columns; Brief shows compact columns
of icons and names; Thumbnails shows a grid with larger host icons and filenames.
Changing layout preserves the raw path, cursor, multiselection, search and sort.
Only visible cells plus a bounded margin are instantiated. The layout translates
keyboard movement and viewport indices to the same immutable entry model; pointer
events retain generation/identity checks. Panel mode is saved with preferences.

Without an installed thumbnail provider the grid displays host icons, with normal
file opening on Enter/double-click. Installed independent image/video providers
decode bounded previews on backend workers; unsupported or failed previews retain
the host icon. Options → Previews controls enablement, network opt-in and cache clearing.

## Decoder choice and plugin boundary

Use **image-rs (`image`)** for raster images and resizing, with explicitly enabled
formats and limits. It provides native Rust decoders and image transformations;
dimension limits can be strict, while allocation limits are not uniformly strict.
Do not treat decoder limits as a complete sandbox. See the primary
[crate documentation](https://docs.rs/image/latest/image/) and
[limit contract](https://docs.rs/image/latest/image/struct.Limits.html).

The same decoder is selected for a future independent image-viewer plugin:
decode/orient/resize in the backend helper, present bounded pixels through the
host, and keep zoom/pan/window controls in the host UI. `image` is
MIT OR Apache-2.0. Version 0.25.10 backs the independent image thumbnail provider;
the full image-viewer plugin remains future work.
The current public viewer API models text/binary/documents, not raster frames;
image/tile messages require a separately versioned, host-owned API extension.

Use **FFmpeg libavformat/libavcodec/libswscale through `ffmpeg-next`** for video
frame extraction. The optional video provider uses ffmpeg-next 9.0.0 with a minimal
shared FFmpeg 9.0.2 build on Windows and Linux; later macOS requires its own native
adapter and deployment validation. The wrapper exposes format, codec, frame and software
conversion modules ([primary API](https://docs.rs/ffmpeg-next/latest/ffmpeg_next/)).
The wrapper's [license](https://raw.githubusercontent.com/zmwangx/rust-ffmpeg/master/LICENSE)
is WTFPL; it does not replace native FFmpeg's terms. The baseline native build
should use LGPL-2.1-or-later components and avoid optional GPL/nonfree components;
FFmpeg's [license description](https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/LICENSE.md)
explains how build options change distribution terms. Codec selection must also
satisfy the [free-of-charge dependency policy](../../DEPENDENCY_POLICY.md).
Do not promise every proprietary/patent-encumbered format is royalty-free merely
because a decoder is open source. Keep the concrete wrapper/native versions,
enabled formats and corresponding source pinned together.
An ordinary video displays one representative frame. Hover or selection enables
up to five additional samples. Every visible selected video progresses through
the samples in a fair rotation; hover receives additional priority without
starving selected videos. There is no audio playback or full-video decode.

Thumbnail providers are separate executable helpers under the installation's
`thumbnailers/` directory. This is a new process protocol, separate from released
viewer C ABI/SDK 0.1.0; released viewer packages retain their bytes and checksums.
The independent `worker-api/` and `thumbnail-api/` workspaces define startup,
bounded seek/read brokerage and bounded RGBA responses. The host owns discovery,
cache, scheduling and presentation. Providers share only a versioned API and
fixtures/contracts, following the existing viewer/filesystem plugin rules. Their
tests live with their implementations; they cannot modify main application sources.
The host brokers read-only provider data and receives bounded RGBA output, dimensions,
source version and an opaque request ID. Provider APIs must support archive and
remote identities without turning them into guessed local filenames.

Run untrusted media parsing in a restricted helper process with OS-specific
implementations, a memory/CPU budget and killable timeout. A dynamically loaded
plugin alone provides no isolation. No shell command construction, external URL
fetches, playlists, embedded scripts or network protocols are enabled. Give the
helper a brokered read-only stream/handle rather than arbitrary filesystem authority;
seek requests are range checked. Apply EXIF orientation, aspect-preserving resize
and explicit color-space handling. GPU decode is optional only after measurement;
CPU decoding with bounded concurrency is the baseline.

## Scheduling and cache

The backend schedules bounded jobs; blocking decoding stays in helpers/workers. Prioritize
the visible viewport, then a small prefetch band. Cancel on navigation or large
scroll jumps and reject stale results by pane generation, provider identity and
source version. Use a shared memory budget (initially 64 MiB), at most two decoder
jobs, serial source reads on HDD/network/unknown storage, and explicit opt-in for
network thumbnails. Unknown or not-yet-classified storage is also withheld until
the network opt-in is enabled; it must not bypass that setting during directory
loading. Repeated failures
receive a short negative-cache TTL, invalidated when the source changes.

Persist every successfully generated static thumbnail and animation frame;
temporary offscreen state does not discard already generated valid frames. Keep
the disk cache per user, outside executable/plugin directories and `_build`:

| Platform | Private application cache |
| --- | --- |
| Windows | `%LOCALAPPDATA%\Midnight Explorer\cache\thumbnails\v1` |
| Linux | `$XDG_CACHE_HOME/midnight-explorer/thumbnails/v1`, default `~/.cache/midnight-explorer/thumbnails/v1` |
| macOS later | `~/Library/Caches/<application-id>/thumbnails/v1` |

The Linux root follows the [XDG cache definition](https://specifications.freedesktop.org/basedir/latest/).
This is an application-private cache, not an implementation of the shared
[freedesktop thumbnail format](https://specifications.freedesktop.org/thumbnail/latest/directory.html).
Optional interoperability can be added by a separate adapter; never assume another
application's cache validates our provider identities or security limits.

Cache keys hash a versioned encoding of provider ID, raw object identity, source
version/size/mtime, requested physical pixel dimensions, decoder/version and color
policy. Use BLAKE3 to keep filenames bounded; metadata verifies the complete key
so a hash alone is not authority. Source revalidation is mandatory before accepting
a cached preview. Use atomic replacement of owned temp files, private permissions,
no symlink traversal and background LRU eviction (initial 512 MiB disk budget).
Cached content is disposable and errors must not stop browsing. Cache clearing and
disable controls belong in Options. Cache write failures are explicit preview
failures; generation must not silently fall back to memory-only retention.
Remote thumbnails remain disabled unless explicitly opted in.
Do not persist credentials, authenticated URLs or cleartext remote secrets in keys.

## Acceptance before real previews ship

Test format fixtures, malformed/truncated/bomb inputs, huge dimensions, decoder
timeouts/crashes, orientation/color, cache invalidation and atomic write failures.
Test Unicode/raw native identities, archive/remote sources, cancellation and stale
completion after rapid navigation. Measure memory and frame time with large folders
on all supported platforms. Each provider owns its decoder tests; the host tests
only API contracts, scheduling, resource bounds and presentation.

### Resource scheduling details

CPU slots and source I/O admission are separate budgets. A CPU job is a bounded
backend worker supervising one killable helper, never a Slint callback. A queued
request keeps a pane/view generation, opaque source identity and captured source
version. Worker completion releases its actual CPU slot even after cancellation;
navigation cannot falsely free a still-running worker. The visible queue is bounded
to 256 entries including its small margin. Offscreen or deselected videos stop
animation requests immediately. Already completed valid frames remain persisted
until ordinary LRU eviction.

Local SSD hints allow a bounded source batch. HDD, network and unknown storage
allow one source at a time. The source gate must be shared with directory scans and
copy operations; introducing a second independent semaphore is not proof of a
shared device budget. Warm animation presentation reads cached frames on backend
workers and rotates a bounded set per tick. GPU use remains limited to Slint's
normal texture presentation until measurements justify additional decode work.

### Native deployment baseline

The implementation targets FFmpeg 9.0.2 and ffmpeg-next 9.0.0 after checking current
upstream release/security information on 6 November 2026. The minimal LGPL shared
build enables VP8, VP9, Theora, FFV1 and raw video, with Matroska/WebM, AVI and Ogg
demuxers. It disables GPL, nonfree, version3, network, protocols, audio, hardware
decoders and unnecessary libraries. This is an explicit initial format set, not a
claim that every video codec has fee-free distribution terms. Build options and
matching source/notices accompany distributed shared libraries.

Windows workers start suspended in a capability-free AppContainer, inherit only
broker pipes, join a memory/CPU/process-limited Job, then resume detached from any
console. The public SDK verifies containment before reading media. Linux workers
preload trusted runtime code, set resource limits, close unrelated descriptors and
install a deny-by-default seccomp policy before acknowledging startup. Neither
platform falls back to decoding untrusted input in the application process.
