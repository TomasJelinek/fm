# Exact glyph compositing performance implementation plan

> **For agentic workers:** Use superpowers:executing-plans for this scoped implementation.

**Goal:** Reduce measured high-DPI text/source/Markdown raster refill work while preserving every output pixel.

**Architecture:** Attribute existing glyph paint time to mask preparation and composition first. Only if composition is material, specialize the existing monochrome mask loop with clipped contiguous rows and exact transparent/opaque fast paths. Keep partial-alpha arithmetic and all color/bitmap rendering unchanged.

**Tech Stack:** Existing Rust, Swash/Parley and renderer test/probe infrastructure; no dependencies.

**Spec:** The coordinator's approved scope and `docs/verification/2026-11-11-viewer-scrolling.md`; retained raster/paging contracts in `docs/developer/SCROLLING.md`.

## Constraints and ownership

- Own `crates/midnight-viewer-render/src/glyphs.rs`, private timing/oracle modules, and `raster.rs` only if sharing its pixel blend avoids duplication.
- No public API, source-read, layout, phase, source-anchor, glyph-cache or retained-raster budget changes.
- Preserve color outlines, bitmap fallback, premultiplied RGBA and exact integer rounding.
- Authoritative checkout is `D:/experiments/dev/FM`; generated output under `_build`.
- Coordinate the shared Cargo target and quiet timing slots. No concurrent build during measurements and no commit.

## Review focus

Transparent and opaque masks over nonopaque backgrounds must remain exact. Negative bearings and clipping must preserve the same pixels. Partial brush opacity must preserve both rounding steps. Color/bitmap sources must keep their existing contract. A faster synthetic inner loop must also improve the actual bounded viewer workload before being retained.

## 1. Establish component baseline

- [x] Add private opt-in, debug-only aggregate mask-generation/compositing timings with fixed labels and counters; no content/path output or per-pixel clocks.
- [x] Add an independent scalar reference oracle covering all 256 coverage values, varied brush alpha/color, clipping/negative bearings and color/bitmap fallback. Confirm the oracle passes on unchanged production rendering.
- [x] Build renderer tests and existing `seek_latency` with the pinned SDK after Cargo handoff; preserve the baseline executable.
- [x] Quiet actual-file traces at 100% and 225% separate mask generation, composition and other glyph work. Record the full sample definition; distinguish existing report statistics from recomputed retained logs.

## 2. Implement only a justified fast path

- [x] If composition is material, choose the Mask path once, clip source/target rows once, skip transparent source pixels and copy exact opaque pixels. Keep the original partial-alpha formula.
- [x] Run the complete pixel oracle and existing renderer framebuffer regressions. Independently review bounds, RGBA rounding and unchanged fallback behavior.
- [x] Run serial baseline/candidate backend measurements with per-glyph tracing disabled with identical profile/input/geometry; retain only a measured improvement.
- [x] Qualify native wheel, sustained refill and drag using current protected plugins and exact Markdown/BAT/JSON/XML files in D3D and software. Separate reused-pixel motion, replacement-pixel gaps and final content acknowledgment. No uniform-60Hz claim from medians alone.
- [x] Record exact commands, hashes, results and limitations in `docs/verification/2026-11-12-glyph-compositing.md`; run targeted strict Clippy, formatting and diff checks.

## Qualification disposition

Windows and Linux pixel suites each pass 14/14. Native qualification is 15/17 by
the original numeric gate; separate bounded pixel proof resolves BAT Text's two
failures as byte-density position-estimate changes, with the guard unchanged.
Visible glyph motion remains continuous across those rebases. A small scrollbar
thumb estimate discontinuity remains a documented follow-up. The diagnostic
example's final four tests, strict scoped Clippy and formatting pass. See the
verification note for exact numbers, hashes and preserved failures. No uniformly
smooth or native baseline/candidate speed claim is made.
