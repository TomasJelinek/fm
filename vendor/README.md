# Renderer source patch

`femtovg/` is the published FemtoVG 0.27.0 source, licensed **MIT OR Apache-2.0**.
The application selects MIT and retains both upstream license files. Upstream:
https://github.com/femtovg/femtovg . The registry crate and its VCS metadata are
recorded by the vendored package. Cargo registry caches are not modified.
The published crate SHA-256 is
`31eebf3ab1b76359ccd5f850c07a7c6b2722c64d9127c40e02358e83f3ff9b1a`;
its recorded upstream commit is `0eb55780894a9200e9a70ca3d16bd1c641281a44`.
The local manifest marker keeps this path dependency in license inventories and
release notices; it does not turn it into project-owned code.

A feature-gating correction in `femtovg/src/lib.rs` keeps the `FilePath` import
only when image loading or text layout uses it, and the private `font_scale`
helper only with text layout. This removes unused-item warnings for the app
feature set (`swash`, `wgpu`) without changing enabled rendering paths. Version
and the MIT license choice remain unchanged; the one manifest change is the
`skrifa` dependency of the text fitting described below.

The host workspace patch adds bounded reuse of managed image/glyph bind groups.
Native DX12 stack sampling identified per-draw bind-group allocation and release
as a scrolling bottleneck: each sampler binding creates an internal GPU buffer.
The upstream mapper retained only the current group and rebuilt groups each frame.

Cache entries validate actual texture identities and sampler flags, not just
image ids (reallocation can preserve an id); hits compare them by reference.
Changing the uniform buffer clears old bindings; image deletion removes all
references to that image. Reallocated or deleted unused images are pruned after the
next nonempty rendered frame or renderer drop, and a draw that still references an
id deleted before the flush is bound without caching it. Pixel-only image updates
retain their bindings. Transient unmanaged filter textures bypass the cache. The
cache holds at most 512 bindings per renderer and evicts the least recently used
binding of an earlier frame. A frame that uses more distinct bindings than that
keeps its cached set and binds the overflow uncached, as upstream does, instead of
evicting bindings it is about to reuse. On DX12 each cached binding also owns a
64 KiB upload-heap placement for its sampler indices, so the cap retains at most
about 32 MiB of CPU-visible memory per renderer. There is no cross-thread or
cross-device cache, and WGPU retains resources needed by submitted commands.

A second local change reuses upload staging for vertices and fragment uniforms.
Correlated native idle-redraw stacks reached DX12 committed-buffer allocation
through `Queue::write_buffer`, even when the first redraw still showed unchanged
content. `wgpu/uploads.rs` retains at most eight mapped COPY_SRC buffers of at
most 2 MiB each (16 MiB cache cap). It records copies before the render passes,
then fills the reserved uniform region and unmaps before command submission. The
uniform reservation is each command's upper bound (none for a render-target
switch, two for concave fills, stencil strokes and Gaussian blurs, otherwise
one); debug builds check every command against it. WGPU 30's
`map_buffer_on_submit` makes a slot reusable only after successful GPU completion
and remapping; `begin` registers it on the encoder that records the slot's
copies, and the returned upload must be finished before that encoder's submit. A
dropped deferred callback or failed mapping retires the slot; it is never treated
as mapped. Polling never waits for the GPU. In the usual steady state the previous
frame's slot is still in flight at the next render, so there is typically one
nonblocking `device.poll` per render; it can run WGPU's deferred resource
destruction and pending callbacks inside `render`.

Busy or oversized uploads use uncached staging with copies in the same command
buffer, preserving ordering when several deferred renders share destinations.
The 16 MiB limit bounds the retained pool, not the total memory of arbitrary
caller-owned unsubmitted command buffers and their one-shot sources. Those
sources remain alive until their GPU commands finish or are discarded. The
application submits frames promptly; the renderer does not add submissions,
block for completion, or change GPU/driver workarounds. Texture allocation reuse
and pipeline retention are not part of this change. GPU device loss remains fatal
as upstream: mapping a staging range after loss panics in `render`.

A third change retains at most 32 immutable viewport bind groups per renderer,
keyed by the exact two viewport float bit patterns. The fixed bind-group layout
belongs to that renderer/device. Each entry owns its original 16-byte uniform
resource through the bind group (a 64 KiB placement on DX12, so at most about
2 MiB per renderer); hits never rewrite buffers. Least-recently-used
eviction drops only cache ownership, so deferred/in-flight GPU commands retain
their bindings safely. This removes the measured per-render viewport buffer
allocation without changing device polling or texture lifetimes.

A fourth change adds an opt-in fitting of filled text to the pixel grid. In the
crate it is off by default: with `GlyphFitting::Subpixel` glyph bitmaps, their
cache keys and their quads are exactly upstream's. `femtovg::set_glyph_fitting`
selects a fitting for the process, and each glyph atlas reads it once when its
canvas is created, so one atlas never holds bitmaps of two fittings. The explorer
selects `Fitted` before its first window and keeps the other two reachable
([how and why](../docs/verification/2026-11-18-ui-sharpness.md)); tests and
examples that create windows without it keep `Subpixel`.

- `Snapped` gives every glyph one bitmap, rasterized at offset zero and drawn at
  the rounded layout position instead of at one of eleven tenth-pixel phases, and
  applies a mild contrast curve to its coverage.
- `Fitted` also fits outlines horizontally. Upstream renders through swash, whose
  fixed hinting mode moves outlines vertically only, so a 1.1 px stem covers two
  gray columns. The fitted path asks skrifa, the crate swash itself hints with,
  for full hinting instead: the font's own TrueType program when that puts the
  stem of `l` on whole pixels, otherwise the automatic hinter with stem adjustment
  plus the contrast curve. zeno, swash's rasterizer, draws the result. Because a
  fitted outline is drawn for a whole-pixel advance, glyphs inside a word follow
  each other at their fitted advances. At small sizes those are often wider than
  the layout's, so a glyph moves a pixel back where its pen would lead the layout
  by more than half a pixel or its fitted advance would end more than half a
  pixel past where layout starts the next glyph (read ahead in the run; a mark
  that layout puts over the glyph does not count), and a pixel forward where
  the pen would lag by more than one. A word starts on the rounded layout
  position after a blank, on
  another baseline and after a glyph that was not fitted; after a blank it starts
  no earlier than keeps that blank at least its layout width less half a pixel.
  Until 20 November 2026 the pen only had to stay within one pixel of the layout
  either way, so words grew into the blank after them: on About dialog text laid
  out from every tenth-pixel start, 1,708 of 5,580 blanks (DejaVu Sans at 11 to
  14 px, its bold at 20 and 26 px, Noto Sans at 11 to 13 px) were drawn more than
  half a pixel narrow, at worst 2.5 px narrow (a 3.5 px blank drawn as 1 px).
  With the current rule none is; the same samples have 1,085 instead of 962 of
  27,450 neighboring glyph pairs whose bitmaps overlap, most of the added ones in
  Noto Sans at 11 and 12 px, whose automatically hinted advances run wide.

Stroked text, glyphs drawn as paths (rotated, large or non-solid text), color
glyphs and every font with color glyph tables keep the upstream path in all three
fittings. Text layout, measurement, carets and selections are not part of this
crate and keep using linear advances, so with `Fitted` ink can sit a pixel, and
at the start of a word after a blank about a pixel and a half, from its layout
position. A word is followed from the first glyph of a
`fill_glyph_run` call. Slint 1.18.1 leaves out the glyphs of unelided text that
lie more than a line height outside the clip, so a word that crosses the left
clip edge, as in a scrolled line edit, can be placed one pixel differently from
one scroll position to the next.

The fitting is in `src/text/fitting.rs`, its use in `src/text.rs`
(`render_atlas`, `render_glyph`, and `upload_glyph`, which is the former tail of
`render_glyph_swash`) and its export in `src/lib.rs`. It makes `skrifa` 0.44 a
direct optional dependency enabled by the existing `swash` feature, with default
features off and `std`; swash 0.2.10 already depends on that version, so no
package is added to the application or to the standalone lockfile. Run its tests
from the repository root:

```bash
cargo test --manifest-path vendor/femtovg/Cargo.toml --lib --no-default-features --features swash --locked --target-dir _build text::fitting
```

The pen and contrast tests need nothing else. Three tests read DejaVu Sans from
`/usr/share/fonts/truetype/dejavu`, because the vendored tree ships no font, and
are ignored by default; add `-- --include-ignored` on a machine that has it. Two
check the fitted rasterizer; the third lays About dialog text out at 11 to 14 px
from every tenth-pixel start and checks that each blank keeps at least its layout
width less half a pixel.

The WGPU source patch is in `src/renderer/wgpu.rs`, `wgpu/bindings.rs`,
`wgpu/uploads.rs` and `wgpu/viewports.rs`, with its tests in `wgpu/*_tests.rs` and
their shared validated test device in `wgpu/test_gpu.rs`. The added code remains under the upstream
MIT OR Apache-2.0
license choice; application packaging continues to select MIT.
Upstream tests remain alongside the source. The packaged book, GitHub workflows
and Cargo extraction marker are omitted; they are not needed to build the library
or run its tests. Apart from the `skrifa` dependency the published manifest is
unchanged and still declares three
`[[example]]` targets under the omitted `book/`, so build or test the vendored
package with `--lib`, `--tests` or `--test <name>`; `--all-targets`, `--examples`
and an unfiltered `cargo test` fail to find those example sources.
The standalone lockfile starts from the published lock. Its Windows API family
is unified at `windows` 0.62.2 to match WGPU 30 and gpu-allocator; the original
0.58 selection did not compile their DX12 interface types together. This affects
only the independent test graph; the application retains its existing versions.
The lock's `femtovg` entry also names `skrifa 0.44.0`, which it already held for
swash.

Run the added GPU regressions from the repository root (the explicit tests require
a real adapter and fail if none is available):

```powershell
$env:WGPU_BACKEND = 'dx12' # Use vulkan on a corresponding Linux GPU host.
cargo test --manifest-path vendor/femtovg/Cargo.toml --lib `
  --no-default-features --features wgpu --locked --target-dir _build `
  renderer::wgpu:: -- --ignored --test-threads=1
```

All of them run with the backend validation layer (`InstanceFlags::VALIDATION`).
They check actual output pixels, unchanged-frame reuse, pixel updates, same-id
texture replacement, deletion (including deletion before a flush), uniform-buffer
growth, eviction between frames, frames with more bindings than the cap, and
pruning. Upload cases additionally cover GPU-completion reuse through `begin`'s
own nonblocking poll, unsubmitted-buffer isolation, discarded submissions,
renderer drop, growth, cache bounds, ordered mixed cached/uncached deferred
commands, and oversized uploads. Viewport cases verify repeat reuse, exact-bit key
separation, bounded eviction, and correct GPU reads after eviction/cache drop with
deferred command submission.
Upstream font tests requiring the excluded example assets need the upstream
checkout; they are not claimed as part of this focused GPU validation.

Performance and native image invalidation checks are documented in
[the scrolling investigation](../docs/NATIVE_SCROLL_VERIFICATION.md). This patch
is retained only with functional GPU checks and a measured native improvement.
