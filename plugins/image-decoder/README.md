# Shared restricted image decoding

This independent library is shared by the image viewer and image thumbnail
executables. It depends on no private application crate or host API. Both callers
enter their existing Windows AppContainer/Linux worker restrictions before
parsing. Only opaque seek/read source bytes enter this library.

PNG, JPEG, GIF, WebP, BMP, ICO and TIFF use image-rs 0.25.10 and moxcms 0.8.1.
Orientation and embedded RGB ICC profiles are honored; output is straight-alpha
sRGB RGBA8. The first still frame is used. Original dimensions are limited to
32768 pixels per edge and 128 MiB of RGBA storage; profiles to 4 MiB.

Plain UTF-8 SVG uses resvg/usvg 0.48.1 with only `text` enabled. Shapes, paths,
gradients, bounded local `use` references and text are supported. Thumbnails
rasterize directly to the requested aspect-preserving size without upscaling.
Viewers rasterize at original dimensions and retain those pixels, so later zoom
uses the existing image sampler rather than rerendering vector detail.

SVG input is limited to 4 MiB, 32768 XML nodes, nesting depth 64 and 65536 expanded
reference visits. DTDs, duplicate IDs, reference cycles, scripts, event handlers,
foreign objects, embedded/external images and subtree effects (`filter`, `mask`,
`clipPath`, `marker`, `pattern`) are explicitly rejected. SVGZ is not supported.
The SVG root must use its standard namespace. Both image resolvers return no
resource; no system-font, memory-mapped-font or raster-image feature is enabled.
The existing worker process memory/CPU limits and host cancellation/deadlines also
bound parsing, font shaping and rasterization. This library does not make those
synchronous CPU stages independently interruptible inside the worker.

Unchanged OFL-1.1 Noto Sans font bytes are embedded in memory, with Latin,
Greek/Cyrillic, Arabic and Hebrew fallback. Unknown requested font families use
Noto Sans; exact system-font appearance and full Unicode coverage are not claimed.
See [font provenance and hashes](fonts/README.md) and [OFL](fonts/OFL.txt).
Missing glyphs outside this bundled coverage may use the font's missing-glyph box.

AVIF uses image-rs's native decoder with dav1d 0.11.0 bindings and statically
linked dav1d 1.5.4. The narrowly patched image-rs constructor receives the same
settings before decoding either color or alpha: one thread, frame delay one,
strict AV1 compliance, one selected layer and at most 16,777,216 pixels per frame.
That pixel cap accounts for 16-bit 4:4:4 color plus 16-bit alpha storage; it is
separate from the existing 32768-edge and 128 MiB decoded-output limits. Native
references, stride padding, conversion scratch and parser metadata are additionally
contained by the worker process budget; files near a limit can still be refused.

Only AVIF stages its encoded container before parsing, capped at 32 MiB with an
explicit excess-input error. This prevents the upstream tolerant parser from
hiding a read-limit error in a trailing box. Existing PNG/JPEG/etc source reads
remain streamed and acquire no new encoded-source ceiling. The upstream AVIF
container parser retains its own bounded-input copies. The first primary still
image is used; animation, derived grids, HDR tone mapping and complete AVIF
metadata fidelity are not claimed. RGB ICC conversion uses the common policy;
otherwise image-rs's native color conversion supplies the pixels.

See [native build instructions](native/README.md) and the recorded
[image-rs patch](../../vendor/image/MIDNIGHT_PATCH.md). The MIT bindings,
BSD-2-Clause native decoder and MPL-2.0 parser notices are distributed with both
workers; the exact unmodified mp4parse source archive accompanies its license.
HEIC/HEIF remains unsupported: this project has no qualified packaged HEVC decoder
or royalty-free distribution determination.

Native package qualification covers Windows x64 and Linux x86-64. ARM64 tooling
is deferred under the application's current system requirements.
