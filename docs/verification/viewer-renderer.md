# Shared viewer renderer proof

Date: 2026-11-06. This records the renderer prerequisite for the viewer-plugin
stage, not a delivered F3 window or plugin loader.

`crates/midnight-viewer-render` is a host-owned, platform-neutral crate with
`unsafe_code = forbid`. Parley 0.11.1 supplies one no-wrap Unicode layout for
drawing, hit testing and selection. Swash 0.2.10 rasterizes those exact glyphs
into premultiplied RGBA. Plugins will provide semantic text and source anchors;
they do not link the renderer, Slint or the application.

## Correctness and limits

Thirteen tests pass on Windows and Linux, including Unicode/RTL selection,
combining text, emoji, semantic colors, DPI/font settings, viewport cropping,
premultiplied pixels, bounded caches and malformed geometry/ranges. Strict
all-target Clippy passes on both platforms. Commands:

```text
cargo test -p midnight-viewer-render --locked
cargo clippy -p midnight-viewer-render --all-targets --locked -- -D warnings
cargo run -p midnight-viewer-render --release --example render_probe -- _build/renderer.bmp
```

Parley treats CR and LF as separate mandatory breaks. The adapter shapes CRLF
as LF through a sparse source map while keeping the exact original text for
clipboard copy. Tabs expand at configurable Unicode display-column stops;
combining sequences and joined emoji use grapheme/display widths. This defines
source-code tab columns, not pixel-aligned stops for proportional fonts. Mouse
positions inside expanded tabs resolve to original boundaries.

Per-page input and expanded text are limited to 256 KiB, 4096 hard lines, 4096
style ranges and 16384 layout replacements. Font size is capped at 256 physical
pixels; the family description is bounded independently of filesystem paths.
A raster is at most 16 million pixels (64 MiB RGBA). Glyph image cache data is
at most 8 MiB and 4096 entries, with at most 64 variation coordinates per key.
These bounds cover our buffers, not an OS/system-font process memory guarantee.
Font library caches and aggregate session budgets need host-level accounting.

Only installed system fonts are used in normal operation. Windows visual proof
shows Czech, CJK, Arabic/Hebrew, combining marks and color emoji. The minimal WSL
installation lacked CJK/emoji fonts; the initial image correctly exposed that
environment limitation. A second run with QA-only
[Noto CJK](https://github.com/notofonts/noto-cjk) and
[Noto Emoji](https://github.com/googlefonts/noto-emoji) fonts verified Linux
fallback and bitmap color emoji. No system installation or font redistribution
was performed. This qualifies the tested glyph/font formats, not every color
font technology or every Unicode character on machines without matching fonts.

## Performance evidence

1280 x 720 raster, ten successive viewport offsets, same workstation:

| Windows build/probe | 4096-line page, 10 frames | 256 KiB single line, 10 frames |
| --- | ---: | ---: |
| Debug, before glyph cache | 6318 ms | 22522 ms |
| Debug, bounded glyph cache | 1262 ms | 2282 ms |
| Release, bounded glyph cache | 79 ms | 207 ms |

Release page layout took 16 ms and 28 ms respectively. Cold system-font setup
varied between runs; the recorded release run took 572 ms for the first small
page. Layout, rasterization, font discovery and large cleanup must therefore
stay on workers. These are renderer timings, not end-to-end UI latency claims.
Linux debug recorded 1115 ms and 2014 ms for the same ten-frame probes; no Linux
release renderer timing is claimed here.

The initial Linux probe spent minutes writing the evidence BMP in four-byte
writes through the WSL/Windows filesystem bridge (`p9_client_rpc`). That time
was outside the measured rendering loops. The example now uses `BufWriter` and
explicit flush; it does not change application rendering behavior.

Local evidence remains under `_build/qa/viewer-plugins/`: `renderer-final-tests.log`,
`windows-clippy.log`, `linux-renderer-final.log`, `render-probe-before.log`,
`render-probe-cached.log`, `render-probe-release.log`, `renderer-cached.bmp`,
`renderer-linux.bmp`, `renderer-linux-noto.bmp`, and pinned font source/hash records.

## Host integration still required

The image must expose a semantic accessible text region in the Slint host;
an image-only viewer is insufficient. Slint's public accessible text role/value
can expose bounded visible text and named navigation/search controls. Rich
screen-reader caret/selection behavior is tied to Slint's own TextInput bridge
and must not be claimed for this raster surface without separate integration.
Do not overlay a differently shaped TextInput for mouse selection. Enabling and
testing the host accessibility backend belongs to the window integration step.

Remaining work includes current-generation viewport scheduling, cancellation,
aggregate memory accounting, host window/accessibility tests and actual plugin
discovery/content/ABI lifecycle. The public API is still an implementation draft.
