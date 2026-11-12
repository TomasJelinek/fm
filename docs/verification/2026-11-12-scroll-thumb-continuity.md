# Bounded scrollbar continuity (SCROLL-01)

Starting commit: `6f06ec4`, authoritative repository `D:/experiments/dev/FM`.
This work follows the preserved BAT evidence in
[the glyph verification](2026-11-12-glyph-compositing.md), which demonstrated correct
pixel motion with opposite normalized thumb jumps of roughly 0.31–0.60%.

## Cause and change

The retained camera used continuous world pixels, but every bounded page supplied a
new average bytes/height estimate. Replacing both its position intercept and visible
extent moved the scrollbar independently of the unchanged content.

The worker now joins visual row boundaries from its existing shaped fragment to the
provider's existing validated source anchors. Parallel table cells share a semantic row
span, with an endpoint independent of a subsequently loaded paragraph. The immutable map
is shared with presentation using Arc and translated into retained world coordinates.
There are no new reads, provider requests, scans, retained navigation histories, dependencies,
or plugin wire/API changes. Storage is bounded by decoded visual rows (an explicit
262146-anchor ceiling); metadata frames reuse the map allocation.

Viewport top and bottom source fractions determine both normalized position and extent.
Presentation evaluates them at the actual camera, and covered absolute seeks use the same
monotone inverse. Unknown positions still follow the existing bounded request/refinement
path. Direction is not clamped. Equal-fraction generated regions preserve their current
visual occurrence for an unchanged interior seek, including its UI float representation;
0 and 1 still choose the real endpoint. Existing completely loaded layouts retain their
exact linear pixel range. A continuation that discovers the complete file retains the
source model, preventing a mid-scroll coordinate-system switch. Width/font changes rebuild
the map with the layout. Tiny incomplete fragments shorter than a viewport retain the
existing explicit fallback because they cannot describe a whole visible source interval.

## Focused regression evidence

QA directory: `_build/qa/scroll-thumb-20261112`.

The RED regression uses long and short BAT-style command/comment lines and an overlapping
bounded replacement at the identical retained surface and world origin. Unchanged production
moved position `0.7528008493311028 -> 0.7892631486789767`; `red.log` records the failure.
The same test now passes without changing its continuity assertions.

Current focused results:

- Core map: 7 passing tests (coupled position/extent, normal/minimum thumb geometry, inverse,
  endpoints, rebase allocation sharing, invalid coordinates and equal-fraction plateaus,
  including rejection of an interior cached plateau as a false BOF/EOF).
- Renderer anchors: 2 passing tests (wrapped UTF-8/tabs and centered unequal table cells,
  including stable table endpoints after adding following context).
- Engine position: 12 passing tests, including complete/partial absolute seeks, bounded
  correction, stale acknowledgement rejection and tiny-fragment fallback.
- Engine retained raster: 37 tests passed before the final edge additions; the newly added
  continuation-to-complete-file and unequal-density tests pass 2/2, and the strengthened
  bidirectional wrapped-tab pixel/coordinate test passes 1/1.
- Viewer UI: 55 focused raster tests pass, including unchanged thumb geometry, immediate
  reversal, local seeks, metadata-only delivery, rebase and exact pixel crop boundaries.

Commands use `windows-env.ps1`, the pinned SDK registry config under
`_build/media-cancel/sdk/worker-sdk-1794390611415302600/.cargo/registry.toml`,
`--target-dir _build --locked --offline`, and `-- --test-threads=1`:

```text
cargo test -p midnight-core --lib viewer::scroll_map
cargo test -p midnight-viewer-render --test scroll_anchors
cargo test -p midnight-viewer-engine --lib worker::tests::position
cargo test -p midnight-viewer-engine --lib worker::tests::raster
cargo test -p midnight-viewer-engine --lib scroll_mapping
cargo test -p midnight-viewer-engine --lib retained_wrapped_tab_refill
cargo test -p midnight-viewer-ui --lib raster_
```

The four existing probe tests pass (`probe-tests.log`), and its initial build succeeds.
The final endpoint correction was independently reviewed and passes all seven std-only
core tests through direct `rustc --edition 2024 --test` (`core-map-final.log`); this avoided
contending for the shared Cargo target. The final probe rebuild, native verification and
combined lint results are recorded below. The prepared runner tests the
actual BAT in Text mode at 0.1, 0.7 and EOF with reversal in D3D/software, plus BAT Source and
Markdown wheel and Markdown absolute drag checks. Its optional spatial crop proof remains
separate from timing. The added thumb fields sample PositionBar's published track/position
and its exact layout formula; they are not GPU compositor readback.


## First native batch and probe correction

The first final candidate probe is SHA-256
`d492fdb8da925bd5ad18bcf61776eaf432b3eb37fca311562c816f28da48959b`, staged uniquely in
`C:/Users/tjelinek/MidnightExplorer-20261112-tree-delete-07f4fcc1`. No plugin, receipt or
registration changed. Both Windows and Linux compilation were quiet during the batch.

The 12 cases return 11 nominal guard successes and one D3D EOF frame-count failure.
**Two of those nominal successes are invalid:** D3D BAT Text requested 0.1/0.7 before the
mode-change frame delivered valid position authority, so their gestures started at 0.
They are retained as invalid probe evidence, not claimed passes. Thus the first batch has
nine valid passing cases, one valid failure, and two invalid cases. All six software cases
reach their intended starts and pass; D3D BAT Source, MD wheel and MD drag also pass.

The probe correction waits for the requested delivered mode and rejects provisional
`mode:N` contexts. It then requires the actual displayed position to acknowledge the initial
seek, in addition to the existing pending/settle checks. It does not relax the existing wheel
fraction/frame-count gates. Two new tests cover the mode race and rejected/pending seeks.
Their final Cargo qualification passes all six probe tests; the corrected D3D repeat is
recorded below.

Every valid recorded adjacent pair moves normalized position and the thumb's computed
layout position in the requested direction. In software BAT Text at 0.1, the three notch
endpoints are 0.113656104, 0.126656413, and exactly 0.113656104 after reversal. Source-pixel
crop correspondence totals +134.957, +134.927, and -134.994 pixels. Four refill boundaries
move the thumb +1.25204, +0.76371, +1.02200 and -1.15947 logical pixels, respectively.
At EOF the endpoints are 1 -> 0.996178985 -> 0.990275919 -> 0.996178985. At 0.7 the first
notch's remaining tail crosses the probe's fixed gesture interval (125.554 + 144.315 pixels
across the two down notches); these results do not establish uniform smooth pacing.

D3D had 200–550 ms redraw delays during the candidate run. A preserved original probe,
`glyph-performance-final-20261112.exe`, then reproduced the large delay on the same exact
Markdown/runtime/geometry: five moving frames per notch and redraw p95 values
340.930 / 651.473 / 264.505 ms (`baseline-d3d-md-wheel.*`). This establishes that the observed
large delay also occurs without this scrolling/footer change; it does not identify an OS,
driver, security-software cause or prove that all new costs are zero. The existing D3D
performance issue remains open. No native speedup or uninterrupted-60-Hz claim is made.

Raw first-batch outputs, `native-summary.json` and `summarize.py` remain in the QA directory.
Thumb positions are derived from the published PositionBar geometry and exact layout
formula; spatial correspondence uses CPU source pixels. Neither is GPU compositor readback.


## Final corrected D3D gate and status

The final corrected probe SHA-256 is
`abdaa33884cc14a657ebb5024446e67beb85927fafc5c7ae3ff496eaceb2f85e`.
Its unique executable in the same qualified runtime is
`scroll-thumb-abdaa338-corrected-20261112.exe`. Only the three previously affected D3D
cases were repeated, with Windows/Linux builds idle. Outputs are under the QA `corrected/`
subdirectory; first-batch reports are unchanged.

All three corrected runs acknowledge their intended starts: 0.100000001, 0.699999988 and 1.
No recorded normalized or computed thumb step reverses the requested direction. The 0.1
case crosses four refill boundaries with thumb deltas +1.25118, +0.92332, +1.22187 and
-1.13148 logical pixels. The 0.7 case crosses two more refill boundaries in the proper
directions. The 0.1 original guard passes. Both 0.7 and EOF retain their original guard
failure because notch 1 has only three presented positions during the fixed interval.
Their source-pixel sums also show incomplete motion within those fixed 700 ms gesture
intervals. They are not smoothness passes, and the guard was not relaxed.

Replacing the first batch's two invalid starts and its EOF failure with these corrected
runs gives **10/12 valid original-guard passes and two D3D frame-count failures**. The
scrollbar-coordinate regression is fixed and verified through bounded refills, reversal,
real endpoints and absolute seeks. The separate D3D pacing issue remains open; the preserved
baseline reproduces it. No further production changes were made to mask those timings.

Final combined Windows evidence, independently inspected in
`_build/qa/life01-engine-20261112`, includes:

- `affected-units-green.log`: core 68, viewer renderer 9, viewer UI 115 tests pass;
  viewer engine 187 pass with four pre-existing tests ignored because they require separately
  configured native/image/PDF/source plugin fixtures. The new scrollbar regressions all run.
- `clippy-windows.log`: strict `--all-targets --no-deps -- -D warnings` succeeds for the
  affected package union, including all viewer/core packages. The first run's reported
  `collapsible_if` was corrected without changing behavior; its original log is retained.
- `probe-tests-final.log`: all six example tests pass, including both readiness regressions.
- `probe-build-final.log`: final probe rebuild succeeds in 4.15 seconds.

The plan's implementation and focused qualification are complete with the native performance
limits above.
