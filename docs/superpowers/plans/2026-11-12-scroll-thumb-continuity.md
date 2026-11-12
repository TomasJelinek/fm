# Bounded scrollbar continuity implementation plan

> **For agentic workers:** Use the executing-plans skill to implement this plan task by task.

**Goal:** Fix SCROLL-01: page replacement must not move the scrollbar opposite to continuous
pixel motion, while preserving reversal, absolute seeking, and real BOF/EOF.

**Architecture:** Replace the average bytes-per-page-height estimate with a monotone map from
shaped visual positions to existing source anchors. Map viewport top and bottom together to
compute both position and extent. Use the same map for the covered seek inverse. A new layout
rebuilds its bounded map; no history, source scanning, or extra provider requests are introduced.

**Tech Stack:** Existing Rust renderer, core contracts, viewer engine, and Slint host.

**Spec:** SCROLL-01 in docs/IMPLEMENTATION_BACKLOG.md and the BAT evidence in
`docs/verification/2026-11-12-glyph-compositing.md`.

## Constraints and ownership

- Authoritative repository D:/experiments/dev/FM, starting at 6f06ec4; no commits by this agent.
- Preserve exact pixel rendering, layout, source revisions, bounded reads, and plugin ABI.
- Root approved narrow internal renderer/core metadata changes; root owns lifecycle files.
- All generated output remains under `_build/`; coordinate Cargo and native quiet leases.
- Follow 100-column formatting and 100,000-byte handwritten source limits; no new dependencies.

## Model and files

- Renderer `scroll_anchors.rs`: enumerate visual line starts in logical source order; parallel
  table cells share one semantic row span. Coordinates come from the existing exact layout.
- Core `viewer_scroll_map.rs`: immutable bounded anchor map, interpolation of document fractions,
  coupled position/extent, and monotone inverse within an explicitly covered origin interval.
- `ViewerRasterInfo`: share the map with the UI using Arc; app-internal metadata only.
- Engine `scroll_map.rs` and worker position/raster integration: join rendered text offsets to
  already validated plugin source anchors, cache per layout, and preserve source offset endpoints.
- UI raster presentation/camera: calculate position and extent at the actual camera and use the
  same inverse for local seeks. Uncovered seeks keep the existing bounded backend path.
- Tests: renderer wrapped/table mapping, core inverse/edges, engine unequal-density continuation,
  UI reversal/refill seeking and context replacement. Probe adds actual thumb geometry evidence
  if existing position+extent tracing is insufficient.

## Review focus

- A replacement page with a different byte density but identical visible lines must keep both
  normalized position and physical thumb placement stable.
- Wrapped lines and centered uneven table cells must have monotone visual/source spans.
- Real endpoints and short files must reach 0/1 or hide the bar without invented content.
- Reversal and absolute seeks must use one mapping; no directional clamps or delayed correction.
- Width/font changes must rebuild mapping with layout; storage is bounded by decoded/rendered
  fragment rows, never file size or accumulated navigation history.

## Steps

- [x] Add an engine regression with unequal-length lines matching the actual BAT failure mechanism;
  compare frames at the same retained world origin before and after overlapping page replacement.
- [x] Run the focused regression RED and report its exact failure before production edits.
- [x] Add the bounded renderer/core map and its endpoint/inverse/wrap/table tests.
- [x] Integrate worker and UI mapping; run focused regression GREEN and direct boundary tests.
- [x] Independently review mapping invariants and source/pixel bounds.
- [x] Run the actual BAT wheel/reversal and absolute seek probes in both native renderers with a
  quiet machine; retain original failures and compare physical thumb plus spatial pixel evidence.
- [x] Record exact commands/results and remaining limitations; return source frozen to root.


## Outcome

Implemented and independently reviewed; focused RED/GREEN, engine/UI boundaries and final
combined Windows lint/tests are qualified. Native source/thumb direction is correct through
refills and reversal. The corrected valid 12-case matrix has 10 original-guard passes and 2 D3D
frame-count failures; the preserved original binary reproduces the large D3D delay. This is
not a universal smoothness claim. See the final verification document for all raw paths,
invalid first-batch seek cases, corrected probe latch, hashes and remaining pacing limits.
