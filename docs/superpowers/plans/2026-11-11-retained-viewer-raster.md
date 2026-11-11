# Retained viewer raster implementation plan

**Goal:** Display text, source, Markdown and binary/hex wheel motion at native frame cadence by moving retained pixels on the GPU, independently of bounded backend page loading and raster completion. Preserve exact settled pixels, source gutters, selection/search and fragment continuity.

**Ownership:** The coordinator owns viewer-ui presentation/camera and native acceptance. The viewer agent owns the app-internal core contract, engine/render implementation and focused regressions. The native measurement agent independently attributes idle restart stalls. No public plugin ABI or dependency changes are needed.

## Approved contract

- The native frontend sends `ViewerInput::RetainedRaster(true)` as part of opening. Existing viewport-only CPU consumers remain exact reference paths and allocate no retained surface.
- `ViewerFrame.raster` describes text rasters separately from `ViewerFrame.image`. Existing frame pixels hold the retained body image; optional gutter pixels live in the raster descriptor. No duplicate visible-viewport raster is allocated.
- A surface ID names one camera/layout context. It survives overlapping fragment refills, but file/mode, arbitrary seek/search, resize/font/wrap changes replace it. An immutable pixel ID changes only when body/gutter pixels change.
- Camera origin, raster origin and coverage use continuous effective raster pixel coordinates. The backend retains a page-local offset so exact source-anchor continuation can rebase a fragment without moving this camera.
- Effective raster geometry retains the existing 4 Mi-pixel viewport cap, which can reduce its dimensions/scale below actual window pixels. The frontend maps each axis using `info.viewport / logical_frame_extent`; that retained extent does not shrink when mode controls change the clipping canvas. Rendering and hit testing share this mapping; tests cover an effective scale different from native window DPI.
- Safe coverage is a min/max range of camera origins for which the full viewport is already rasterized. Known file/content limits are separate optional min/max origins. An unloaded frontier is not EOF.
- `Raster { surface, input }` carries absolute latest-wins `PanTo { sequence, origin, settled }`, origin-aware `Select { pixel_id, origin, ... }`, `SelectPage { pixel_id, origin }`, and surface-scoped `Copy`. Acknowledgements never pull a newer frontend camera backward. Non-pan commands are queue barriers.
- The frontend translates existing body pixels in both axes and the source gutter only vertically. It asks for replacement pixels before the guard is exhausted, pauses at actual coverage if delayed, and retains remaining intended motion.
- Selection requires an issued pixel identity from the current page/layout serial, plus a camera inside safe coverage. A page replacement invalidates selection authority before replacement pixels are available. Selection-only repaint retains the same coordinate authority, including while a guard read is in flight. Stale selection is rejected with a retry status; a rejected new selection blocks copying the previous selection. Ordinary overlap retains exact selected source endpoints and bounded selected text. SelectPage covers lines intersecting the displayed viewport, excluding added guard rows.
- Body and gutter together target at most 6 Mi pixels (24 MiB RGBA) per retained snapshot. Guards target 256 logical pixels per side, converted with the effective raster scale, and shrink to respect that cap. The initial parent DPI therefore preserves logical lookahead without increasing the allocation budget. A single Service-owned registry admits at most three live/reserved snapshots per worker and twelve across all workers, including closed windows and context resets. It holds the only weak pixel references and prunes a slot only after both body and gutter owners are gone; weak slice references themselves retain allocation storage. The resulting retained Arc storage cap is 288 MiB. Safe Vec-to-Arc construction can add up to one snapshot per active worker (four windows, at most 96 MiB). Frontend/GPU copies, layout caches and small retirement metadata are separate; this is not a claim about total process/GPU usage.
- Decode demand adds guard lines but retains the existing 12 KiB text / 4 KiB binary and 512-line ceilings. Shaping always uses the visible text width. Overscan never widens wrapping or reads a whole file.
- Forward refill requests start near camera minus guard; backward requests end near camera plus viewport plus guard. Refill overlap and frontend lookahead are capped to half the available camera span, and page completion does not recursively start another optional guard. Plain/source boundaries retain complete available logical-line context so wrapping and tabs do not change at a visual-row crop. Binary and semantic document continuation retain their existing rules. The continuation records the actual camera through exact source anchors. Selected source endpoints/text survive ordinary overlap refills.
- An incomplete byte-limited fragment shorter than a viewport temporarily uses the existing viewport path: it cannot promise continuous vertical coverage for a partial logical line. It retains the opt-in, advances exact source anchors, and can regain a new retained context on a later complete fragment. Short real EOF pages remain retained. No incomplete frontier is reported as EOF.
- Failed, cancelled, or empty guard replies release pending state and emit surface-scoped `RasterUnavailable`; presentation preserves current pixels and cancels the failed coast. A new reversal/retry is allowed, and an unprepared target never acknowledges. Exact settling within loaded content does not start optional guard reads.
- A settled request rerasterizes at the exact final fractional phase when needed. Intermediate GPU translation is allowed; settled framebuffer pixels must equal the existing CPU reference.

## Implementation and regression gates

- [x] Add internal types and focused failing engine tests for retained coverage, immutable pixel reuse, stale sequences/context, bounded guards and independent known limits.
- [x] Preserve continuous origin across forward/backward continuation, pending reversal and horizontal movement; never acknowledge a page that cannot cover the displayed camera.
- [x] Render bounded body/gutter overscan while retaining visible wrap width. Prove integer crop equivalence at fractional origins and exact source gutter/reference pixels.
- [x] Keep one current pixel snapshot and one active/latest pending page/camera request. Coalesce unconsumed absolute pans without crossing selection/file/mode barriers.
- [x] Preserve origin-aware selection, selected text/Copy, search highlighting and exact source coordinates across guard refills.
- [x] Add frontend camera tests with a deliberately stalled worker: multiple displayed fractional positions from one pixel ID, coverage frontier/resume, reversal, precise delta without a tail, direct scrollbar, context change and stable gutter X.
- [x] Run focused red/green tests in the coordinated Cargo slot and the full viewer-engine library suite.
- [x] Complete the coordinator-owned affected-package/all-target gates and strict Clippy.
- [x] Measure actual submitted/displayed content positions and pixels in the native probe at high DPI, including sustained wheel, idle restart, continuation and settled return. Record approximately 60 Hz typical cached movement and disclose each remaining frontier/idle/frame-gap limitation; a strict 60 Hz guarantee is not achieved.
- [x] Run final Windows app/native feature graph and Linux checks, document exact measurements and remaining limits, then let the coordinator deploy/smoke the authorized runtime.

The previous viewport-per-delta baseline is recorded in `docs/verification/2026-11-11-viewer-pixel-scroll.md`: high-DPI text/source medians 23–40 ms and idle first-content delays up to 514 ms. Those are the behavior to improve, not evidence that retained raster is already complete.

## Intermediate evidence (superseded by final qualification below)

The initial five engine regressions were observed failing because native opt-in still produced no retained metadata (`_build/qa/smooth-scroll-20261111/retained-raster-red.txt`). Later controlled failures reproduced both a terminal guard reply leaving the camera pending and an older displayed pixel ID being incorrectly rejected after selection-only repaint (`retained-raster-failure-red.txt`). Both regressions pass after their fixes.

The final focused eight-package application feature graph passes 29 engine raster cases (19 retained integration, seven allocation-budget, one mailbox, two existing raster-image/PDF cases), one renderer outline case, and 29 UI raster cases. The UI gate includes actual rendered body/gutter geometry and exact settled crop comparisons; backend comparisons additionally exercise semantic Markdown heading/code/quote/selection at effective scales 1 and 2.25 and binary hex rendering. Log: `_build/qa/smooth-scroll-20261111/retained-raster-final-green.txt`.

The freshly built complete viewer-engine library binary then passes 146 tests with four installed-package opt-ins ignored, no failures. This includes the new ten-thousand-pan coalescing/selection-barrier test and existing source/Markdown continuation, scrollbar, image, capture and preference regressions. Log: `_build/qa/smooth-scroll-20261111/retained-raster-engine-full.txt`. Scoped core/engine rustfmt and whitespace checks pass.

The shared allocation registry has independently observed global and per-owner red/green cases, including gutter-only ownership, worker close/context churn, concurrent reservations and prompt reaping without another open. Normal retirement reaps after dropping its frame; worker shutdown explicitly drops its last cache before reaping on the existing worker thread.

Native cadence, real plugin sustained down/up traversal, GPU pixel proof, and the coordinator's broad final checks remain pending. Neither backend offsets nor software-rendered geometry alone establish native 60 Hz performance. The previous viewport-per-delta native baseline remains in `docs/verification/2026-11-11-viewer-pixel-scroll.md` until final native comparison is recorded.


The bounded-fragment follow-up observed two additional RED regressions: an 80-pixel available camera span rerequested the same source anchor, and a one-pixel span incorrectly advertised retained continuity. Adaptive refill overlap now uses at most half the available camera span, matching frontend lookahead; page completion applies the latest target without recursively requesting another optional guard. Fragments unable to preserve and advance a whole source row use the viewport path and accumulate small deltas until a source anchor advances. Both directions pass, retaining the 12 KiB/512-line limits and the displayed frontier. The complete engine binary now passes 148 tests with four installed-package opt-ins ignored (`retained-raster-overlap-engine-full.txt`).

The wrapped/tab and fallback reversal follow-up observed three more RED regressions, then passed all three after narrow fixes: refill at a visual-row midpoint lost logical-line context; a backward response adding a missing prefix incorrectly kept the previous surface; and opposite-direction input consumed unapplied forward residual. Plain/source refill now preserves available logical lines using the renderer's logical break characters. If a backward response reveals a previously missing prefix of the first old line, source-camera restoration still applies but the raster context changes; it does not claim unchanged pixels. A fragment without enough complete-line overlap follows the bounded viewport path, and reversal immediately discards opposite-direction pending residual. Exact visible RGBA matches before/after forward and backward wrapped-tab refills.

Final follow-up evidence: the complete engine binary passes 151 tests with four installed-package opt-ins ignored, and the complete viewer UI binary passes all 73 tests. Logs: `retained-raster-wrapped-red.txt`, `retained-raster-wrapped-final-green.txt`, `retained-raster-wrapped-engine-full.txt`, and `retained-raster-wrapped-ui-full.txt`, under `_build/qa/smooth-scroll-20261111/`. Scoped rustfmt and whitespace checks pass. Native candidate1 was frozen before these follow-ups; final native qualification must use a rebuilt candidate. Broad final checks and native acceptance remain coordinator-owned gates.


## Automatic context handoff follow-up

Review identified a further boundary: a retained guard can load a bounded fragment that needs viewport-only rendering (for example, a byte-limited single long logical line). Publishing the prefetched camera directly would jump ahead of the displayed camera and discard pending movement. The same coordination is required when a newly revealed prefix changes a partial first line's layout.

The approved internal handshake is `ViewerOutput::RasterRebase { surface, transition }`, followed by `ViewerInput::RasterRebase { transition, origin }` carrying the actual displayed old-world camera in effective raster pixels. The worker keeps the requested-world/raw-restored-local mapping, holds speculative frames, and prepares the corrected camera using only the current bounded page or one ordinary bounded continuation. `ViewerFrame.raster_rebase` acknowledges the completed correction and stays on coalesced metadata frames until accepted new input/context invalidation. Stale/duplicate replies do not consume it. Explicit navigation invalidates the transaction. Failure/cancellation emits a scoped error, clears the wait and exposes a usable fresh context without a success acknowledgement. The frontend keeps the old picture and buffers remaining motion until correction is acknowledged.

Core/backend/UI handoff implementation is complete. The initial six new engine behavior cases and full-mailbox delivery case were observed RED, alongside seven UI behavior cases. The implemented backend withholds speculative pixels, rejects stale/duplicate and nonfinite replies, preserves bounded selected text, invalidates old Copy after rejected new selection, and cancels obsolete correction reads on explicit context changes. Missing anchors, failed layout, failed/cancelled/empty correction reads cannot produce a success acknowledgement. Failure control precedes a usable unacknowledged frame, including under mailbox pressure.

The mailbox remains bounded at eight outputs. It reserves one slot for the coalesced content frame and one for the latest raster control (`RasterRebase` or `RasterUnavailable`); ordinary events share the remainder. Thus neither a once-only handoff request/error nor its acknowledged content can be lost to ordinary-event saturation. No task, channel, full-page snapshot or unbounded ownership was added.

The combined handoff GREEN passes ten filtered engine cases and eleven UI cases. Complete freshly built suites then pass **160 engine tests with four installed-package opt-ins ignored** and **84 viewer UI tests**. These are intermediate full-suite counts before the subsequent UI precise-input backpressure case and error-status assertion. Logs: `retained-raster-rebase-red.txt`, `retained-raster-rebase-green.txt`, `retained-raster-rebase-engine-full.txt`, and `retained-raster-rebase-ui-full.txt` under `_build/qa/smooth-scroll-20261111/`.

A later controlled UI RED showed resumed precise pixels being dropped under queue pressure; its narrow fix passes all twelve UI handoff cases. The same combined run observed the backend error-status RED: the fresh frame immediately overwrote the preceding typed error. The backend now retains that message in the fresh frame. The final combined focused GREEN passes ten engine, twelve viewer UI, and two main UI cases (`retained-rebase-final-green.txt`), with no remaining handshake regression failures. Broader final gates and native requalification remain coordinator-owned; no current native 60 Hz acceptance is claimed.
## Final qualification and remaining limits

The final exact-file follow-up corrects scrollbar visual-range mapping and keeps
covered drags entirely on the retained presentation path, with bounded backend
requests for uncovered targets. Small text/source inputs up to 8 KiB may use the
existing 512-line ceiling within the unchanged block budget; larger files remain
viewport-paged. Explicit drag release always commits its final settled position.
Tests cover active-read authority and deferred-origin ordering as well as actual
software-rendered body/gutter movement.

Final Windows gates pass 1,217 tests with 20 explicit opt-ins ignored, formatting,
strict all-target Clippy and the app/native-probe build. Linux checks pass for the
same feature graph. All eight exact JSON/XML wheel/drag cases pass on GPU/software.
Startup smokes include actual no-adapter and late-surface failure followed by one
successful CPU retry. See the final hashes, commands, timings and limits in
[verification](../../verification/2026-11-11-viewer-pixel-scroll.md).

GPU is optional: retained images and their camera contract also work in Slint's
software renderer. Native cached motion commonly measures around 16-17 ms between
redraws, but late gaps up to 66.519 ms remain in the exact-file runs. Long incomplete
logical lines still use bounded viewport fallback, and PDF document tiles are a
separate optimization. These limitations remain tracked; the checkboxes above
mean qualification was executed, not that every viewer/platform meets uniform
60 Hz or that the broad implementation goal is complete.

## Opening geometry and pixel-delivery follow-up

The implementation now retains each frame's logical display extent through the
first source/Markdown toolbar layout and same-surface metadata updates. An initial
viewport-only frame can hand that geometry to retained presentation without a
scale change. Rendering, selection and rebase-resume input use the same effective
pixel mapping. Captured parent placement also seeds the child's exact DPI before
its first physical sizing; native scale events remain authoritative, while
logical-size-only placement does not invent a monitor scale.

Safe pixel-buffer conversion now runs on one bounded worker in the UI crate.
There is at most one prepared batch awaiting presentation, backed by the existing
coalescing source mailbox. Metadata updates reuse cached shared buffers without
retaining source pixel Arcs. Slint images remain UI-owned; engine/plugin contracts,
decoded-page limits and source-read behavior are unchanged. Frame admission still
uses existing stale-context checks, accepted buffers retain their lease through
replacement, and close/cancellation retires pending ownership off the event loop.
The detailed contract is in [Viewer scrolling](../../developer/VIEWER_SCROLLING.md#retained-text-pixels-and-delivery).

The first bounded page now uses the captured parent DPI. Guard sizing preserves
256 logical pixels of lookahead per side at that effective scale, subject to the
unchanged combined 24 MiB raster cap. Text decode remains capped at 12 KiB and
512 lines; this does not preload the whole file or increase source budgets.

Absolute scrollbar seeks retain the requested fraction until the returned page
has been shaped and can map that fraction to actual visible pixels. A bounded
context prefix allows for changing source-byte density; it is capped by the guard,
a quarter of the current page and half its available scroll span. If that page
still misses the target, the engine permits one bounded correction read before
reporting failure without a success acknowledgement. A newer deferred target
supersedes the old correction. Partial-page mapping therefore has a fixed read
bound and never acknowledges an estimated anchor as the final displayed position.

- [x] Add opening pixel/selection and captured-parent DPI regressions; observe their failures before the fixes.
- [x] Implement fixed frame extents, captured DPI initialization and bounded presentation-buffer preparation.
- [x] Record focused checks and native captured-parent opening/cadence qualification in [the follow-up verification](../../verification/2026-11-11-text-opening-scroll.md). GPU/software and exact drag targets pass; JSON drag outliers and slower high-DPI software pacing remain documented limitations.
