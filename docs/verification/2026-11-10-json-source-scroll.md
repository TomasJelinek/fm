# JSON/source scrolling, 10 November 2026

The exact reported file is
`D:\temp\ovclash\results\2026-09-07-mfg\manifest.json` (2659 bytes), SHA256
`CA5E4B26FEBA253568C17A62869471A2AEB2646D194593F18D2F9611263D21E7`.
All investigations used read-only access or its identical `_build/qa/json-scroll/`
copy. JSON uses the independent Source viewer's JSON lexical profile.

The fixes are in the host renderer/engine. Page-local baseline rounding changed
the spacing of overlapping rows after a fragment was rebuilt; page-local glyph
raster phase compounded the change. Layout now preserves fractional baselines,
and glyph raster/cache phase comes from final screen coordinates. Font hinting
and cache budgets remain enabled. Source pages also retain known global lines
through exact shared scalar anchors. An unknown first-fragment column remains
explicitly unknown instead of erasing valid gutter numbers.

A final incomplete/trailing layout row previously entered the viewport before
the next text page was loaded. The refill guard now keeps it outside a complete
visible viewport. Short bounded fragments still display immediately, and huge
unwrapped logical lines advance by their captured fragment boundaries. Semantic
Markdown windows retain their existing context behavior.

The separate large-file follow-up bounds optional Source Locate work to the
nearest cheap checkpoint. Declined coordinates are per-page state; nearby
coordinates can recover, and unavailable columns are never fabricated. The
coordinator owns the updated Source DLL/package and native 10GiB qualification.
No dependency, SDK ABI or Slint change was needed.

## Linux evidence

The public renderer regression failed with **8295 changed pixels** before the
correction. Actual plugin/worker captures independently showed **5617 changed body
pixels and 634 gutter pixels** after waiting for Locate. Forward/backward source
coordinates, chained partial-line overlaps, empty optional Locate recovery,
incomplete tail rows and bounded long-line progress have causal regressions.

Final `tail-locate-green-linux.log` passes **116 viewer-engine tests**, four
existing installed-package opt-ins ignored, and **41 renderer tests** across all
targets. Session4536 exited0. No independent plugin source was changed for the
rendering correction; the subsequent bounded-Locate package change is separate.

`source_scroll` uses the production service and installed Source plugin, asserts
Source mode and highlighted string pixels, compares every first moving frame,
returns to identical top pixels twice, and compares 18 repeated absolute scrollbar
positions using actual generation/sequence acknowledgments. It additionally runs
240 warmed quarter-pixel moves, requiring identical pixels after each down/up.

| Viewport | Wheel step | Wheel frames | Changed overlap pixels | Wheel p50/p95 |
|---|---:|---:|---:|---:|
|1000x600|72px|32|0|8.63/10.78ms|
|1000x600|17px|120|0|4.37/10.71ms|
|700x500|43px|60|0|6.54/8.96ms|

Quarter-pixel p95 was **4.37/4.42/2.45ms**, respectively; all repeated drag
positions and return pixels matched exactly. These local Linux measurements
include plugin/worker/render/mailbox polling, not native UI composition latency.
Logs and PPM captures are `final-user-*` beneath `_build/qa/json-scroll/`.

A probe revision alone was insufficient to identify wheel movement: a pending
Locate redraw could arrive first with an unchanged camera. Raw false-failure
evidence is retained under `probe-correlation-before/`. The corrected probe waits
for changed position/page and still checks the first moving frame without pixel
masking or a relaxed threshold. Its final acceptance does not enable settling.

## Windows and deployment

The actual Windows file passes all three configurations through the installed
Source package. All overlap pixels, repeated drag positions and fractional
down/up return pixels match. Logs/captures are in `_build/qa/json-scroll/windows/`.

| Viewport | Wheel step | Wheel frames | Changed overlap pixels | Wheel p50/p95 | Quarter-pixel p95 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1000x600 | 72px | 32 | 0 | 4.92/9.60 ms | 4.74 ms |
| 1000x600 | 17px | 124 | 0 | 4.57/7.19 ms | 7.25 ms |
| 700x500 | 43px | 60 | 0 | 4.70/7.17 ms | 2.75 ms |

`final-viewers-windows.log` passes renderer **41**, viewer UI **43** library and
**17** integration tests. One new engine fixture initially omitted a mandatory
terminal source anchor; it was corrected to use the real validated Page shape,
without weakening its progress assertions. `final-engine-windows.log` then passes
all **116** engine tests, four existing installed-package opt-ins ignored.
`final-clippy-windows.log` passes strict all-target Clippy. The shared comparator's
30 tests pass under the desktop account; two native identity cases fail only in
the sandbox, as recorded in the paired comparator logs.

The corrected debug application was deployed and its runtime registration and
startup smoke passed with SHA256
`5FD645D5650C359497E1E22A7129FEA420C86B95D7B9ACADE90C0FB84E1CA8F2`.
The installed bounded-Locate Source DLL has SHA256
`2E9A0700D88013E787B217B963B8363F04EF1BD38DAB6CDA5509B5473D5AF9AB`.
The [large-file record](2026-11-10-large-text-viewers.md) distinguishes actual
10 GiB reads and process memory from these warmed scrolling measurements.
