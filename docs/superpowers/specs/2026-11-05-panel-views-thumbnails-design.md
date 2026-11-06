# Panel views and media thumbnails

Each panel independently chooses Detailed, Brief or Thumbnails in its top action
toolbar. Detailed retains Name/Size/Modified columns; Brief shows compact columns
of icons and names; Thumbnails shows a grid with larger host icons and filenames.
Changing layout preserves the raw path, cursor, multiselection, search and sort.
Only visible cells plus a bounded margin are instantiated. The layout translates
keyboard movement and viewport indices to the same immutable entry model; pointer
events retain generation/identity checks. Panel mode is saved with preferences.

The initial Thumbnails implementation does not inspect file contents. It displays
host icons, with normal file opening on Enter/double-click. Actual preview decoding
is a later independently tested backend capability; failure always leaves the icon.

## Decoder choice and plugin boundary

Use **image-rs (`image`)** for raster images and resizing, with explicitly enabled
formats and limits. It provides native Rust decoders and image transformations;
dimension limits can be strict, while allocation limits are not uniformly strict.
Do not treat decoder limits as a complete sandbox. See the primary
[crate documentation](https://docs.rs/image/latest/image/) and
[limit contract](https://docs.rs/image/latest/image/struct.Limits.html).

Use **FFmpeg libavformat/libavcodec/libswscale through `ffmpeg-next`** for video
frame extraction. This is a proposed optional dependency, not a newly bundled
runtime. Select and pin a compatible Rust wrapper/native FFmpeg build when the
feature is implemented; validate deployment and codec licensing for Windows,
Linux and later macOS. The wrapper exposes format, codec, frame and software
conversion modules ([primary API](https://docs.rs/ffmpeg-next/latest/ffmpeg_next/)).
One representative frame is sufficient initially; no audio playback or full-video
decoding is required. GStreamer is an alternative if the product later needs full
playback, but would add a larger runtime/plugin deployment for this task.

Thumbnail providers are developed separately and share only a versioned API and
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

Tokio schedules bounded jobs; blocking decoding stays in helpers/workers. Prioritize
the visible viewport, then a small prefetch band. Cancel on navigation or large
scroll jumps and reject stale results by pane generation, provider identity and
source version. Use a shared memory budget (initially64MiB), two local-SSD decoders,
one on HDD/unknown, and explicit opt-in for remote thumbnails. Repeated failures
receive a short negative-cache TTL, invalidated when the source changes.

Keep the disk cache per user, outside executable/plugin directories and `_build`:

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
no symlink traversal and background LRU eviction (initial512MiB disk budget).
Cached content is disposable and errors must not stop browsing. Cache clearing and
disable controls belong in Options; private/remote content can use memory only.
Do not persist credentials, authenticated URLs or cleartext remote secrets in keys.

## Acceptance before real previews ship

Test format fixtures, malformed/truncated/bomb inputs, huge dimensions, decoder
timeouts/crashes, orientation/color, cache invalidation and atomic write failures.
Test Unicode/raw native identities, archive/remote sources, cancellation and stale
completion after rapid navigation. Measure memory and frame time with large folders
on all supported platforms. Each provider owns its decoder tests; the host tests
only API contracts, scheduling, resource bounds and presentation.
