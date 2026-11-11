# Viewer pixel scrolling, 2026-11-11

The viewer input/continuation corrections are implemented. The final native run
confirms fractional pixel movement, with retained images near 60 Hz during motion.
High-DPI text/source rasters and idle restart still have the limits quantified below.
This record distinguishes backend raster completion from native redraw submission.

## Changes

- Native discrete wheel detents now use the shared fractional motion controller
  for plain text, source, binary, Markdown and untiled document rasters. The UI
  sends physical pixel deltas; file access, layout and painting remain workers.
- Native precise pixels and phased touchpad gestures keep their direct route.
  Keyboard navigation, scrollbar input, file/mode changes, presses and focus loss
  cancel the old wheel tail. Moving the pointer alone preserves deceleration;
  starting wheel input over a different target cancels it.
- The viewer composes the shared ancillary scrollbar observer in its existing
  Winit hook, covering popup lists without replacing image input handling.
  It observes native transitions before browser/image consumption so a consumed
  release cannot leave auxiliary scrolling stuck. Embedded HTML yields pointer,
  wheel, key and IME input to an active non-tooltip SDK popup while retaining the
  current cursor and authoritative modifier/shortcut-release/focus notifications.
- A text-page continuation accumulates all subsequent input, including reversal,
  and preserves the first diagonal gesture's horizontal pixels. Pending movement
  is applied before painting the replacement fragment, even before its terminal
  event. Existing bounded lazy page loading and caches remain in place.
- Input backpressure retains a refused delta. This prevents loss on a full queue;
  it is not a display acknowledgement and does not guarantee one paint per delta.

## Regression evidence

The continuation tests failed before their fixes (reversal displayed 3.5 pixels
instead of 2.25; crossing gesture lost 7.5 horizontal pixels). A stronger regression
also caught the wrong first replacement frame before the terminal event.

Keyboard cancellation failed with 24 movements instead of one; phased LineDelta
was incorrectly eased. Review additionally caught precise image residuals being
cancelled by a redraw while guard tiles were pending. All are covered by focused
regressions, including same-mode next-file navigation and switching wheel targets.

- Initial all-target viewer/engine gate: engine 118 passed, four existing installed
  opt-ins ignored; UI 46 passed; UI integrations 19 passed.
- Viewer library gate after cancellation review: 51 passed.
- Final popup/composition regressions first failed: Chromium consumed a pointer
  move over the real Viewer mode popup; an auxiliary bar rejected wheel input
  after Chromium consumed a mouse release. Both passed after the narrow guard
  and observer-order fixes; the full viewer library gate passed 53 tests.
- Final app-feature viewer library gate: 53 passed; its strict Clippy gate passed.
  The coordinator also completed final formatting, all-target strict Clippy and
  the combined app/native-scroll/viewer-probe debug build. The affected Linux/app
  compile check passed. Final native viewer qualification is recorded below.

Logs are in `_build/qa/smooth-scroll-20261111/`: `viewer-tests.log`,
`viewer-first-frame-red.log`, `viewer-cancel-red.log`, `viewer-guard-red.log`,
`viewer-target-red.log`, `viewer-cancel-green.log`, `viewer-web-popup-red.log`,
`viewer-web-popup-green.log`, and `viewer-web-popup-clippy.log`.

## Native measurements before CPU follow-up

`performance_probe --wheel --d3d` posts real native wheel messages to its own
window. It uses the application's DX12 policy and observes Winit RedrawRequested
and about_to_wait. It never installs a Slint rendering notifier, which changes
FemtoVG flushing. A bounded stripe from the presented raster is sampled after the
completion timestamp; pixel matching runs after each gesture. Matching can be
ambiguous for repeated text rows, so its estimated displacement is diagnostic,
not an exact coordinate oracle.

Private runtime: `C:\Users\tjelinek\MidnightViewerWheel-20261111-55ed211d`.
Its viewer plugins were copied from the protected installed runtime; no plugin
trust policy was weakened. Native window 2250×1575 physical, scale 2.25, no
synthetic CPU load. Frozen `wheel-trace1.exe` is the original-renderer control;
`wheel-candidate1.exe` includes the initial bounded binding-cache candidate.

The candidate completed all three fixture stages. Plain/source rendered 5–9
positions per notch, with typical movement gaps 34–43 ms and p95 about 50–58 ms.
The first plain/source notch's pixel matches show physical steps around 2.2, 8.9,
15.8, 22.0, 29.1 and 36.2 pixels, rather than whole-line jumps. Retained images
rendered 13–14 positions with typical gaps around 16 ms. Idle restart still cost
roughly 480–520 ms for text and 352 ms for the image. These results do not meet a
consistent 60 Hz target.

Evidence: `viewer-wheel-trace1.txt`, `viewer-wheel-candidate1.txt` and their stderr
logs. The first diagnostic had an image ActualPixels preparation race; the probe
now retries only after a distinct stable image context.

## Backend isolation

The new `midnight-viewer-engine/examples/wheel_latency.rs` times direct raster
work and real-worker pixel changes separately from all native UI/GPU work. It
uses 100 small arbitrary fractional movements per case. The baseline used the
existing renderer opt2 / engine opt1 overrides.

| Baseline case | p50 ms | p95 ms |
|---|---:|---:|
| Direct raster, 1000×600, scale 1 | 16.44 | 17.41 |
| Plain worker, same geometry | 13.18 | 13.92 |
| Source worker, same geometry | 13.46 | 14.23 |
| Direct raster, 2250×1400, scale 2.25 | 37.66 | 38.86 |
| Plain worker, same geometry | 31.59 | 37.33 |
| Source worker, same geometry | 33.67 | 35.74 |

`viewer-backend1.log` contains the clean timing run. A separate instruction sample
(`viewer-backend-profile1-symbols.csv`) attributed 310 of 842 executable samples
to skrifa, 248 to zeno, 147 to the viewer renderer, 50 to swash and 40 to read-fonts.
The Zeno-associated raster work was hot; a targeted opt2 profile experiment is
reported below. These symbols alone do not establish the optimization level of
monomorphized work in its callers.

The native stack diagnostic is separate from clean timing. Its first bounded
capture exhausted its sample limit early: initial device enumeration, then WGPU
Queue::write_buffer / StagingBuffer allocation and full-frame image textures.
It did not establish pipeline compilation as the idle-stall cause.
## Zeno profile comparison

The frozen baseline and the zeno opt2 candidate ran serially, without concurrent
builds. High-DPI median direct raster was 49.30 versus 48.28 ms; plain worker
41.97 versus 41.96 ms; source worker 45.66 versus 45.04 ms. At scale 1, plain
worker was 13.57 versus 12.65 ms. Absolute timings varied from the earlier run;
both variants showed the same insufficient high-DPI result. This profile change
was not sufficient and was removed from the final configuration.
Logs: `viewer-ab-backend1.log`, `viewer-ab-backend-zeno2.log`.

The renderer now reuses phase-independent hinted monochrome outlines and their
exact bounds. Reference tests keep the existing Swash route for comparison.
Color/bitmap fonts retain that route, and fractional glyph placement is unchanged.
## Exact outline reuse and culling

The retained path/key budget is 2 MiB with at most 512 entries. HashMap/VecDeque
capacity and reusable Zeno scratch storage are additional. Boxed point/verb
arrays have no unused vector capacity; accounting includes both stored variation
coordinate keys. Unsupported/negative fallback results are not retained.

The cache retains hinting and synthetic embolden/skew results, then applies the
original exact phase during mask rendering. Actual hinted control-point bounds,
with conservative mask-edge padding, reject offscreen glyph masks. Culled paths
never enter the phase-image cache as missing glyphs. This is tested by rendering
offscreen before returning to the same visible phase.

The all-target renderer gate passed 45 tests, including 24 exact framebuffer
comparisons across fractional origins, clipping, sizes and requested styles.
Those framebuffer cases use installed fonts; their sample text does not force
every possible variable/color/bitmap route. A separate direct Swash comparison
forces synthetic embolden and skew and compares mask placement/data.

Clean serial comparison, 100 fractional frames per case, same zeno2 profile:

| Case | Control p50 ms | Outline cache p50 ms | Candidate p95 ms |
|---|---:|---:|---:|
| Direct raster, 1000×600 |15.55|5.17|5.50|
| Plain worker, 1000×600 |12.99|5.91|6.83|
| Source worker, 1000×600 |16.62|6.75|7.48|
| Direct raster, 2250×1400 |46.80|14.21|22.97|
| Plain worker, 2250×1400 |38.92|20.36|33.42|
| Source worker, 2250×1400 |41.23|26.71|41.80|

The change is material, but high-DPI backend p95 still exceeds a 16.7 ms frame
budget. These backend numbers alone do not establish displayed 60 Hz motion; the
final native run below confirms the remaining raster limit. No phase quantization,
whole-file read, plugin change or unbounded glyph cache was introduced.

Evidence: `viewer-outlines-red.log`, `viewer-outlines-green.log`,
`viewer-outlines-final-lib.log`, `viewer-outlines-ab-backend-zeno2.log`, and
`viewer-outlines-ab-backend-outlines.log`.

## Final native qualification

The final combined application feature graph built `me`, `native_scroll` and
`performance_probe` together. The frozen probe is
`C:\Users\tjelinek\MidnightViewerWheel-20261111-55ed211d\wheel-final1.exe`,
SHA-256 `3AE49CAE7AB5C75E58C4C9BD7F152DA99B687DD6E71EF83A7988951440262AE2`.
It includes the exact outline cache and final renderer changes, with the unhelpful
Zeno opt2 override removed. It ran after all Windows/Linux compilers exited,
using the existing trusted viewer plugins, DX12 application policy, no synthetic
CPU load and no source-read instrumentation. Physical window: 2250×1575;
scale: 2.25. No rendering notifier was installed.

The process exited 0 and all three fixture stages passed the multi-frame gate.
Each stage used three real native wheel detents (forward, forward after idle,
reverse). The gate requires multiple completed content positions and prevents a
single frame from accounting for most of a notch; it is not a 60 Hz guarantee.

| Fixture | Completed positions per notch | Per-notch p50 movement gap, ms | Worst per-notch p95 gap, ms | Idle second-notch first content, ms |
|---|---:|---:|---:|---:|
| Plain text |7–11|24.35–39.55|56.83|513.96|
| Source |9–13|22.75–26.50|44.23|451.76|
| Retained image |13–15|15.99–16.77|18.16|307.58|

Raster stripe matches demonstrate fractional physical motion, including 6.964,
2.921 and 1.068 pixel steps with low matching error. Repeated rows also produced
ambiguous large-shift aliases; those are not displacement evidence. The final
native source case presents more intermediate content positions than the earlier
5–9-position candidate, but neither raster case sustains a 16.7 ms frame budget.
The image reaches approximately 60 Hz during motion; its idle restart still
stalls. Idle-stall attribution is not established by this timing run.

Evidence: `viewer-wheel-final1.txt`, `viewer-wheel-final1-stderr.log`, and
`viewer-wheel-final1-metadata.txt` under `_build/qa/smooth-scroll-20261111/`.
The native measurement slot was released only after the viewer process exited.

## Concrete follow-up for raster cadence

This is an identified next step, not an implemented or measured claim:

- Retain a bounded raster viewport plus an edge strip and move its presentation
  camera on the GPU while the worker prepares newly exposed pixels. Current text
  painting regenerates exactly the full viewport, and UI presentation uploads a
  fresh surface for every completed frame. The two decoded edge lines are not
  currently retained pixel coverage.
- Reuse the existing image camera/texture ownership and acknowledgement pattern,
  but keep raster content metadata distinct from `ViewerFrame.image`, which also
  controls image mode, selection, typography and search behavior. Pause at missing
  coverage rather than revealing blank pixels or applying stale acknowledgements.
- Preserve source-anchor rebasing across text fragments, the source gutter during
  horizontal panning, selection coordinates, and file/font/zoom/theme invalidation.
  Require bounded memory/loading, stable seams and exact settled reference pixels;
  fractional GPU texture translation is not automatically identical to rerendering
  glyph masks at every phase.
- PDF already has image surface/sequence metadata and a cropped viewport API, but
  `ImageView::request_document` bypasses retained tiles. Feeding bounded document
  tiles into the existing image camera is a concrete separate route without a
  plugin ABI change.

No retained-raster architecture change was made during final qualification.
