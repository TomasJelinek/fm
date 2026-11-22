# Restricted image thumbnails

The independent worker uses [the shared decoder](../image-decoder/README.md) for
PNG, JPEG, GIF, WebP, BMP, ICO, TIFF, bounded static SVG and AVIF. The first still frame
is reduced without upscaling, with alpha-weighted averaging so pixels next to
transparency are not darkened or tinted, then oriented and converted to straight-alpha
sRGB. A frame that already fits the cell is not resampled.
SVG rasterizes directly to the requested aspect ratio with embedded Noto fonts,
through the same decoder path as the viewer: markers, patterns, clip paths, masks,
filters, text and `data:` raster images are drawn; it receives no path, network or
system-font access. Its drawing bounds apply at the thumbnail's size, so an SVG
whose filters or layers are too costly at full size can still get a thumbnail. See
the shared decoder's [SVG contract](../image-decoder/README.md#svg), resource limits
and bundled font provenance.

## PDF first pages

The worker also previews PDF documents, with the PDF viewer's renderer: pinned
Hayro 0.9.0 with its compiled standard fonts and CMaps (`embed-fonts`,
`embed-cmaps`) and no filesystem, network or system-font lookup. No file name
reaches the worker, so the content decides. Content that starts with an image
signature that the image decoder chooses a codec by (image-rs's, such as PNG or
JPEG), or with markup that it reads as SVG, goes to the image decoder even when its
metadata, text or comments mention `%PDF-`. Otherwise `%PDF-` within the first
kilobyte selects the PDF path, and anything else goes to the image decoder. The
bytes needed for that choice are the first brokered read that the image decoder
makes anyway, so sniffing adds no host round trip.

- Only page 1 is drawn, as an opaque white page fitted inside the requested cell
  with its aspect ratio. A page is vector content, so a small page is drawn at the
  cell's size rather than enlarged from a smaller raster.
- The document is read whole, once, and at most 64 MiB: a larger source fails before
  any of it past the first read is fetched. Page edges must be 1–32,768 points.
- Password-protected, damaged and page-less documents fail, and so does a page
  that the renderer can draw only with warnings (an unloadable font or image),
  exactly as the PDF viewer refuses it. The host then keeps the placeholder.
- The host's limits for every thumbnail worker still apply: 512 MiB of memory,
  8 CPU seconds, a 10-second deadline and 256 MiB of brokered reads per request.

The provider version in `thumbnailer.json`
(`image-0.25.10-moxcms-0.8.1-resvg-0.48.1-dav1d-1.5.4-hayro-0.9.0-r6`) names the pinned
decoders and a policy revision. It changes whenever decoding policy changes, which
invalidates the cached thumbnails' identities, so they are regenerated once. AVIF
shares the viewer's pinned static dav1d decoder,
32 MiB encoded-container cap, and primary/alpha native frame limits. HEIC/HEIF is
not enabled. The root thumbnail build scripts prepare pinned native tools/static
code and copy the full shared notice set, including the MPL parser source archive.
Use `-NativeOffline` or `MIDNIGHT_IMAGE_NATIVE_OFFLINE=1` to forbid native downloads.

`notices/` holds the reviewed notices that crate packages omit, which the notice
staging adds under `licenses/`: the Foxit standard fonts and Adobe CMaps that
Hayro embeds, its other asset notices and upstream NOTICE, and the BSD-3-Clause
license that alloc-stdlib 0.2.4 does not publish. They are the PDF viewer's
reviewed copies. The development profile optimizes all dependencies, like the
viewer plugins, so debug builds draw pages within the host's CPU limit.

`scripts/ci.ps1 -SdkConfig <released worker SDK .cargo/registry.toml> -BuildRoot
<directory below _build> [-NativeOffline]` (Windows, PowerShell 7) and
`bash scripts/ci.sh <config> <build root>` (Linux; `MIDNIGHT_IMAGE_NATIVE_OFFLINE=1`
forbids native downloads) run the checks of the `thumbnail-image` CI job: the pinned
native dav1d build, formatting, tests, strict Clippy and an optimized build. Cargo runs
offline there, so fetch the dependencies first, as the job does:
`cargo --config <config> fetch --manifest-path Cargo.toml --locked`.
On Linux the build root must lie below this project's own `_build`, that of the
repository holding it in `plugins/`, or otherwise the first `_build` of the path; that
`_build` and every folder below it must not be a symlink, while the folders above it
may be links. The native helpers apply the same rule on both platforms.
