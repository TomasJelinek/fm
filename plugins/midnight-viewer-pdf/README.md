# PDF viewer

Independent, removable PDF raster provider for Midnight Explorer. It depends only
on the released public raster SDK 0.4.0 (API 0.3.0) and pinned `hayro` 0.9.0; no private host
crate or UI toolkit is linked. Installing/removing its package requires restart.
Windows x86-64 and Linux x86-64/aarch64 use the existing native worker sandbox.
Hayro itself is portable Rust; a macOS host sandbox has not been implemented.

The viewer opens all supported pages, with page navigation, fit, actual size,
zoom and pan supplied by the host. Space retains the host's captured file cycle;
Esc closes the viewer. PDF coordinates are points (1/72 inch before page scaling),
and every request rasterizes the selected page part at the requested scale: glyphs
and paths are antialiased and horizontal text is hinted at that exact size. The host
asks for bands of 256-pixel tiles at the shown zoom and keeps them for scrolling; no
first-page-only preview or whole-document raster is produced here.

The worker receives source length and bounded opaque reads, never a pathname,
native handle or URL. It enters the native sandbox before parsing. It reads at
most 128 MiB of encoded source once and retains at most 4,096 page descriptions.
Each page size obeys the public metadata limits. Viewports remain at most 4,096
pixels on either edge and 4,194,304 pixels total. The host independently caps
process memory at 512 MiB and cumulative CPU at 60 seconds, with cancellation
and per-operation wall deadlines. Hayro's render caches (parsed fonts, glyph
outlines and bitmaps, decoded images) are kept for the two pages rendered last at
their scale, so the bands of one page reuse them (about a fifth less time per page;
[measurements](../../docs/verification/2026-11-21-pdf-scrolling.md)); any other page or
scale replaces the older entry. The
parsed document lives as long as the worker process, which serves one document.
Source identity validation remains host-owned before/after opening and rendering.

Only embedded document assets and compiled standard fonts/CMaps are available.
JavaScript, external links, attachments, network retrieval, filesystem font lookup,
forms editing and password prompts are not implemented. Documents requiring a
password are rejected; empty-password encrypted documents may open if Hayro can
decrypt them. Hayro is experimental: some PDF constructs and nonembedded fonts
may render differently or be unsupported. Reported missing-font/image/annotation
warnings reject the page; the plugin does not promise complete PDF fidelity.

Its error texts reach the viewer's status line through the SDK's failure reason,
after the host's sentence: an empty file, an invalid or incomplete structure,
a password or unsupported encryption, a document over 128 MiB or 4,096 pages, an
oversized or invalid page, and a page with content Hayro cannot draw each have their
own short, path-free text (for example "This document is damaged or incomplete: the
document needs a password or uses unsupported encryption").

Build/test against an exported public SDK bundle, with generated files below a `_build`
directory: this project's own, that of the repository holding it in `plugins/`, or
otherwise the first `_build` of the path. That `_build` and every folder below it must
not be a link or junction; the folders above it may be links or hidden, such as
`AppData`:

```powershell
./scripts/ci.ps1 -SdkConfig SDK_CONFIG -BuildRoot _build/pdf-plugin -Profile debug
./scripts/build.ps1 -SdkConfig SDK_CONFIG -BuildRoot _build/pdf-plugin -Profile debug
```

```sh
MIDNIGHT_PDF_VIEWER_PROFILE=debug bash scripts/ci.sh SDK_CONFIG _build/pdf-plugin
MIDNIGHT_PDF_VIEWER_PROFILE=debug bash scripts/build.sh SDK_CONFIG _build/pdf-plugin
```

`cargo fetch` is an explicit dependency-preparation step; CI and package builds
use `--locked --offline`. Plugin tests live in this project and generate their
own PDF fixtures. `examples/render_probe.rs` generates two synthetic text-heavy
Letter reports (3 and 300 pages: headings, bullet lists, a table with 0.5-point
borders and a small logo), times in-process rendering, and `--write DIR` stores them
for the host's scroll probe. The distributed `notices/` directory is required, along with
the complete pinned dependency list in `THIRD-PARTY.md`.
