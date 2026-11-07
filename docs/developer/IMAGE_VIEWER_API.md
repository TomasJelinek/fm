# Restricted image-viewer API

Image viewers are independent executable packages. The public workspace is
`image-viewer-api/`, containing `midnight-image-viewer-api` and
`midnight-image-viewer-sdk` version 0.1.0. Plugins consume immutable released
registry packages; they never depend on or patch application crates. The SDK
uses the shared public `midnight-worker-sdk` for sandbox startup. The separate
host crate is `midnight-image-viewer-host`.

## Installation and discovery

Install a package under `<application directory>/viewers/<package>/` with
`image-viewer.json`, its executable, `LICENSE`, `THIRD-PARTY.md` and notices:

```json
{
  "schema": 1,
  "protocol": 1,
  "id": "midnight.viewer.image",
  "name": "Images",
  "version": "0.1.0",
  "target": "x86_64-pc-windows-msvc",
  "executable": "midnight-viewer-image.exe",
  "extensions": ["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tif", "tiff"]
}
```

Linux packages use their actual `x86_64-unknown-linux-gnu` or
`aarch64-unknown-linux-gnu` target and `midnight-viewer-image` executable. The
manifest is strict: unknown fields, foreign targets, unsupported versions,
relative executable traversal, duplicate extensions and control characters in
the display name are rejected. Extensions are lowercase ASCII letters/digits,
at most 32 bytes each and 64 entries. A package must not contain both this
manifest and native-viewer `plugin.toml`. Duplicate image-provider IDs exclude
all conflicting providers; application composition also checks native IDs.

`discover(application_executable: &Path)` takes the **full executable path**, not
its directory, and returns usable providers. `discover_with_diagnostics` returns
`Discovery { providers, diagnostics }`, preserving usable packages when another
package is invalid. Each `Provider` exposes ID, name, version, extensions,
`supports_extension(&str)` and `package_dir()`. Executable identity stays private.

Discovery reads at most 128 package candidates, a 16 KiB manifest, and 512 entries
per package with depth at most 8. Every package entry and ancestor passes the
existing native owner/write-permission and redirection checks; the package is
rechecked immediately before launch. Special files, redirected paths and
ambiguous package roles fail. No working directory, panel folder, archive, PATH
or source filename selects executable code. These checks establish installation
trust, not publisher authentication or protection against the trusted account.

## Host source and session contract

All host calls, including `close` and destruction, run on backend workers. The
UI submits requests and cancellation; it does not decode, read files or reap
processes. `Source: Send + Sync` supplies `length`, `version`, `validate`, bounded
`read_at(offset, length)` and an optional positive `confirmed_local_ssd` hint.
`CapturedFile` implements this contract directly. Native and virtual sources
retain their own raw identities; the child receives only source length and
opaque reads. Version and length are checked after opening and before/after
rendering. Live validation detects observed changes; it is not a filesystem
snapshot.

```rust
ImageSession::open(&provider, source, &cancel, timeout) -> io::Result<ImageSession>
session.metadata() -> Metadata
session.render(viewport, &cancel, timeout) -> io::Result<Frame>
session.close()
```

The source parameter is `Arc<dyn Source>`, cancellation is `AtomicBool`, and
timeout is `Duration`. Open decodes once and retains original oriented sRGB
RGBA8 pixels in the restricted child. Rendering uses those retained pixels and
cannot request more source data. `Metadata { width, height }` describes original
oriented dimensions. `Frame { width, height, rgba }` contains a complete physical
viewport with straight alpha. Every render error, cancellation or source change
invalidates the session; subsequent requests cannot revive it.

The host checks cancellation/deadlines at 10 ms polling intervals, kills a failed
or cancelled worker, and reaps it on the backend. A source OS call already in
progress can remain blocked until the OS returns; decoder termination does not
claim to interrupt that call or release its worker early. Source reads share
the application's weighted I/O budget. Session admission/generation checks and
discarding obsolete results belong to the viewer engine.

## Wire format and resource limits

Each message starts with `MIV1`, a little-endian u32 payload length and a u8 tag.
The protocol exposes only `Open`, `Metadata`, `Viewport`, `Rgba`, `Read`, `Data`
and `Failure`. Request IDs bind each returned raster to its viewport. Paths,
URLs, UI objects, native handles and command strings are absent. Startup first
performs the worker SDK's sandbox-ready handshake; content is not parsed before
Windows AppContainer or Linux restriction succeeds.

| Resource | Bound |
|---|---|
| Physical viewport | `MAX_EDGE = 4096`, `MAX_PIXELS = 4 * 1024 * 1024` |
| Decoded image | `MAX_DECODED_EDGE = 32768`, `MAX_DECODED_BYTES = 128 * 1024 * 1024` |
| Broker read/reply | `MAX_READ_BYTES = 256 * 1024` |
| Total source reads during Open | 256 MiB and at most 8192 response-loop iterations |
| Embedded ICC profile | 4 MiB |
| Worker process | 512 MiB memory, 60 cumulative CPU seconds |
| Per Open/render response | Caller-supplied wall-clock timeout and cancellation |
| Scale | `MIN_SCALE = 1 / 32768`, `MAX_SCALE = 64` |

`rgba_length` validates output dimensions before allocation. Metadata separately
checks original dimensions and RGBA storage. Message decoding uses type-specific
allocation limits and rejects truncation, trailing fields, invalid tags/version,
nonfinite geometry, read overflow and inconsistent raster lengths. The cumulative
CPU allowance covers the entire persistent session; exhausting it ends the
session rather than silently refreshing its budget.

`Viewport { width, height, scale, origin_x, origin_y }` uses physical output pixels,
source-to-physical scale and source-coordinate top-left. Negative origins permit
centering; outside-image pixels are transparent. Output pixel centers map to
`origin + (output + 0.5) / scale`. Scale 1 with integral origins preserves exact
decoded pixels; zoom never enlarges a thumbnail. The image plugin uses nearest
original pixels for magnification and alpha-weighted area filtering for reduction,
without allocating an entire scaled image. Animated formats expose their first
still frame. image-rs 0.25.10 and moxcms 0.8.1 match thumbnail codec features,
orientation and sRGB color handling.

## Independent build and verification

`scripts/build-image-viewer.ps1` and `.sh` build/package the worker and are called
by `build-viewers`. Their worker SDK config is independent of the native viewer
SDK (`-WorkerSdkConfig` or `MIDNIGHT_IMAGE_VIEWER_SDK_CONFIG`). Without a supplied
config they cache a source-stamped immutable bundle from `worker-packages`.
Plugin-owned `scripts/build.*` and `scripts/ci.*` operate only on the exported
plugin and public registry; all output stays under `_build`. With build root
`_build/image-viewer-ci`, the independent package is
`_build/image-viewer-ci/package/midnight-viewer-image`.

The public API tests cover malformed/truncated messages and independent resource
bounds. SDK tests cover exact broker replies, seek limits and one decode serving
multiple viewports. Plugin tests generate owned PNG/JPEG/GIF/ICC fixtures for
exact pixels, alpha, cropping/zoom, fit filtering, EXIF orientation, color
conversion, malformed content and excessive dimensions. Host native tests require
`MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION` pointing into an owned trusted test
installation; they exercise the actual sandbox, cached renders, changed sources,
corruption, cancellation and deadlines behind the shared I/O gate.
