# Shared SVG and AVIF image decoding implementation plan

Implemented and qualified on 12 November 2026. The image viewer and thumbnail
workers share one independent decoder while retaining their public SDK and native
sandbox boundaries. The shared crate depends on no application or host crate.
No host protocol, public API or source-path authority changed.

## Design and limits

PNG, JPEG, GIF, WebP, BMP, ICO and TIFF retain the shared image-rs 0.25.10 and
moxcms 0.8.1 policy. Plain UTF-8 SVG uses resvg/usvg 0.48.1 with only its text
feature. Unmodified OFL Noto Sans regular, Arabic and Hebrew font bytes are
embedded from the pinned cosmic-text source, with provenance and SHA-256 hashes.
System fonts, source-selected files, URLs and embedded images are unavailable.
Scripts, foreign objects and unsupported subtree effects fail explicitly.

SVG is limited to 4 MiB, 32768 XML nodes, depth 64 and 65536 expanded local
reference visits. DTDs and cycles are refused before conversion. The common
original-image limits remain 32768 pixels per edge and 128 MiB decoded output.
Thumbnails rasterize directly to the requested aspect fit, without upscaling;
rounded extents are capped to the requested size. Viewers retain original-size
pixels and use their existing viewport sampler rather than rerendering vectors.

AVIF uses image-rs's native decoder with dav1d 0.11.0 bindings and pinned static
dav1d 1.5.4. The minimal image-rs constructor patch supplies the same settings
before primary and alpha decode: one thread, frame delay one, strict compliance,
one selected layer and 16777216 pixels per frame. AVIF alone stages its encoded
container, limited to 32 MiB before tolerant parsing. Existing raster source reads
remain streamed; no new host capture ceiling or full-source read was introduced.
Native scratch/reference allocations remain inside the existing worker budgets.

## Completed work

- [x] Record real SVG and AVIF failures before implementing their shared decoder.
- [x] Extract existing raster/color behavior and route both independent workers
      through it; retain orientation, RGB ICC, alpha and viewport regressions.
- [x] Add bounded SVG parsing, bundled-only font fallback and explicit unsupported
      resource handling; prove the namespace-reset refusal regression.
- [x] Add bounded primary/alpha native AVIF decoding and prove both limits using
      real bitstreams; close the reproduced trailing-box input-cap bypass.
- [x] Reproduce and fix 13x13-to-7x7 SVG rounding at shared, thumbnail and actual
      restricted-host boundaries; preserve non-square fit and no-upscale behavior.
- [x] Qualify pinned private native bootstrap/static builds and the real standalone
      viewer/thumbnail packaging entrypoints on Windows x64 and Linux x86-64.
- [x] Run 34 affected decoder/worker tests per platform and strict all-target
      Clippy for all three independent workspaces, with bounded parallel runners.
- [x] Run all nine actual production-package host cases on each platform, including
      shipped associations, exact pixels, alpha, retained zoom and cancellation.
- [x] Advertise SVG/AVIF only after decode and package qualification; update provider
      versions and the thumbnail codec cache identity to r3.
- [x] Retain 213 exact notice/source files in each package, including native BSD
      terms, OFL fonts and the unchanged MPL parser source archive; refresh/check
      the dependency inventory and format/build documentation.

## Final evidence and remaining scope

See [decoder and package verification](../../verification/2026-11-12-image-formats.md),
[actual restricted-host results](../../verification/2026-11-12-image-host-formats.md)
and [pinned native tooling](../../verification/2026-11-12-native-av1.md).
Both final packages link dav1d statically. Windows imports only system/CRT
libraries; Linux imports only standard system libraries and has no development
RPATH. Explicit private-install permissions are required for Linux host trust;
a harness copy preserving DrvFS writable modes was correctly refused before the
unchanged tests passed from a correctly protected installation.

HEIC/HEIF remains unavailable: no packaged decoder and distribution terms have
been qualified under the project's requirements. SVGZ, full SVG effects/images,
full Unicode/system-font fidelity, AVIF animation/derived grids/HDR tone mapping,
and ARM64 native package tooling are outside this qualified batch.
