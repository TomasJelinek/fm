# Shared restricted image decoding

This independent library is shared by the image viewer and image thumbnail
executables. It depends on no private application crate or host API. Both callers
enter their existing Windows AppContainer/Linux worker restrictions before
parsing. Only opaque seek/read source bytes enter this library.

PNG, JPEG, GIF, WebP, BMP, ICO and TIFF use image-rs 0.25.10 and moxcms 0.8.1.
Orientation, embedded RGB ICC profiles and gray ICC profiles of gray images are
honored; output is straight-alpha sRGB RGBA8. image-rs converts CMYK samples to RGB
without a profile, so an embedded CMYK profile is not applied, exactly as for CMYK
files without one. An image whose embedded profile has another color model, or a
gray profile on a color image, fails to decode with an `Unsupported` error rather
than being claimed sRGB. CICP-only color signalling (AVIF nclx, PNG cICP) is not
converted: those pixels are used as sRGB. The first still frame is used. Original
dimensions are limited to 32768 pixels per edge and 128 MiB of RGBA storage;
profiles to 4 MiB. Thumbnails reduce the frame first, averaging colors with alpha
weights so pixels next to transparency are not darkened or tinted, and orient the
small result afterwards. A frame that already fits the requested bound is not
resampled.

Source reads go through a seek-preserving window of at most 256 KiB: it batches
small codec reads into brokered reads and keeps fetched bytes across codec seeks,
so TIFF strips and tiles are not refetched once per strip.

## SVG

Plain UTF-8 SVG uses resvg/usvg 0.48.1 with the `text` and `raster-images` features.
Thumbnails rasterize directly to the requested aspect-preserving size without
upscaling. Viewers rasterize at original dimensions and retain those pixels, so later
zoom uses the existing image sampler rather than rerendering vector detail.

Everything resvg draws is drawn: paths and shapes, text, gradients, patterns, markers
(including `orient="auto-start-reverse"`), clip paths, masks, filters, local `use`,
`switch`, style sheets and raster images embedded as `data:` URIs. Embedded PNG, JPEG,
GIF and WebP go through this library's raster path with every raster limit (32768
pixels per edge, 128 MiB per image, ICC to sRGB, orientation), then reach resvg as a
PNG that this library encoded, so resvg's own decoders never see document bytes.
Identical data is decoded once; all distinct embedded images share 128 MiB of pixels.

Content that is not drawn is skipped, never refused, and named in the note that
`decode_with_note` returns ("Not shown: linked images, scripts."):

- `script` elements and event attributes (`on…`): never run.
- `foreignObject` (HTML): skipped; inside a `switch`, its SVG fallback is drawn.
- Links to files or URLs: `image`/`feImage` and `use` hrefs and `url(...)` paint, clip,
  mask, filter and marker references that are not `#local`. Neither image resolver
  opens anything: no resource directory, filesystem or network access.
- Embedded images that are not PNG, JPEG, GIF or WebP (BMP, nested SVG, …), are
  damaged, or exceed a raster limit or the shared budget.

The note is computed and tested, but image viewer API 0.3.0 cannot carry it to the
status line yet (VIEW-06 in the [backlog](../../docs/IMPLEMENTATION_BACKLOG.md)).

A document that is malformed or over a bound is refused with the exact reason and
bound. Damage is a fragment that continues the host's "This image is damaged or
incomplete:" ("The SVG is not well-formed XML: the document ends too early (line 1,
column 75)", "The SVG text is not valid UTF-8 (line 2)"). A limit or unsupported
construct is a complete sentence starting with "This ", which the host shows alone.
The bounds are protection against hostile input, generous for real files:

| Bound | Value | Where |
| --- | --- | --- |
| Source size | 4 MiB | before parsing |
| XML nodes (elements, text runs, comments) | 262,144 | parser |
| Nesting of elements and references | 64 levels | before conversion |
| Converted tree after expanding references, estimated as 1 KiB per element plus 4 bytes per byte of attributes and text (so at most 262,144 elements or 64 MiB of data) | 256 MiB | before conversion |
| Style-rule checks (rules × elements) | 2^27 | before conversion |
| Original size | 32768 px per edge, 128 MiB | after conversion |
| Memory while drawing (tree, embedded images, canvas, buffers) | 448 MiB | before rendering |
| Weighted pixel operations at the drawing size | 2^33 | before rendering |

Expansion counts what usvg instantiates: each `use` target again, marker content once
per vertex (bounded by the shape's numbers), and pattern, clip-path, mask, filter and
`feImage` content once per referring element, with references from presentation
attributes, `style` attributes, inherited `fill`/`stroke`/`marker-*`, `context-fill`
and style sheets matched with usvg's own selector library. The href usvg follows wins:
an unprefixed `href` before `xlink:href`, in any attribute order. Reference cycles are
refused ("This SVG is invalid: the reference on line N leads back to itself."), except
a marker within its own content, which usvg skips. A repeated ID and a reference to a
missing element do not refuse the document: usvg ignores what it cannot resolve.
The drawing bound follows resvg's recursion: layers for opacity, clip paths, masks,
filters and blend modes (clamped by resvg to five times the canvas per side), their
mask buffers, every filter primitive's result over the filter region, pattern tiles at
device scale, images decoded at each draw, and the dashes of dashed strokes (tiny-skia
allows a million per path drawn). The weights come from micro-benchmarks of resvg 0.48.1
(one unit is about 3 ns of release work here, so 2^33 units is roughly half a minute):
a box blur weighs 56 per region pixel at any radius (running sums), the recursive blur
used below 2 device pixels 110, a turbulence octave 36, a convolution kernel element 3,
a morphology kernel element 1, a pattern-filled pixel 3 and a dash 100. The memory
bound adds the reference pass's tree estimate (measured 0.6 to 0.9 KiB per converted
element and 3.5 bytes per byte of path data) to the buffers, leaving 64 MiB of the
workers' 512 MiB for code, fonts and allocator slack.

Measured on 21 November (release build, Windows development machine while other builds
ran): the largest real-world-like case, a 4000 × 3000 flowchart of 1,200 boxes with a
gradient and a shared drop-shadow filter, 2,400 text spans, arrow markers, a patterned
background and a clipped page group (399 KB), takes 1.3 to 2.7 s and 175 to 225 MiB at
full size, 0.16 s at a 256-pixel thumbnail; its estimates are 20,408 elements (8% of
the tree bound), 163 MiB of drawing memory and 0.86 billion work units (a tenth of
the bound). A 3000 × 2000 map of 20,000 clipped polygons, a 50,000-point marker chart
and a 6-megapixel embedded photo take 0.2 to 1.1 s and 60 to 120 MiB. Hostile files
just under a bound take about 27 s (blur layers at the work bound), 1 s and 200 MiB
(262,000 elements) and 4 s and 235 MiB (64 MiB of path data); 200 full-canvas blurred
layers, which took 113 s with the first weights, are now refused in 2 ms. A DOCTYPE without an internal subset (`<!DOCTYPE svg PUBLIC …>`) is
blanked before parsing, keeping line numbers; an internal subset, where entities are
declared, is refused, so entities are never expanded. SVGZ is not supported. The SVG
root must use its standard namespace. The worker's memory/CPU limits and host
cancellation/deadlines remain the last bound; these synchronous stages are not
independently interruptible inside the worker.

Unchanged OFL-1.1 Noto Sans font bytes are embedded in the executable, with Latin,
Greek/Cyrillic, Arabic and Hebrew fallback. A font-family list without one of these
falls back to its generic family or to Noto Sans, so text never disappears. The fonts
are registered by reference (no copy) and only for documents with text. Exact
system-font appearance and full Unicode coverage are not claimed. See
[font provenance and hashes](fonts/README.md) and [OFL](fonts/OFL.txt). Missing
glyphs outside this bundled coverage may use the font's missing-glyph box. System
fonts are deliberately not passed in from the host: that needs a host API, exposes
the installed font set to document content and makes rendering differ by machine.

AVIF uses image-rs's native decoder with dav1d 0.11.0 bindings and statically
linked dav1d 1.5.4. The narrowly patched image-rs constructor receives the same
settings before decoding either color or alpha: one thread, frame delay one,
strict AV1 compliance, one selected layer and at most 16,777,216 pixels per frame.
That pixel cap accounts for 16-bit 4:4:4 color plus 16-bit alpha storage; it is
separate from the existing 32768-edge and 128 MiB decoded-output limits. Native
references, stride padding, conversion scratch and parser metadata are additionally
contained by the worker process budget; files near a limit can still be refused.
The patch also ends construction with an error when the color or alpha item yields
no displayable frame (for example only a sequence header), instead of waiting for
one forever.

Only AVIF stages its encoded container before parsing, capped at 32 MiB with an
explicit excess-input error. This prevents the upstream tolerant parser from
hiding a read-limit error in a trailing box. Existing PNG/JPEG/etc source reads
remain streamed and acquire no new encoded-source ceiling. The upstream AVIF
container parser retains its own bounded-input copies. The first primary still
image is used; animation, derived grids, HDR tone mapping and complete AVIF
metadata fidelity are not claimed. ICC conversion uses the common policy;
otherwise image-rs's native color conversion supplies the pixels.

See [native build instructions](native/README.md) and the recorded
[image-rs patch](../../vendor/image/MIDNIGHT_PATCH.md). The MIT bindings,
BSD-2-Clause native decoder and MPL-2.0 parser notices are distributed with both
workers; the exact unmodified mp4parse source archive accompanies its license.
HEIC/HEIF remains unsupported: this project has no qualified packaged HEVC decoder
or royalty-free distribution determination.

Native package qualification covers Windows x64 and Linux x86-64. ARM64 tooling
is deferred under the application's current system requirements.
