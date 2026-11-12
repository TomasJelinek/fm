# HTML performance investigation — 2026-11-11

The installed CEF provider, its permissions, sandbox, resource capabilities and
renderer policy are unchanged in this baseline. The fixtures are owned local,
script-free documents under `_build/qa/html-performance-20261111`; no remote URLs
or personal HTML files are read. All native wheel input targets only the probe's
own window, without moving the desktop cursor.

## Measurements before further optimization

`wheel_latency` profiles the installed host's first colored content and each
wheel-to-changed-frame interval. Its bounded sampled pixel identity avoids timing
an unrelated blank frame. Twelve isolated gestures settle before the next input.
At 640×480 the first changed frame took 33–61 ms (median approximately 42 ms).
At 2187×1467 and 225% scale it took 64–103 ms (median approximately 75 ms), with
three or four changed frames per gesture rather than five or six. Startup after
session admission was 2733 ms for the initial small run and 606 ms for the warm
large run. Catalog discovery was measured separately at 163 and 211 ms.

`html_performance` uses the production ViewerWindows path and an owned native
parent. It sends native wheel messages and separately records first changed pixel
publication and the completion of the native redraw. Shared premultiplied pixel
buffers are sampled without copying a viewport. At 225% scale the content surface
was 2187×1422. The D3D run measured 185 ms construction, 1044 ms first publication
and 1095 ms first completed draw. The twelve wheel inputs reached publication in
81–131 ms and completed redraw in 102–154 ms. Software measured 5 ms construction,
909 ms publication and 942 ms draw; wheel redraws took 100–171 ms.

The existing combined resize/zoom/scroll diagnostic measured 371 ms to its first
accepted resized frame, but that is not a pure wheel latency measurement. These
new probes separate that behavior and explicitly fail an incomplete run.

## Investigation boundary

The HTML provider converts CEF BGRA paints to owned RGBA surfaces, retains a base
for popup composition, clones that base for transport and sends full raw frames.
The host validates generation, dimensions and raw budgets before publication;
the UI prepares one bounded pixel batch off the UI thread. The current viewport
requires about 12.4 MB per frame. CEF's configured frame ceiling is 30 Hz. Those
facts suggest several possible contributors; they do not yet attribute all of
the measured latency. `wire_latency` isolates the existing production browser
pipe and binary frame protocol using an owned child executable, without CEF.

The production output scheduler now wakes immediately when a control or paint
arrives, using one coalesced wake token. It preserves the 64-control FIFO and one
latest-frame slot, and gives a pending paint a turn after at most eight controls.
Previously a paint was taken only after a 5 ms control-channel timeout; continuous
resource/state messages could indefinitely postpone it. A regression fails against
that behavior and passes with the bounded scheduling change. The 30 Hz browser
frame ceiling, raw frame format, dimensions, generations and resource policy are
unchanged.

`wire_latency` measured 0.4–0.7 ms for 640×480 and 4.5–6.5 ms for 2187×1467 through
the actual contained browser-process pipes and validated raw protocol. Thus bulk
pipe transfer alone does not explain the measured wheel delay. This attribution
run overlapped compilation and is not used for a precise before/after claim.

Three output regressions pass: paint fairness under continuous controls, frame
coalescing/control overflow bounds, and repeated wake delivery. The integrated
native provider suite passed six library tests and five policy/surface tests.
Provider DLL SHA-256 is
`45a74c5e73ea627864832bce3278b364ce864096db73dcf18f0dba201da5a821`.
The original qualified DLL is retained for A/B with SHA-256
`347b99e554c5e111d9414ce14561c6f3b445e12284cf09e15d0a9c43f1fec3c2`.
Native after-measurements are recorded below. Raw logs, owned fixtures and
original changed package files are retained under the QA directory above.


## Native application-size A/B qualification

The same probe used a 3015×1845 physical parent at 225% scale. The bounded browser
surface was 2705×1550. The original provider's first completed draw was 1714.000 ms,
with a 143.233 ms median over twelve wheel inputs. The changed provider's warm run
was 1681.504 ms to first draw with 140.816 ms median wheel latency; the preceding
changed-provider run had 138.985 ms median wheel latency but an unexplained 17502.297 ms
startup outlier. All runs reported successful opening and all twelve changed-frame
results. The ~2–3% wheel difference is small and does not demonstrate that the
user's overall HTML slowness is solved. The queue starvation regression is fixed;
Chromium paint cadence and presentation remain dominant investigation targets.

The qualified production package is staged in
`_build/qa/viewer-settings-20261111/runtime-input/viewers/html`. Changed files were
verified against `runtime-files.json`; the original CEF/bootstrap payloads are
unchanged. The matching staging record is `runtime-input-stage.log` in this QA
directory. It must deploy with the rest of the final runtime input.

## Diagnostic instrumentation, not part of the qualified production artifact

The optional `diagnostic-timing` feature is off by default. It records only
numeric timing/dimensions and fixed stage labels in `viewer-timing.csv` under the
broker's already private profile; no filename, document bytes, key text or secret
is logged. Creation does not overwrite an existing entry. At most 2048 records are
accepted, less than 256 KiB even with concurrent writers, verified by a standalone
regression. It marks initialization, context creation, wheel receipt, paint entry,
conversion, layer copy, queue admission and wire write. Its purpose is to separate
Chromium compositor/readback delay from the already measured fast raw transport.
The isolated diagnostic build passed seven library tests and five policy/surface
tests, including the concurrent trace bound. Its DLL is retained only as
`_build/qa/html-performance-20261111/diagnostic-only-provider.dll`, SHA-256
`246dfde44915fb4e94f16829792334bf0fde182ffea978ad42b73e5896e24192`.
The matching logs are `diagnostic-build-final.log` and `diagnostic-tests.log`.
One native trace (`native-trace-1.csv`) was collected while the final Windows
build ran. It is attribution evidence under concurrent load, not an absolute
performance benchmark. The twelve gestures and opening succeeded. Across 75
paints, median conversion was 30.060 ms, layer copy 6.206 ms, queue admission to
wire start 0.051 ms, and wire write 11.223 ms. Wheel receipt to the next paint entry
had a 27.645 ms median. The qualified DLL and inventory were restored and their
identity verified immediately afterward. The feature is not included in the
staged production package, which remains the qualified default build.

## Checked-build conversion follow-up

The earlier cached-buffer microbenchmark used `rustc -O`, which disables development
checks. It therefore understated the production development conversion cost.
Repeating it with `opt-level=2`, `debug-assertions=yes` and `overflow-checks=yes`
(the provider's existing development settings) measured approximately 15–16 ms
for per-pixel byte-vector extension and 2.7–3.9 ms for exact typed row extension.
These are isolated conversion measurements, not whole-viewer speed claims.

The cropped paint path now allocates `Vec<[u8; 4]>` once, extends exact cropped
pixel rows, and converts it into bytes with `into_flattened`, reusing the allocation.
It keeps development checks, input length/dimension/revision guards and all alpha
bytes; it adds neither unsafe code nor another full-image copy. Scalar-oracle tests
cover every alpha value, zero/one/sixteen-pixel rounded edges, immutable input,
stale revision and malformed dimensions/length. Eight default-feature library
tests and five policy/surface tests pass (`conversion-default-tests.log`).

The default production candidate is retained separately in
`_build/qa/html-performance-20261111/provider-conversion-candidate`, SHA-256
`c00eb47e6352db612d61e2db0d875e58b75f9890a12a1f884e4589f8012c5603`.
It has no timing instrumentation. The fairness-only package remains separately
retained as `provider-fairness-before-conversion`.

Quiet native application-size qualification used the same 3015×1845 parent at
225% scale, the same probe binary, and twelve isolated wheel gestures per sample.
The fairness-only baseline median wheel-to-draw latency was 138.045 ms. The typed
conversion runs measured 104.712 ms and 109.311 ms, improvements of 24.1% and 20.8%.
All three openings and all 36 changed-frame gestures completed successfully.
First draws were 2585.711 ms before and 1049.327/1558.188 ms afterward, but native
window construction itself varied from 835.914 ms to 247.744/742.347 ms. Those
startup figures do not isolate the conversion improvement or explain all cold
opening delay. The remaining wheel delay is still visible; this is a measurable
improvement, not a claim that HTML now opens or scrolls instantly.

The qualified default candidate was staged into the package directory, the
private measurement runtime and the final runtime input. Each of the six changed
package files was byte-verified against the candidate, and DLL/manifest/docs hashes
were checked against its inventory. `conversion-runtime-stage.log` records the
staging. The deployed DLL must have the `c00eb47e…` identity above; the diagnostic
`246dfde4…` DLL is retained only under QA. Raw results and the exact median summary
are `native-conversion-*.stderr` and `native-conversion-summary.json`.

## Subsequent frame-pacing qualification

The [12 November follow-up](2026-11-12-html-frame-pacing.md) measures the later
60 fps ceiling with the same native probe. The 30 Hz settings and hashes above
describe this report's historical baseline.
