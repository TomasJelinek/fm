# JSON Source Scrolling Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans inline, with actual
> failing regressions before implementation. Preserve unrelated dirty work and
> scoped before-images; no commits, dependencies or plugin ABI changes.

**Goal:** Keep JSON/source pixels and line numbers stable through forward/backward
scrolling, bottom-to-top traversal and scrollbar dragging.

**Architecture:** Preserve screen-space glyph raster phase independently of the
page-local layout origin. Preserve exact known source coordinates across overlapping
pages, including backward gutter numbering; request asynchronous Locate only for
unknown coordinates. Keep decoding/highlighting and independent source-plugin
architecture unchanged unless an actual plugin defect is separately reproduced.

**Tech stack:** Existing Rust viewer renderer, viewer engine, public source plugin
SDK and read-only installed-plugin probe. No new dependency or Slint changes.

## Evidence and scope

The exact read-only user file is
`D:\temp\ovclash\results\2026-09-07-mfg\manifest.json`, 2659 bytes,
SHA256 `CA5E4B26FEBA253568C17A62869471A2AEB2646D194593F18D2F9611263D21E7`.
Its identical captured copy is `_build/qa/json-scroll/fixtures/user-manifest.json`.

JSON routes to the independent `midnight.viewer.source` lexical profile, not
Markdown or an editor. An actual installed Linux plugin/production-worker probe
fails its first 72-pixel scroll with 1.539% changed overlap pixels: line numbers
vanish while Locate runs. Waiting for Locate still changes 1.184% of pixels.
The glyph cache/raster offset currently uses page-local fractional positions,
then applies the viewport origin after flooring, so page rebasing changes glyph
raster phase. The Markdown fixes preserved semantic context and border phase;
they did not exercise fractional-height source-line rebasing.

## Task 1: causal renderer correction

Files: `midnight-viewer-render/src/glyphs.rs`, possibly shared raster coordinate
normalization in `raster.rs`, new public renderer `tests/source_continuity.rs`.

- [x] Add translation-invariance regression comparing a complete source layout
  with its overlapping suffix restored through a fractional caret offset.
- [x] Confirm the actual pixel failure at normal and scaled font geometry.
- [x] Derive glyph raster/cache phase from final screen coordinates, preserving
  meaningful fractional scrolling and existing glyph/font/color/cache budgets.
- [x] Run complete renderer regressions, including Markdown border/heading cases.

## Task 2: immediate source-coordinate continuity

Files: `midnight-viewer-engine/src/worker_text.rs`, narrow `worker.rs` call site,
new `worker_source_continuity_tests.rs` registered in existing worker tests.

- [x] Add regression requiring exact line numbering on the first overlapping
  forward page and backward page without an intervening Locate reply.
- [x] Confirm missing coordinates before the correction.
- [x] Transfer a known scalar source coordinate through exact shared anchors.
  Count lines backward as well as forward; never fabricate a first-fragment
  column when its preceding logical-line prefix is unknown.
- [x] Keep asynchronous Locate available for unknown/distant positions. Reject
  stale replies using existing request/page identity checks.
- [x] Run full viewer-engine regression suite.

The narrow repeated quarter-position probe additionally proved that known global
lines must survive when a backward fragment's first column is unknown. The
internal coordinate now carries `Option<u64>` for that column. A newline restores
column one; missing prefix context keeps None. Gutter labels use known line
authority independently. Optional Locate completion is suppressed only for its
current page; a later nearby page resets a declined lookup and can recover.

Small-step actual-file runs proved a separate incomplete final-row defect.
Plain text/source/binary pages reserve that uncertain visual line only when a
complete viewport already fits. Semantic document windows keep their existing
context behavior. Short unwrapped fragments paint immediately and advance via
their captured terminal/start anchors, without waiting for a whole logical line.

## Task 3: actual-file qualification

File: new dependency-free `midnight-viewer-engine/examples/source_scroll.rs`.
All generated output belongs to `_build/qa/json-scroll/`.

- [x] Add read-only production-worker probe comparing overlapping pixels, two
  bottom/up rounds and repeated absolute scrollbar positions with real intent
  generation/sequence acknowledgments. PPM captures need no new image dependency.
- [x] Classify first-frame versus settled mismatches with diagnostic-only
  `JSON_SCROLL_SETTLE`; final acceptance uses first frames.
- [x] Rebuild the probe and run exact user file at several step/viewport sizes.
  Require stable highlighted pixels and identical return-to-top captures.
- [x] Check repeated scrollbar drag positions on the exact reported manifest.
- [ ] Check a longer nested JSON fixture if needed for the separate large-file
  follow-up; current native exact-file qualification is not replaced by synthesis.
- [x] Run final Windows viewer/renderer gates and protected installed runtime
  probe with the actual user file, then rebuild/deploy the debug application.
  The coordinator owns Windows Cargo, registration and final artifact hash.

Linux final session4536 passes engine116 tests (four installed-package opt-ins
ignored) and renderer41 tests across all targets. Actual first-moving-frame
72/17px and narrow43px runs all have zero changed overlap, exact repeated drag
positions, and exact quarter-pixel return images. Detailed timing/evidence is in
`docs/verification/2026-11-10-json-source-scroll.md`.

The JSON rendering correction is entirely host-owned. The subsequent 10GB
visible-page requirement separately bounds the Source plugin's optional Locate
to 256KiB from an existing checkpoint; an expensive unknown position returns no
coordinate, without disabling cheap recovery on later pages. The plugin owner
qualifies/repackages that change, and the coordinator owns final Windows gates
and application deployment. No full-file position index or fake numbers are added.
