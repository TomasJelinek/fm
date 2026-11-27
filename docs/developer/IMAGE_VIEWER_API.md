# Restricted image and document viewer API

Image and paginated document viewers are independent executable packages. The public workspace is
`image-viewer-api/`, containing `midnight-image-viewer-api` version 0.4.0 and
`midnight-image-viewer-sdk` version 0.5.0. Plugins consume immutable released
registry packages; they never depend on or patch application crates. The SDK
uses the shared public `midnight-worker-sdk` (0.2.0) for sandbox startup. The
separate host crate is `midnight-image-viewer-host`.

API version 0.2.0 preserves every existing MIV1 image message and adds paginated
document messages. API 0.3.0 adds the failure reason (tag 10, see
[Failure reasons](#failure-reasons)); every earlier message is unchanged. API 0.4.0
raises `MAX_DECODED_BYTES` from 128 MiB to 256 MiB and changes nothing else (see
[Decoded-size compatibility](#decoded-size-compatibility)). SDK 0.3.0 built on
`midnight-worker-sdk` 0.2.0; SDK 0.4.0 also sends the reason; SDK 0.5.0 only pins API
0.4.0, so its worker accepts the larger images. Existing 0.1.0 release archives remain
immutable; the image viewer (0.4.0) and PDF viewer (0.3.0) plugins explicitly pin SDK
0.5.0 and its lockfile checksums.
Both public member manifests declare their version explicitly. They must not
inherit `version.workspace`: Cargo also adopts the API as a host path dependency,
where the root application workspace has an independent version. Release archive
checksums cover `Cargo.toml.orig` as well as the normalized manifest. A correction
before publication requires a distinct candidate bundle and verified checksum
refresh; an already published package requires a new version and must never be
retargeted to different bytes.

## Installation and discovery

Install a package under `<application directory>/viewers/<package>/` with
`image-viewer.json`, its executable, `LICENSE`, `THIRD-PARTY.md` and notices:

```json
{
  "schema": 1,
  "protocol": 1,
  "id": "midnight.viewer.image",
  "name": "Image viewer",
  "version": "0.3.0",
  "target": "x86_64-pc-windows-msvc",
  "executable": "midnight-viewer-image.exe",
  "extensions": ["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tif", "tiff", "svg", "avif"]
}
```

Linux packages use their actual `x86_64-unknown-linux-gnu` or
`aarch64-unknown-linux-gnu` target and `midnight-viewer-image` executable. The
manifest is strict: unknown fields, foreign targets, unsupported versions,
relative executable traversal, duplicate extensions and control characters in
the display name are rejected. Extensions are lowercase ASCII letters/digits,
at most 32 bytes each and 64 entries. A package must not contain both this
manifest and native-viewer `plugin.toml` or web-viewer `web-viewer.json`.
Duplicate image-provider IDs exclude
all conflicting providers; application composition also checks native IDs.

`discover(application_executable: &Path)` takes the **full executable path**, not
its directory, and returns usable providers. `discover_with_diagnostics` returns
`Discovery { providers, diagnostics }`, preserving usable packages when another
package is invalid. Each `Provider` exposes ID, name, version, extensions,
`supports_extension(&str)` and `package_dir()`. Executable identity stays private.
`Provider::is_document() -> bool` identifies a protocol-2 paginated provider.
Single-image consumers, including the image comparator, must exclude those
providers unless they explicitly support selecting a document page.

The PDF package uses the same manifest schema, with `protocol: 2`, ID
`midnight.viewer.pdf`, extension `pdf`, and executable `midnight-viewer-pdf.exe`
(or `midnight-viewer-pdf` on Linux). Image packages retain protocol 1. Discovery
requires document metadata from protocol 2 and single-image metadata from
protocol 1, so a provider cannot silently change its declared role.

Discovery reads at most 128 package candidates, a 16 KiB manifest, and 512 entries
per package with depth at most 8. Every package entry and ancestor passes the
existing native owner/write-permission and redirection checks, in the shared package
walk of the plugin host: the package folder with every folder above it once, then each
object below it on its own (its folder was accepted earlier in the same walk). The
package is rechecked immediately before launch. Special files, redirected paths and
ambiguous package roles fail. No working directory, panel folder, archive, PATH
or source filename selects executable code. These checks establish installation
trust, not publisher authentication or protection against the trusted account.

## Shared image formats

The image viewer and image thumbnail workers share the independent
`plugins/image-decoder` library. PNG, JPEG, GIF, WebP, BMP, ICO and TIFF retain
image-rs/moxcms decoding and color management: embedded RGB profiles, and gray
profiles of gray images, are converted to sRGB. image-rs converts CMYK samples to
RGB without a profile, so an embedded CMYK profile is not applied. An image whose
embedded profile has another color model, or a gray profile on a color image,
fails to decode with an `Unsupported` error instead of being shown as sRGB.
CICP-only color signalling (AVIF nclx, PNG cICP) is not converted; those pixels are
used as sRGB. Raster thumbnails reduce the frame first, averaging colors with alpha
weights so pixels next to transparency are not darkened or tinted, and orient the
small result afterwards; a frame that already fits the requested bound is not
resampled. Static SVG uses pinned resvg 0.48.1 inside the same restricted worker
boundary. SVG thumbnails rasterize directly at the requested size; the viewer
retains a bounded original-size raster, so magnifying SVG uses image sampling
rather than rerendering vectors.

SVG draws what resvg draws: paths and shapes, text from embedded Noto Sans fonts,
gradients, patterns, markers, clip paths, masks, filters, local `use` and raster
images embedded as `data:` URIs (PNG, JPEG, GIF, WebP, decoded by the raster path
with its limits). Scripts and event attributes are never run, `foreignObject` is
skipped (a `switch` draws its SVG fallback), and links to files or URLs (images,
`use`, paint and effect references) are never opened. The decoder names skipped
content in a note (`decode_with_note`); API 0.4.0 cannot carry it to the status line
yet (VIEW-06 in the [backlog](../IMPLEMENTATION_BACKLOG.md)). Entity declarations (a
DOCTYPE internal subset), reference cycles and documents over a bound are refused
with the exact reason and bound: 4 MiB of source, 262,144 XML nodes, depth 64, an
estimated 256 MiB of converted tree after references are expanded (1 KiB per element
and 4 bytes per byte of attributes and text), 2^27 style-rule checks, and, at the
drawing size, 448 MiB of memory while drawing and 2^33 weighted pixel operations
(about half a minute); the ordinary image pixel limits remain. The bundled Latin,
Arabic and Hebrew fonts provide deterministic fallback, not exact system-font matching
or complete Unicode coverage. See the
[decoder contract](../../plugins/image-decoder/README.md) and its font notices.

AVIF uses the same independent decoder through the narrowly patched image-rs
constructor, with pinned static dav1d 1.5.4. The patch also ends construction with
an error when an AV1 item yields no displayable frame, instead of waiting for one
forever. The decoder stages at most 32 MiB of encoded container data and rejects
excess input before the tolerant parser. Both primary and alpha receive one-thread
native settings and a 16,777,216-pixel frame limit before allocation; common
output/edge limits still apply. This cap is specific to
AVIF and does not change host source capture or existing streamed raster formats.
The exact MPL-2.0 mp4parse source accompanies both packages. No public API or host
protocol change is needed. HEIC/HEIF remains unsupported pending a qualified
packaged decoder and distribution terms; suffix-only registration is prohibited.

## Host source and session contract

All host calls, including `close_checked` and destruction, run on backend workers.
The
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
session.is_document() -> bool
session.page_count() -> usize
session.page_metadata(page: usize) -> Option<Metadata>
session.render(viewport, &cancel, timeout) -> io::Result<Frame>
session.render_page(page: usize, viewport, &cancel, timeout) -> io::Result<Frame>
session.validate(&cancel) -> io::Result<()>
session.close_checked() -> io::Result<()>
```

The source parameter is `Arc<dyn Source>`, cancellation is `AtomicBool`, and
timeout is `Duration`. Image Open decodes once and retains original oriented sRGB
RGBA8 pixels; document Open retains the parsed source in the restricted child.
Rendering uses that retained content and
cannot request more source data. `Metadata { width, height }` describes original
oriented dimensions. `Frame { width, height, rgba }` contains a complete physical
viewport with straight alpha. Every render error, cancellation or source change
invalidates the session; subsequent requests cannot revive it.

For the first raster-image frame, the host requests adjacent 256-pixel tiles in
horizontal strips through the positioned viewport operation. Each strip covers at
most 16 tiles, clips to the image edges, and fills the ordinary tile cache. Later
pan and zoom requests fetch individual missing tiles. Retained host tiles still
require source revalidation before a new camera frame is published. `validate` performs
that check on the backend without another decoder render. Tiles lying wholly
outside the source image are transparent letterboxing: the engine creates them
locally, with the same bounded pixel ownership and unique tile identities, without
asking the decoder to render or validate each empty tile. Their immutable
app-owned pixel object records guaranteed transparency at construction time.
The presenter keeps their bounded grid/identity metadata for camera coverage,
but creates neither an image row nor a GPU/software texture for empty pixels.
It never scans source pixels on the UI thread to infer transparency. This is an
internal presentation contract, not a plugin wire/ABI change. Tiles intersecting any
source pixel still use the restricted worker. A scene made only from cached or
transparent tiles still requires source validation for its current revision before
publication; optimizing blank space never bypasses that gate.

Raster-image tile sampling is capped at source resolution. Once original pixels
are cached, magnification changes their display coordinates instead of asking the
worker for enlarged copies. Tile keys use the sampling scale; camera surface
identity separately includes the requested zoom, page and viewport so old pan
requests cannot alter a new view. The raster viewport shrinks by
`sampling_scale / display_zoom`, preserving the existing viewport pixel budget.
New camera revisions still validate the source before publishing reused pixels.
Downsampling keeps its filtered worker path, and PDF/document rasterization is
uncapped by this image-only rule. Source-resolution image tiles request nearest
sampling for magnification; reduced-density tiles and documents keep smooth
presentation.
The UI requests its initial viewport from the actual native canvas and display
scale. Each accepted image frame retains explicit physical viewport dimensions;
its pixels are converted to logical display coordinates without stretching them
to a different-sized canvas. Native DPI changes and image toolbar/status layout
changes therefore do not temporarily change the image's declared zoom while a
replacement frame is prepared. Rejected frames cannot update these dimensions.

The host also rejects
a reply whose declared payload exceeds the requested raster before allocating its
body. These are host changes; the independent wire/SDK version is unchanged.

Document providers are identified by actual document metadata, not a filename
suffix. The host renders a document as retained 256-pixel tiles at the exact shown
zoom, in bands: each `PageViewport` request covers one page's part of a rectangle of
whole tiles (at most 4096 pixels across and down, 4 megapixels in all), so a page is
interpreted once per band, not once per tile. Scrolling moves retained tiles; it requests nothing
for tiles that exist. This is a host change only: the wire format, the SDK and the
public viewport bounds are unchanged, and both paths retain cancellation and
captured-source validation. See [viewer scrolling](VIEWER_SCROLLING.md#documents-pdf).

Page indices are zero-based. `metadata()` and existing `render()` address page
zero; an ordinary image has one page and `is_document() == false`. Out-of-range
page metadata returns `None`; an out-of-range render invalidates the session.
Documents return ordered `DocumentMetadata { pages: Vec<Metadata> }`. PDF sizes
use logical points rounded upward; the provider preserves exact page crop and
rotation when rendering. Selection/copy/search of PDF text is not yet exposed.

The host checks cancellation/deadlines at 10 ms polling intervals, kills a failed
or cancelled worker, and reaps it on the backend. A source OS call already in
progress can remain blocked until the OS returns; decoder termination does not
claim to interrupt that call or release its worker early. Source reads share
the application's weighted I/O budget. Session admission/generation checks and
discarding obsolete results belong to the viewer engine.

### Worker preparation

The host exposes `PreparedImageWorker::prepare_owned` and
`ImageSession::open_prepared`. Preparation pins and validates installed code,
starts the restricted worker and waits for the existing Ready handshake. It owns
no selected source or content authority, sends no Open and performs no source
validation, reads or decoding. The prepared transport is consumed once:

```rust
let prepared = PreparedImageWorker::prepare_owned(&provider, &cancel, timeout, Box::new(reservation))?;
let session = ImageSession::open_prepared(&provider, prepared, source, &cancel, timeout)?;
```

The viewer engine shares at most one source-free spare across its service's
windows. Every open image or PDF window has its own child: opening never waits for a
child count. The spare is prepared only while fewer than four children (starting, ready,
active and retiring together) are live, so speculative warm-up never adds to a busy
machine. Preparation has a 30-second start budget; an unused ready
spare expires after 60 seconds. These limits do not interrupt a blocked native
process-creation call: its reservation remains owned until that call returns and
the child is terminated and reaped.

Committed disabled-provider/association settings and the installed catalog govern
eligibility; document providers are not selected for speculative preparation.
Policy/catalog generation changes, provider mismatch, changed pinned code,
expiry and shutdown invalidate the spare. Claim revalidates installed identity,
and the host retains the final live code/source guard immediately before Open.
A successfully opened session can request one background refill; failed preparation
does not create a speculative retry loop. The explorer prepares the spare once,
after a successful initial settings read, and revokes it once when exit is
requested; no timer polls the preparation policy.

Explicit Open requests take priority over refill and may share a matching start.
Canceling one waiter only detaches that waiter; another waiter's shared launch
continues. A stale spare rejected at engine admission is cleaned up before ordinary
cold Open under the same reservation. Once `open_prepared` begins, Open/broker/
decoder failures are terminal and never trigger an automatic cold retry.

Preparation, claim, cleanup and process reaping run on backend owners. UI-held
handles only configure, cancel and wake that owner. Shutdown invalidates pending
preparation/refill; cancellation and expiry retain child capacity through actual
cleanup. A prepared child cannot be recycled for another source after consumption.
The independent image/document API, SDK, MIV1 messages, sandbox and resource-policy
contracts remain unchanged.

**Host lifecycle contract.** `prepare_owned` / `open_owned` accept a
`Box<dyn Send>` ownership token. Clone
the same `ChildPermit` reservation into the host before opaque process
creation, so unclear spawn/reap outcomes retain actual child capacity. Use
`close_checked` and `cleanup_status` / `CleanupStatus` to observe cleanup;
unconfirmed process termination retains the reservation, cannot advertise
`Empty`, and cannot trigger a cold fallback. A pin-only close failure after
confirmed child termination frees child capacity but seals foreground admission
and spare refill. Cleanup errors are inspected before conversion to text.

The Windows typed executable pin contract retains a data-read, SHARE_READ-only
lease for the full `ImageSession`. Identity, length, modification time and trust
guards remain checked; only ChangeTime drift is tolerated while the lease denies
data writes. No executable hash is added. This contract leaves generic user
`CapturedFile`, manifest validation and Linux strict behavior unchanged; the
image viewer's user content is a viewer capture, which accepts a change-time-only
drift under its own policy (see
[change-time drift](CONTENT_READS.md#change-time-drift-view-05)). This worker-code proof
is distinct from own-EXE SHA default-OFF selection. These additions belong to the
Rust host API, not the plugin SDK, ABI or MIV1 protocol. Its native tests run on
Windows; the Linux-specific cases and UNC paths are not covered.

## Source ranges and archive memory

Archive streaming, private disk spooling and the small-content RAM budget (ARCH-01)
are implemented for archive workers of protocol 2 and later; the remaining application
qualification is tracked as ARCH-01 in the
[implementation backlog](../IMPLEMENTATION_BACKLOG.md). All raster formats use the
shared image-rs decoder path.

Broker `Read { offset, length }` requests allow a decoder to request only part of
the encoded file; they do not guarantee that the decoder itself streams. In the
pinned `image 0.25.10`, `JpegDecoder::new` calls `read_to_end` into its own `Vec<u8>`.
This happens for both native and archive JPEGs, so the image worker holds the
complete encoded JPEG during decoding, in addition to decoded pixels; that input is
dropped after decode. The image viewer therefore refuses a JPEG file over 96 MiB: a
progressive CMYK JPEG of 256 MiB of RGBA keeps about 512 MiB of coefficients besides its
output, and at that bound the decode peaked at a measured 835 MiB of the worker's 1 GiB.
Thumbnails use the same bound: their decode also buffers the whole file, and their
workers have 512 MiB.
A future format-specific decoder could read only an embedded preview. PSD is not
supported. The raster plugin retains the complete decoded still image, within its pixel
budget.
Broker source reads are available during Open only, not in later
render calls; deferred layer/content loading would need a separately qualified
protocol extension.

An archive member reaches a viewer as a content lease over the host's member
staging, not as a streaming source. The worker streams the selected member to the
host, which keeps it in memory when it is at most 16 MiB and the process-wide
128 MiB staging budget has room, and otherwise in a private host-owned spool file.
The host publishes the stage only after complete integrity verification: the first
read of a ZIP member decodes and checks that whole member, a 7z read decodes and
verifies its whole compression block, and compressed TAR first verifies its
complete outer stream into one scratch stage. A small viewer read can therefore
cause much more decompression and staging I/O than the requested range, even when
only a preview is displayed. The worker holds no complete encoded archive in RAM;
decoded content goes to host staging, except the readable siblings of a decoded 7z
block, which the worker keeps in memory (up to 128 MiB) until they are read.
`read_at` then reads checked ranges of the sealed stage, which each mount keeps for
up to 16 members. Viewer pixels are released with the decoder; staged members can
outlive the viewer while a panel or another reader retains the mount. Thumbnail
disk caching is separate. See the [filesystem API](FILESYSTEM_API.md) for the
staging protocol, the memory and spool budgets and the spool directories.

Replacing the JPEG decoder's own full-input buffer requires separately verified
streaming decoder behavior; disk spooling cannot remove that allocation.

The accepted
[ARCH-01 design](../superpowers/specs/2026-11-09-archive-streaming-design.md) keeps
small content under an aggregate RAM budget and spills only larger content to
private disk. The host does so with the 16 MiB stage limit and the 128 MiB
process-wide budget above; a stage that outgrows either moves its bytes
to a spool file. The design's other requirements remain: no
fixed archive or member byte-size ceiling, bounded working memory, cancellation,
integrity verification and explicit out-of-space errors (the host stops staging
before the spool volume would fall below its free-space reserve of max(1 GiB, 2% of
its capacity); see the [filesystem API](FILESYSTEM_API.md)). Application qualification must
cover that budget, measure peak memory across
both workers, not just broker chunk sizes, and cover preview-only reads, source
changes, cancellation, CRC failure and exhausted spool storage.

## Wire format and resource limits

Each message starts with `MIV1`, a little-endian u32 payload length and a u8 tag.
Image protocol 1 uses `Open`, `Metadata`, `Viewport`, `Rgba`, `Read`, `Data`
and `Failure` (tags 1–7). Document protocol 2 adds `DocumentMetadata` (tag 8:
u32 count then u32 width/height for each page) and `PageViewport` (tag 9:
u64 request ID, u32 zero-based page, then the existing viewport fields).
The `MIV1` envelope and existing tag encoding are unchanged. Protocol 2 uses
`PageViewport { id: u64, page: u32, viewport: Viewport }` after opening.
Request IDs bind each returned raster to its viewport. Paths,
URLs, UI objects, native handles and command strings are absent. Startup first
performs the worker SDK's sandbox-ready handshake; content is not parsed before
Windows AppContainer or Linux restriction succeeds. API 0.3.0 adds `FailureReason`
(tag 10), which may follow `Failure` in either protocol (see
[Failure reasons](#failure-reasons)).

| Resource | Bound |
|---|---|
| Physical viewport | `MAX_EDGE = 4096`, `MAX_PIXELS = 4 * 1024 * 1024` |
| Decoded image | `MAX_DECODED_EDGE = 32768`, `MAX_DECODED_BYTES = 256 * 1024 * 1024` (128 MiB before API 0.4.0) |
| Document pages | `MAX_DOCUMENT_PAGES = 4096`; nonempty, each logical page size validates as `Metadata` |
| PDF encoded source | 128 MiB, checked before allocation or source reads |
| Broker read/reply | `MAX_READ_BYTES = 256 * 1024` |
| Failure reason | `MAX_REASON_BYTES = 160` printable ASCII bytes, at most one, after `Failure` |
| Total source reads during Open | 1 GiB and at most 32,768 response-loop iterations (host) |
| Embedded ICC profile | 4 MiB |
| Worker process | 1 GiB memory (the worker SDK maximum), 60 cumulative CPU seconds |
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

The SDK exposes `Document::metadata(&self) -> DocumentMetadata` and
`Document::render(&self, page: u32, viewport: Viewport) -> io::Result<Frame>`.
`serve_document(decode)` takes the same `FnOnce(&mut dyn ReadSeek)` parser pattern
as `serve`, but validates document metadata and each page index before rendering.
The SDK enters the sandbox before either parser and allows broker reads only
during Open. Hayro 0.9.0 rasterizes each PDF viewport directly from retained PDF
content; it never allocates an entire zoomed document. Hayro's render caches (fonts,
glyph outlines and bitmaps, decoded images) are kept for the two pages rendered last at
their scale, so the bands of one page reuse them; the same host process/time limits as
for images apply.
Compiled standard fonts/CMaps and embedded document assets are available, while
filesystem/network font lookup, JavaScript and link activation are absent.
Password prompts and PDF editing are not implemented. Hayro is experimental;
unsupported rendering warnings reject the page and full PDF fidelity is not
claimed. See the plugin README and bundled license notices for exact limits.

### Decoded-size compatibility

API 0.4.0 raises `MAX_DECODED_BYTES` from 128 MiB to 256 MiB of RGBA8 (owner decision
of 26 November 2026, with the image workers' 1 GiB memory). The wire format is
unchanged and there is no negotiation. Hosts and workers both validate `Metadata`
against their own constant:

- A host with API 0.4.0 accepts every worker: an older worker never reports more than
  128 MiB.
- A host before API 0.4.0 with a worker built on SDK 0.5.0 refuses metadata between
  128 and 256 MiB as an invalid message, stops the worker and shows its plain "damaged
  or incomplete" sentence, not a limit. This is accepted as is: the image and PDF
  viewer plugins ship with the application, so a release pairs them with a matching
  host.

### Failure reasons

API 0.3.0 adds `FailureReason` (tag 10): 1–160 bytes of printable ASCII
(0x20–0x7E) without `/` or `\`, so it carries no control or direction characters,
paths or URLs. A worker may send it only directly after `Failure`, as its last
message. SDK 0.4.0 writes `Failure`, then the reason, when Open, a document's
metadata or a render fails, and ends the session. The reason is the provider's error text
reduced by `FailureReason::sanitize`: whitespace and controls become spaces, other
non-ASCII characters `?`, words containing `/` or `\` become `...`, and longer text
is cut to 160 bytes ending in `...`. Text without a letter or digit sends no reason.

Compatibility needs no negotiation in either direction:

- Hosts before API 0.3.0 stop reading at `Failure` and stop the worker, so they
  never read a newer worker's reason.
- A newer host reads on after `Failure` until the reason or the end of the stream,
  under the session's cancellation and for at most one second, then stops the
  worker. Workers end after `Failure`, so this normally takes milliseconds. A worker
  built with SDK 0.3.0 or earlier sends no reason, and the host keeps the plain
  sentence. Stopping the worker first could lose the reason: Rust's line-buffered
  stdout splits the write at the tag byte (0x0A).
- A malformed, oversized or unexpected trailer is ignored, never a protocol error;
  its declared length is checked before the body is read.

The host shows the reason as the second part of the error, with a capitalized first
word lowercased: "This image is damaged or incomplete: the file is empty", "This
document is damaged or incomplete: …", "This image could not be drawn: …" or "This
page could not be drawn: …". Without a reason the texts are unchanged ("This image
is damaged or incomplete, or its format is not supported."). The reason comes from
untrusted content and is shown as plain text only. The shared image decoder reports
an empty source ("The file is empty") and non-image content ("The content is not a
recognized image format") before trying SVG; a PNG cut short reports "unexpected end
of file". The PDF viewer reports, for example, "The PDF structure is invalid or
incomplete", "The document needs a password or uses unsupported encryption" and "The
page contains content this viewer cannot draw".

API 0.3.0 carries a reason's text, not its kind, yet a limit or unsupported content
is not damage. Until a kind exists (VIEW-06), the text decides: a refused Open whose
reason is a complete sentence starting with "This " shows that sentence alone, with a
full stop added if it lacks one ("This SVG is larger than 4 MiB, the limit for SVG
images."); any other reason continues the "damaged or incomplete" sentence ("This
image is damaged or incomplete: the SVG is not well-formed XML: the document ends too
early (line 1, column 75)"). Providers should phrase limits and unsupported content
that way and damage as a fragment. The shared image decoder does; the PDF viewer's
limit texts ("The document has more than 4096 pages") still read as damage. Hosts
before this rule show a sentence after their own ("This image is damaged or
incomplete: this SVG is larger than 4 MiB, …").

## Independent build and verification

`scripts/build-image-viewer.ps1` and `.sh` build/package either raster worker and
are called by `build-viewers` for image and PDF. Select PDF with `-Viewer pdf` on
PowerShell or the fourth shell argument `pdf`; existing image invocations remain
valid. Their worker SDK config is independent of the native viewer
SDK (`-WorkerSdkConfig` or `MIDNIGHT_IMAGE_VIEWER_SDK_CONFIG`). Without a supplied
config they cache a source-stamped immutable bundle from `worker-packages`.
PowerShell wrappers restore both the values and absence of saved environment
variables, so sequential image/PDF builds also work when `CARGO_TARGET_DIR` was
initially unset.
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
The independent PDF project generates multipage, standard-font, cropped/rotated
and vector zoom/pan fixtures. `MIDNIGHT_PDF_VIEWER_TEST_APPLICATION` enables the
host's `pdf_native` tests for real sandboxed page rendering, source retention,
changed revisions and cancellation. CI exports the public SDK and sparse-checks
each plugin alone on Windows and Linux before host integration.

### Host startup diagnostics

`midnight-image-viewer-host` has a default-off `startup-diagnostics` feature. It
also enables the same feature on the shared `midnight-worker-platform` crate.
Build `cargo build -p midnight-image-viewer-host --example image_startup
--features startup-diagnostics`, using the normal SDK config and `_build` target,
then run `image_startup <trusted-app-executable> <existing-image>`.

The host records provider/source validation, restricted process startup, Ready,
Open, first broker request, I/O admission and decoded metadata. Platform-specific
marks split native startup further. Each operation buffers at most sixteen fixed
labels and elapsed-millisecond values, then writes one stderr line; paths,
environment values and content are excluded. No worker wire/API or sandbox
behavior changes. Without the feature the timing code is compiled out.
See the [measured startup attribution](../verification/2026-11-11-image-loading.md)
for interpreting cold native-process delays separately from decoding and UI
publication.

### Prewarming checks

Headless host tests on Windows cover prewarming: a prepared spare opens with no
process creation and no source validation or read during preparation, and owned
cleanup completes. In one recorded run on a 2x1 BMP, cold Open to exact CPU pixels
took about 254 ms and Open of a ready spare about 139 ms, preparation being a
separate interval of about 205 ms; engine refill and the application UI were not
timed, and other images, scrolling, idle memory and peak resources are not measured
([evidence](../verification/2026-11-13-checkpoint-evidence-history.md#image-prewarming-source-and-evidence)).
