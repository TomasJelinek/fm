# PDF viewer

Independent, removable PDF raster provider for Midnight Explorer. It depends only
on the released public raster SDK 0.2.0 and pinned `hayro` 0.9.0; no private host
crate or UI toolkit is linked. Installing/removing its package requires restart.
Windows x86-64 and Linux x86-64/aarch64 use the existing native worker sandbox.
Hayro itself is portable Rust; a macOS host sandbox has not been implemented.

The viewer opens all supported pages, with page navigation, fit, actual size,
zoom and pan supplied by the host. Space retains the host's captured file cycle;
Esc closes the viewer. PDF coordinates are points (1/72 inch before page scaling),
and every zoom request rasterizes the selected page at the requested scale.
No first-page-only preview or whole-document raster cache is used.

The worker receives source length and bounded opaque reads, never a pathname,
native handle or URL. It enters the native sandbox before parsing. It reads at
most 128 MiB of encoded source once and retains at most 4,096 page descriptions.
Each page size obeys the public metadata limits. Viewports remain at most 4,096
pixels on either edge and 4,194,304 pixels total. The host independently caps
process memory at 512 MiB and cumulative CPU at 60 seconds, with cancellation
and per-operation wall deadlines. Render caches are discarded after each frame.
Source identity validation remains host-owned before/after opening and rendering.

Only embedded document assets and compiled standard fonts/CMaps are available.
JavaScript, external links, attachments, network retrieval, filesystem font lookup,
forms editing and password prompts are not implemented. Documents requiring a
password are rejected; empty-password encrypted documents may open if Hayro can
decrypt them. Hayro is experimental: some PDF constructs and nonembedded fonts
may render differently or be unsupported. Reported missing-font/image/annotation
warnings reject the page; the plugin does not promise complete PDF fidelity.

Build/test against an exported public SDK bundle, with generated files under `_build`:

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
own PDF fixtures. The distributed `notices/` directory is required, along with
the complete pinned dependency list in `THIRD-PARTY.md`.
