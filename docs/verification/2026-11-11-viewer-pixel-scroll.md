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

## Earlier viewport-renderer native qualification

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

## Architecture identified by the earlier qualification

The earlier qualification identified the following next steps. The retained text implementation is now under qualification; PDF document tiles remain a separate task:

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

The measurements above describe the earlier viewport renderer, before the retained-raster change below.

## Retained raster follow-up

Text, source, Markdown and binary/hex now opt into a bounded retained body image
and a separately pinned source gutter. The native camera moves existing pixels
between backend replies. The worker retains lazy byte/line limits and prepares
coverage ahead of the camera; an unavailable frontier never masquerades as EOF.
Exact settling uses the original CPU raster phase. See the
[implementation contract](../superpowers/plans/2026-11-11-retained-viewer-raster.md)
and [developer scrolling guide](../developer/SCROLLING.md).

The focused integrated gate passed 29 engine, 29 viewer UI and one renderer
regression. An unfiltered engine run passed 146 tests, with four explicitly
installed-runtime tests ignored, in 0.20 seconds. The UI gate includes an actual
software-rendered body/gutter comparison: a settled crop matches all 96,000
reference pixels, with native DPI different from the effective raster scale.
Those tests found and corrected the source gutter's implicit horizontal centering.
Guard-load errors, stale selections, selection-only repaint and selections made
while a guard read is in flight have separate regressions. Later final gates and
native results supersede these intermediate counts.

A separate common-clock stack capture identified the large idle stalls in the
old native trace: most samples inside slow redraws were DX12 staging-buffer
allocation, with the remainder in texture allocation/destruction. The pinned
FemtoVG renderer now has a bounded completion-aware upload-buffer pool. It does
not reuse in-flight buffers, introduce blocking GPU waits or submit additional
command buffers. Nine GPU validation regressions passed on the local Intel Iris
Xe DX12 adapter, including deferred submission order and exact output pixels.

The next immutable viewport-binding cache passed all eleven GPU regressions,
including eviction and deferred use after the cache itself is dropped. It keeps
32 bindings per renderer and never rewrites an in-flight uniform.

The frozen post-cache candidate (`DB0C4F38D13B39DB94AF92658CC094E31D9459E411CC1D8D0EC896222E15BD1B`)
completed seven-case native isolated and sustained runs with no concurrent compiler.
Cached motion gaps are typically 16–17 ms, but idle and refill stalls remain:
DX12 idle restart ranges from 323 to 527 ms in this run; sustained source and real
JSON reach 540 and 554 ms. Separate GPU snapshots pass all seven cases, proving
body movement, fixed horizontal source gutter and unchanged uploads while panning.
The System32 panel recheck has p50/p95/max frame gaps of 16.531/20.024/21.947 ms.
These are submission observations, not monitor scanout measurements.

Explicit Vulkan and high-performance DX12 adapter experiments did not produce a
consistent improvement. Vulkan reduced some idle delays but roughly doubled warm
frame intervals to 31 ms. The high-performance preference selected the available
NVIDIA RTX 3500 Ada laptop GPU, yet still showed moving-frame stalls up to 1.07 s.
Neither experiment changes the shipped adapter/backend policy. Full metadata,
limitations and artifact names are in [native verification](../NATIVE_SCROLL_VERIFICATION.md).

The later automatic page-handoff correction is tested separately: its new
regressions first fail on speculative publication/lost motion, then pass for
actual-camera restoration, late replies, full queues, failures, selection and
reduced effective DPI. The intermediate full engine suite passes 160 tests with
four installed-runtime opt-ins ignored; the viewer UI passes 84 tests. Final
integrated Windows gates below supersede those counts; native requalification is recorded separately.
## Intermediate integrated Windows gate (candidate 2)

The final application feature graph passes **1,187 tests, with 20 explicit
installed-runtime/measurement opt-ins ignored and no failures**. This includes
160 viewer-engine and 85 viewer-UI tests. The test command took 66.36 seconds,
including 42.81 seconds of compilation. Workspace formatting, affected-package
all-target Clippy with `-D warnings`, source-size limits (1,823 handwritten files)
and diff whitespace checks pass. Clippy took 151.86 seconds; the final combined
application/native-probe build took 7.62 seconds. Existing vendored FemtoVG warnings
are not new application warnings. No dependencies or lockfiles changed.

Fresh debug `me.exe` SHA-256:
`E698F54BD33051CBA48D251AD66D7A61C1433E8A7D6E4E0111021E5F9D6D1861`.
Its private plugin runtime was deployed and registered for this exact hash.
Backend, software UI and native GPU startup smokes all pass. The UI smokes use
System32 and the repository as panel locations; the captured native window was
visually checked for complete panels, toolbar rendering and absence of startup
plugin errors. These startup checks establish launch correctness, not scrolling
latency.

Logs: `retained-final2-{format,tests,clippy,build}.log`; startup artifacts:
`retained-ui-{software,native}-startup.png`, under the same ignored QA directory.
The matching Linux all-target check passes in Ubuntu 24.04 WSL after correcting
one stale Unix-only timestamp assertion to check both date and time columns.
The initial failed compile is recorded as a qualification finding, not a runtime
failure; the repeated check completes in 2 minutes 51 seconds. It checks the same
eight-package application/viewer/comparator feature graph. No Linux native frame
pacing claim follows from compilation. Log: `retained-linux-check.log`.

Subsequent user feedback identifies the exact 2,659-byte/67-line JSON manifest and
3,865-byte/80-line Windows XML as remaining interaction cases. These are small
bounded source/text pages, not the very-long-line fallback fixture. Targeted
wheel and scrollbar qualification, plus optional GPU/CPU startup behavior, is
tracked below as a follow-up to the completed gates above.
## Final exact-file and optional-renderer qualification

The final debug application SHA-256 is
`80E7D310F755218BDB652B5FE2602608F320372AAE305A155C8F1F4BA1D4D7E6`.
The exact JSON and XML files above reproduce a shared scrollbar bug on both GPU
and CPU: requesting 70% positioned at approximately 70% of file bytes, then
reported the visible scroll range as 100%. Repeated drags also discarded retained
pixels unnecessarily. The corrected mapping excludes the visible extent, reuses
covered pixels immediately and acknowledges the actual displayed position.

All eight final cases pass: JSON/XML, wheel/drag, GPU/software. Each of the four
drags finishes at 0.70000005, with changed, nonblank visible crops. GPU drag
input-to-rendered-thumb medians are 6.212 ms (JSON) and 4.851 ms (XML); software
medians are 15.548 and 16.002 ms. Final content settles in 4.655-17.522 ms after
release. Wheel moving-gap medians are approximately 16-17 ms. Individual late gaps
still reach 66.519 ms, so uniform 60 Hz is not established. Raw native wheel input,
Slint-dispatched drag input, CPU crop evidence, separate earlier GPU pixel proof
and foreground samples are distinguished in the
[complete measurement record](../NATIVE_SCROLL_VERIFICATION.md#exact-user-jsonxml-candidate-eight-cases-pass).
No compiler/linker overlapped any timed run.

Final Windows verification passes **1,217 tests, 20 explicit opt-ins ignored,
zero failures** (85.03 s including 47.56 s compilation): viewer engine 167,
viewer UI 95, app 39, core 51, main UI 807, UI input 21, renderer 6,
comparator engine 30 and comparator UI 1. Workspace formatting passes (40.62 s),
strict affected-package all-target Clippy passes (32.48 s), and the combined
app/native-probe build passes (6.23 s). All 1,830 handwritten source files meet
the 100,000-byte limit. The corresponding Linux all-target feature graph compiles
in Ubuntu 24.04 WSL (2 min 35 s); this is not Linux native timing qualification.
No dependencies or lockfiles changed. Logs: `retained-final3-*.log` and
`retained-linux-check.log` in the ignored QA directory.

The exact app hash has a registered protected plugin runtime. Backend, native GPU
UI and explicit software UI startup smokes pass. Three additional native process
checks exercise recovery: unavailable GPU at selector preflight and a later GPU
surface failure each recover exactly once into software and exit successfully;
explicit software rendering succeeds without a retry. Their three snapshots are
identical. Both the native startup and no-adapter recovered screenshots were
visually checked for complete panels/toolbars and absence of plugin errors.
The startup recovery never restarts an already visible session or replays file
operations. Failed hidden attempts cannot overwrite saved preferences/geometry.
See [startup evidence](2026-11-08-startup-first-frame.md).

Build output: `_build/debug/me.exe`. Protected installation:
`C:\Users\tjelinek\MidnightExplorer-20261111-pixel-scroll-80e7d310`.
Artifacts: `viewer-exact5-*`, `final3-ui-*-startup.png`,
`final3-{no-adapter,late-device,explicit-software}*`. The broad project goal and
remaining large-fragment/PDF/Linux desktop performance qualification are separate
from these completed regressions.
