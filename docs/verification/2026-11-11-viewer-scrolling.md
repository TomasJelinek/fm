# Viewer scrolling and batch highlighting — 2026-11-11

## Reproduction and cause

The native Windows probe used the supplied files:

- `D:/dev/ov-physics-trunk/ovclash/AGENTS.md` (213,103 bytes).
- `D:/dev/ov-physics-trunk/ovclash/build.bat` (22,632 bytes).

The user geometry was reproduced at 1,340 × 820 logical / 3,015 × 1,845 physical
pixels, 225% scaling, Segoe UI 13. Viewer typography retained its defaults.
Native wheel messages were sent after clicking the text canvas; the scrollbar
probe dispatches real Slint pointer press, move and release events.

Seeking to 10% enters a large Markdown table. In the baseline, forward wheel
motion stopped for 2.249 seconds at position 0.125773430, despite continued input
and redraws. Rendering fell back from retained raster to legacy viewports and
horizontal extent changed from 1.0 to 0.963030159. Reversing direction resumed
motion. Opening the file at its beginning did not reach this table during the
same gesture, which explains why initial scrolling alone missed the defect.

Two causes interacted:

1. The plugin cropped an oversized table to scalar pages while retaining partial
   table metadata. Measuring different fragments changed column widths.
2. The renderer combined source ranges from every cell in a visual row, including
   cells whose content did not cross the requested Y coordinate. The clamped last
   line of a short first cell pinned the forward continuation anchor while a tall
   later cell continued. Vertically centered GitHub cells require the same check.

## Behavior and bounds

Document requests now permit a semantic unit up to the existing 64 KiB read cap.
Complete tables that fit the caller and public wire budgets retain all measuring
rows. Tables exceeding the raw, byte, exact-anchor or wire-item budgets use
bounded wrapped row text without partial table geometry. Scalar fallback remains
8 KiB. Raw-truncated tables do not claim complete layout metadata. No source
anchors are interpolated or discarded to squeeze a table through the protocol.
The existing parser, cache, cancellation and no-whole-file-reading bounds remain.
The supplied large table consequently uses readable text rather than its rich
column layout; smaller tables retain formatting.

Visual-line continuation uses only cells crossing that line, preserving progress
through taller cells in either column order and both scroll directions.

BAT and CMD join the source plugin's extension routing. The lexer highlights
case-insensitive batch commands, command-position `REM`/`::` comments, numbers
and double-quoted strings, with batch backslash/quote rules. Argument text such
as `echo rem` remains ordinary text. There are no new dependencies.

## Targeted verification

All commands ran in `D:/experiments/dev/FM`, with output under `_build/` and the
repository Windows toolchain environment. Cargo used `--locked --offline`.

- Markdown independent plugin: all 61 tests passed, including five new large-table
  cases covering rich layout above 8 KiB, smaller caller budgets, public anchor
  exhaustion, >64 KiB units, bidirectional progress, distant backward anchors and bounded reads.
- Final renderer boundary suite: all 29 tests passed across library tests, raster,
  document GitHub/table layout, heading pixels and table metrics. This includes
  short-first/tall-later continuation, centered-cell exact-bottom coverage and
  the full RGBA oracle for the paint optimization.
- Source independent plugin: all 32 tests passed, including two BAT/CMD regressions.
- Both independent plugins passed `cargo clippy --all-targets -- -D warnings` and
  their release package helpers built successfully against the public SDK bundle.
- `seek_latency` built successfully; it exercises the actual engine with 60 absolute
  position requests at 10 ms intervals, retires frames, and requires the final ack.

Representative commands (SDK configs are generated local paths):

```powershell
. _build/qa/navigation-ordering/windows-env.ps1
cargo --config _build/viewer-source-regressions/sdk.config.toml test `
  --manifest-path plugins/midnight-viewer-markdown/Cargo.toml `
  --target-dir _build/viewer-markdown-regressions/target --locked --offline --all-targets
cargo --config _build/viewer-source-regressions/sdk.config.toml test `
  --manifest-path plugins/midnight-viewer-source/Cargo.toml `
  --target-dir _build/viewer-source-regressions/target --locked --offline --all-targets
cargo --config $workerSdk test -p midnight-viewer-render --test document_tables `
  --target-dir _build --locked --offline
```

Clippy requires an absolute SDK config path because its child Cargo invocation
can resolve relative paths differently. The worker SDK and public plugin SDK are
separate bundles; the worker bundle does not contain the plugin SDK crate.

## Native evidence

Logs are under `_build/qa/viewer-regressions-20261111/`.
The first candidate runs used rebuilt plugins with the preserved pre-final probe
executable while the combined application compiled. They establish plugin behavior;
performance numbers from that concurrent-build run are provisional.

- `markdown-large-table.*`: failing baseline, including the 2.249-second stall.
- `markdown-candidate-table.*`: 614 moving frames through the same table, all retained
  raster, forward to 0.173722535 and back to 0.099956669. No legacy transition;
  horizontal extent remained 1.0. Within-upload p95 gap 17.308 ms, refill p95
  25.345 ms, maximum observed gap 50.126 ms.
- `markdown-candidate-drag.*`: all 60 pointer moves accepted, final content 0.7,
  changed nonblank pixel crop, horizontal extent 1.0 throughout. The 15 content
  updates during the rapid whole-file drag still need separate timing attribution.
- `batch-candidate-source.*`: the exact batch file selected Source mode, with 607
  moving retained frames through a midpoint forward/reverse gesture.
- `batch-baseline-text.*`: original basic Text routing also scrolled the exact file
  at the reproduced geometry/focus, with 603 moving retained frames. The reported
  Text-mode stall has not been reproduced in this configuration.
- `seek-markdown.txt`: backend-only 60-target run produced 22 frames; final target
  acknowledged after 87 ms. This excludes native upload/presentation and motivates
  the debug-only request/layout/paint attribution rather than assuming a UI delay.

Probe controls added for reproducibility: `MIDNIGHT_VIEWER_PROBE_SIZE`,
`MIDNIGHT_VIEWER_PROBE_UI_FAMILY`, `MIDNIGHT_VIEWER_PROBE_UI_SIZE`,
`MIDNIGHT_VIEWER_PROBE_START`, `MIDNIGHT_VIEWER_PROBE_WHEEL_MS`,
`MIDNIGHT_VIEWER_PROBE_FOCUS_CLICK`, and `MIDNIGHT_VIEWER_PROBE_MODE_INDEX`.
For the source plugin, mode index 0 is Source and 1 is Text.
`MIDNIGHT_VIEWER_SCROLL_TRACE` enables debug-only request/layout/paint timestamps;
`MIDNIGHT_VIEWER_SEEK_SCALE` compares identical logical viewports at different DPI.

## High-DPI paint attribution and exact-pixel optimization

The debug backend trace separated layout from paint. At 225% scaling, median
layout was 1.760 ms and paint 34.793 ms; at the same logical size with 100%
scaling, layout was 1.595 ms and paint 7.640 ms. Initial read/plugin/decode took
about 9 ms; subsequent requests could queue behind paint. These runs overlapped
other compilation and are attribution evidence, not final user-facing budgets.

A standalone harness includes the production `raster.rs`, with glyph painting
stubbed only to isolate fills. At 2,826 × 1,855 pixels, the original clear took
3.409 ms, a large rounded background 61.111 ms, and inline-code backgrounds
25.966 ms. The rounded fill evaluated `hypot` and blended identical interior
coverage separately for every pixel.

The narrow optimization batches each row's constant-coverage middle, retains the
original scalar corner calculation, and copies opaque spans directly. The same
harness then measured 3.331 / 5.563 / 4.378 ms. All three complete-buffer hashes
were identical before and after. A full RGBA differential test against the prior
scalar coverage/compositing covers clipped, fractional, thin and offscreen
rectangles, multiple radii, and alpha values 0, 1, 77, 128, 254 and 255. It passes
in the standalone Rust test harness. Two independent read-only reviews found no
remaining span, coverage or premultiplied-alpha issues.

Evidence: `raster-hotspot-before.txt`, `raster-hotspot-after.txt`,
`raster-pixel-standalone.log`, and `seek-markdown-phases-{225,100}.txt` in the
same QA directory. The full targeted renderer suite subsequently passed 29/29.
Final actual-file checks follow the combined build; the standalone gain must not
be presented as a whole-viewer speedup.

## Final actual-file verification

The rebuilt probe (`BA1C17F2A58661BF` SHA-256 prefix) and current plugin packages
ran after the shared UI test/native slots were released. All four native reports
contain completed gesture summaries; exit status alone was not treated as proof.

- `markdown-final-wheel.*`: Markdown mode, 618 moving frames, all retained raster.
  The failing 10% table region scrolled to 0.173764303 and returned to 0.100001417.
  Within-texture p95 gap was 17.248 ms, refill p95 26.139 ms, maximum 26.894 ms.
- `markdown-final-drag.*`: 60 pointer moves, 57 rendered thumb positions and final
  content position 0.7. Content updated 25 times during dragging (15 in the prior
  run); median content gap 33.486 ms, final content settled after 119.747 ms.
  This is an improvement, not a claim of 60 Hz whole-file seeking.
- Both Markdown wheel and drag traces kept horizontal extent exactly 1.0.
- `batch-final-text.*`: actual mode index 1, Source=false, Markdown=false; 602
  moving retained frames through the exact batch file, forward and backward.
- `batch-final-source.*`: actual Source mode index 0; 607 moving retained frames.
  Both batch runs used the same midpoint, focused canvas, geometry and DPI.
  The reported Text-mode stall remained unreproduced; both final modes scroll.

`seek-markdown-final-225.txt` confirms backend final-position acknowledgment in
59.084 ms. Median layout was 1.647 ms and paint 28.067 ms (prior phase run:
1.760 / 34.793 ms). Background fills now cost at most 1.528 ms in that run;
remaining glyph painting dominates at median 19.826 ms. The 100% comparison
acknowledged the final position in 29.246 ms. The prior runs overlapped builds,
so absolute before/after latency comparisons retain that limitation.

The existing `wrap_control` integration regression had listened to `seek` while
the actual scrollbar emits `drag-seek`; its thumb/capture assertion passed before
the missing-event unwrap failed. The test now observes `drag-seek` and checks the
vertical axis explicitly. This correction changes no production UI behavior.
