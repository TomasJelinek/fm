# Text viewer opening and scrolling, 11 November 2026

This pass addresses the reported opening jump in `ClashApiCommon.cpp` and choppy
Markdown/JSON scrolling. The supplied C++ (12,033 bytes), Markdown (5,692 bytes)
and JSON (2,659 bytes) files were opened read-only. Their contents, application
preferences and plugin ABI were not modified.

## Causes and changes

The first source/Markdown frame could be mapped to the live canvas immediately
before an additional toolbar reduced its height. That squeezed existing glyphs
until the replacement frame arrived. A native child can also be created after the
call to `show`, so opening from an existing high-DPI window could initially use a
scale of one. Captured parent DPI now precedes child sizing. Each accepted raster
retains its own logical viewport: canvas changes clip it without changing the
body/gutter scale or selection coordinates.

A second bottleneck was conversion of backend RGBA to Slint pixel buffers on the
UI thread. A local debug microprobe measured about 15.5 ms for 2250x1400 pixels.
A bounded presentation worker now performs that conversion. Metadata-only frames
reuse safe shared buffers; native image creation remains on the UI thread. One
prepared delivery waits for consumption, backed by the existing coalescing
mailbox. Rejected replies retain the displayed buffer, opening success waits for
an admitted frame, and cancellation does not join a worker on the UI thread.

Correct initial DPI exposed a separate high-DPI paging weakness. A fixed
256-physical-pixel guard gave less scrolling time to prepare the next fragment
on a scaled monitor. The guard now represents 256 logical pixels, subject to the
unchanged 24 MiB retained-pixel limit. Source requests remain bounded by 12 KiB
of output and 512 lines; plugins retain their existing context-read limits.

Absolute seeks into an unloaded fragment also used an estimated byte density.
When the new layout changed that estimate, the final position could change from
0.7 to 0.650704 (Markdown) or 0.706477 (C++). The engine retains the desired
position and resolves it against the actual loaded layout before acknowledging
it. A bounded prefix and at most one correction read can supply missing context.
A newer target supersedes old refinement. Failure cannot falsely acknowledge a
position that the displayed content did not reach.
Salamander's `src/viewer3.cpp`, `OnVScroll` and `SB_THUMBTRACK`/
`SB_THUMBPOSITION` handling were checked again: they keep bounded context around
the seek, coalesce tracking updates and apply the final release immediately.
These are behavioral references; the Rust implementation uses its asynchronous
request/acknowledgement pipeline and does not copy the C++ implementation.

## Regression evidence

The source-opening marker/selection regression failed before the fixed-frame
mapping. Captured-parent placement failed at 125% DPI before scale initialization.
The extracted production guard failed the 225% logical-distance regression:
256 physical pixels instead of the required 576.

Focused checks passed:

- Pixel preparation: 7 tests, including backpressure, buffer reuse, bad input,
  failed notification and cancellation.
- Opening/geometry, retained raster, image and delivery checks: 87 distinct UI
  tests across the focused filters; overlapping filters are not added together.
- Captured placement: 3 tests at several display scales and later DPI changes.
- Final engine position cases: 11 tests, including changed density, bounded
  correction, failure without acknowledgement, deferred targets and short pages.
- Retained-raster follow-up: 37 engine and 20 UI tests; position acknowledgements:
  3 engine tests. These overlap the preceding focused groups.
- Existing configurable selection/view/menu bindings and all bottom-button
  modifier combinations were audited. Fifteen focused function-bar checks passed.

Logs and generated fixtures belong under `_build/qa/text-scroll-20261111/`.
Full workspace suites were not rerun during this targeted implementation pass.

## Native method and qualification

The debug `performance_probe` example runs beside protected viewer packages and
uses the supplied files without accessing application preferences. Runs are
sequential, without builds, deployment or synthetic CPU load at the same time.
The Windows window is 2250x1575 physical pixels at 225% display scaling.
`--native-parent` captures placement from a realized native parent, matching the
application launch path. GPU mode uses FemtoVG/WGPU D3D12; `--software` explicitly
exercises the CPU renderer.

```text
performance_probe <qa> <report> --d3d --native-parent --wheel --sustained --wheel-trace --file=<file>
performance_probe <qa> <report> --d3d --native-parent --scrollbar --wheel-trace --file=<file>
performance_probe <qa> <report> --d3d --native-parent --opening-proof --file=<file>
```

The observer excludes parent-window redraws. Opening captures latch the viewer's
publication, fixed frame geometry and window/layout inputs at a redraw. Only the
live canvas may resolve during that draw; content, scale, size, font and toolbar
changes are rejected. Two focused latch tests cover this distinction. Slint renders
that matching state to an offscreen snapshot; this is not compositor framebuffer
readback and does not observe Slint's direct pre-show draw. Pixel/selection unit
regressions complement this native geometry evidence. Snapshot runs are excluded
from cadence timing.

For scrolling, measure gaps between actual moving redraws, separately for reused
pixels and transitions receiving new pixels. Endpoint idle time is excluded.
Scrollbar thumb response is separate from content response and final completion;
the probe requires visible content to change during the drag.

## Final native cadence

All final timing runs use captured-parent startup. The table shows milliseconds
between moving redraws; uploads mean a new retained pixel allocation was admitted.

| D3D12 wheel scrolling | Reused pixels p95 | Reused pixels maximum | New pixels p95 | New pixels maximum |
| --- | ---: | ---: | ---: | ---: |
| C++ source | 17.181 | 18.161 | 24.930 | 34.680 |
| Markdown | 17.562 | 20.471 | 27.972 | 33.332 |
| JSON | 17.543 | 19.711 | 24.858 | 24.858 |
| Plain text, owned 600-line fixture | 17.199 | 19.403 | 23.746 | 25.713 |

Every final drag reaches the requested 0.7 position, with changing visible content
during the gesture. Final content is ready 77-125 ms after release in the main GPU
runs; the JSON repeat completes in 131.071 ms. The original
`final-native-json-drag.txt` reports 77.312 ms and
`final-native-json-drag-repeat.txt` reports 131.071 ms under
`_build/qa/text-scroll-20261111/`. Thumb responsiveness is measured separately
and is not substituted for these content measurements. JSON still has a 123.643 ms maximum content-update
gap during dragging; a repeat observed 92.406 ms. This outlier is retained in the
results rather than discarded. Source and Markdown maximum drag gaps were
52.870 and 43.650 ms, respectively.

Software Markdown/source wheel and drag checks also pass, including exact final
positions. At this high-DPI debug configuration, reused-pixel p95 gaps are
40.835/42.137 ms and new-pixel p95 gaps are 37.633/46.859 ms. Final content arrives
152-163 ms after release. The CPU renderer remains functional; these numbers do
not establish uniform 60 Hz on software or every document.

The native diagnosis distinguished initial page coverage from settled geometry.
With the same standalone startup, moving pixel conversion off the UI reduced
Markdown's new-pixel p95 gap from 53.584 to 25.644 ms. Correct captured-parent DPI
then exposed the too-short physical guard and approximate absolute-seek bug.
That intermediate captured-parent run had a 184.919 ms p95 gap; the logical guard
and exact seek corrections produce the final 27.972 ms above. Standalone and
captured-parent measurements must not be treated as an isolated worker comparison.

The previously supplied 607x204 PNG remains 607x204 from its observed opening
through settling. Twelve zoom steps to 381.46% and back to 100% also pass, using
the same source pixels. This snapshot run is excluded from timing comparisons;
it supplies geometry evidence, not an image-opening latency claim.

Reports: `final-native-*`, `final-software-*`, the plain-text reports, JSON repeat
and image proof under the QA directory above.

## Final opening and application checks

The revised opening proof passes for C++, Markdown and JSON. Publication 1 and
publication 2 have identical observed glyph-row positions and heights: eight
bands for the C++ crop, five for Markdown and thirteen for JSON. The first frame
retains a 972x631 logical viewport while the toolbar resolves the live canvas to
972x595. The replacement frame uses 972x595 without moving or resizing those
glyph bands. Native and Slint display scales both remain 2.25.

The immutable-state latch passes on both captures. Earlier diagnostic opening
runs deliberately failed because the first latch incorrectly treated in-draw
canvas layout as superseded content; those logs remain separate. The corrected
reports are `final-proof-{cpp,md,json}-opening.txt` and their `.opening` crops.
The source crops and full application startup snapshot were visually inspected.
This evidence retains the offscreen-rerender scope described above, rather than
claiming direct compositor first-paint measurement.

The final debug application and probe build passes. The application executable is
`_build/debug/me.exe`, SHA-256
`b529b0c260dff3b83bae43faa3269e10d4cced789ca408a4cdfcc1c8ce90c8b4`.
Its matching protected plugin runtime is deployed and registered, including
filesystem and thumbnailer packages. The startup smoke exits 0, creates its
snapshot and writes zero bytes to stderr.

The timing probe's SHA-256 is
`acf61f456832e9a912d9f5c8d79a48ed0f298cc230f470bf87e3b7a0881b49fa`.
The final opening-only probe, with corrected instrumentation and unchanged
production viewer code, is
`b3dd0fa500feacc60c3916e8a4362c7a677657fc9c92247eadd6abfc7a74242b`.

`cargo fmt --all --check`, `git diff --check` and the source-size check pass:
1,893 handwritten source files, all at or below 100,000 bytes. Generated probes,
fixtures, snapshots, logs, toolchains and executables are excluded from Git.
## Later bounded-refill qualification

A later isolated change starts a retained-raster refill at the same logical
lookahead on scaled displays, capped at half the available guard. It does not
increase the raster allocation budget or load the complete source. Two alternating
baseline/candidate runs used the supplied JSON, native parent placement, 2.25 DPI
scale, a 2250x1575 window and FemtoVG/WGPU DX12 with no concurrent builds.

| Run | Input-to-redraw p95 | Content-motion longest gap | Final content after release |
| --- | ---: | ---: | ---: |
| Baseline 1 | 24.732 ms | 117.253 ms | 122.283 ms |
| Candidate 1 | 23.851 ms | 78.999 ms | 124.171 ms |
| Baseline 2 | 17.363 ms | 92.523 ms | 121.343 ms |
| Candidate 2 | 18.216 ms | 77.588 ms | 118.498 ms |

The largest content-motion gap improved in both pairs (about 33% and 16%).
Typical frame latency and final seek latency were similar; these measurements do
not establish a uniformly faster frame rate or uninterrupted phone-like motion.
The bounded lead change is retained for the reduced refill stalls. Each run
reaches position 0.70000005 with changed, nonblank content during dragging.
The crop check is a CPU rerender, not GPU compositor readback.

The software-renderer JSON run also passes: p95 redraw 20.808 ms, maximum content
gap 69.889 ms and final content 95.373 ms. Native GPU wheel runs pass for the exact
JSON, Markdown and C++ files, with 13-14 rendered positions per notch. Their
measured largest gaps still range up to 80.063 ms; further smoothing remains a
performance task, not a claim made by this patch.

Logs are `_build/qa/json-refill-20261111/{baseline,candidate}*-json-drag.txt`
and `candidate-{json-software,json-wheel,md-wheel,source-wheel}.txt`.
Candidate probe SHA-256:
`a4f5e6f674aaf220bb1d21d27df1b376626170ef49842df04039708c4c92acdc`.
Its independently built viewer packages were deployed to a hash-matched protected
runtime before measurement. An initial attempt without that registration was
stopped before first content and is excluded from the timing comparison.
