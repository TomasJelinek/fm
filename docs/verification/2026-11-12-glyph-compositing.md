# Glyph compositing qualification, 12 November 2026

This investigation starts at repository head `3522775`. A measured mask-composition
optimization now passes the scalar pixel oracle and paired backend measurements.
Final native qualification records 15/17 original numeric gates passing. Separate
pixel evidence resolves the two BAT Text failures as a normalized-position guard
limitation; a small scrollbar-thumb estimate discontinuity remains documented below.

The retained previous exact-Markdown trace has 24 recorded raster samples.
Recomputing the complete set gives median paint 27.856 ms, glyph paint 18.4645 ms,
clear 3.6875 ms and layout 1.6145 ms; the glyph p95 under floor((n-1)*0.95) is
21.940 ms. The earlier report's 28.067/19.826 ms paint/glyph values do not exactly
match that retained sample set, so they are not silently reused as a fresh
baseline. All numbers describe the recorded 225% run, not current-code timing.

The candidate must first demonstrate a material composition share, then preserve
complete output equality and improve clean, paired actual-workload runs.
`MIDNIGHT_VIEWER_GLYPH_TRACE` opts into debug-only mask-generation/composition
aggregate durations and call counts. It prints no text, paths, font identities or
source bytes and takes no per-pixel timestamps. Generation includes outline-cache
lookup/materialization and the existing Swash fallback; unaccounted glyph work
includes key/iteration/scaler preparation. Trace overhead is excluded from clean
performance comparisons by unsetting the variable.

Ruling: the functional pixel oracle is expected to pass before optimization;
the failing baseline is the measured refill/frame-time budget. Do not invent a
functional regression in already-correct pixels to manufacture a red test.

## Fresh attribution and candidate

All runs use the actual `D:/dev/ov-physics-trunk/ovclash/AGENTS.md`, SHA-256
`6656eea6ce179c31d7c2c1e08ab24443bf44afc4fae5dcc0e9e8462229d633ad`, Segoe UI 13,
and current protected plugins beside
`C:/Users/tjelinek/MidnightExplorer-20261112-eta-17e2147a/me.exe`. Only uniquely
named diagnostic executables were added there; no package, receipt or registration
was changed. The existing `seek_latency` example submits 60 position changes at
10 ms intervals across three legs (two reversals), waits for the final acknowledgement, and
opts into production retained raster delivery. It runs without a native window or
GPU upload. Its requested 1312x689 logical geometry yields a 2826x1484 raster
viewport at 225% after the existing pixel-budget clamp, and 1312x689 at 100%.

Three attribution runs at each scale, with both trace variables enabled, recorded:

| Scale | Painted frames | Median glyph | Median generation | Median composition |
| --- | ---: | ---: | ---: | ---: |
| 100% | 150 | 4.737 ms | 1.3095 ms | 2.0335 ms |
| 225% | 63 | 19.653 ms | 10.562 ms | 6.458 ms |

Composition accounts for 32.27% of summed glyph time in the 225% traces. This
justifies the narrow candidate: select mask content once, pair clipped source and
destination rows, skip alpha zero, and copy alpha 255 directly. Rounded alpha 255
requires both coverage and brush alpha 255. Intermediate alpha uses the same
premultiplication and source-over rounding through a shared private pixel helper.
Color outlines, color bitmaps and subpixel handling are unchanged. No cache, glyph
phase, layout, source-read, retained-raster or paging policy changed.

The independent oracle computes pixels by iterating destination coordinates. It
covers all 256 coverage values, seven brush alpha values, four brush colors,
transparent/translucent/opaque backgrounds, clipping and negative/fractional
positions, offscreen images, empty width/height, and color/bitmap fallback. The
oracle passed before optimization and passes afterward. Independent review found
no bounds or pixel-equivalence issue. Disabled debug tracing performs no clock or
log work (it still checks a cached flag); release builds compile its state and
clock/log operations out.

## Paired backend results

Three serial pairs at each scale alternate A/B and B/A order, with no concurrent
compiler or benchmark. Both binaries use the existing Windows dev profile and
package optimization overrides; these are not release-build measurements. Per-glyph tracing is disabled. The existing aggregate
scroll/raster phase trace remains enabled equally in both versions; these are
instrumented backend timings, not native frame-delivery timings. Medians use the
complete per-run sample set, and p95 uses sorted index floor((n-1)*0.95). Paint
counts differ because faster work allows more queued positions to be rendered.

| Scale, pair | Paint median A -> B | Paint p95 A -> B | Content frames A -> B | Final acknowledgement A -> B |
| --- | ---: | ---: | ---: | ---: |
| 100%, 1 | 6.380 -> 5.964 ms | 10.265 -> 10.117 ms | 52 -> 52 | 16.931 -> 26.252 ms |
| 100%, 2 | 6.323 -> 5.960 ms | 10.405 -> 10.674 ms | 51 -> 51 | 16.825 -> 26.485 ms |
| 100%, 3 | 6.589 -> 5.805 ms | 10.356 -> 10.408 ms | 51 -> 53 | 24.415 -> 25.855 ms |
| 225%, 1 | 27.425 -> 23.536 ms | 31.005 -> 26.727 ms | 23 -> 26 | 77.650 -> 50.943 ms |
| 225%, 2 | 28.500 -> 24.614 ms | 31.039 -> 27.158 ms | 22 -> 26 | 62.448 -> 55.263 ms |
| 225%, 3 | 26.665 -> 23.507 ms | 30.111 -> 26.565 ms | 24 -> 26 | 61.597 -> 52.372 ms |

The 225% paint median improves by 11.8-14.2% in every pair; glyph medians fall from
18.733-20.183 ms to 15.108-15.901 ms. A separate candidate attribution run shows
composition of 3.012 ms on the final large viewport with 3,402 composition calls;
baseline attribution runs 2 and 3 end with the same call count at 5.335 and
5.310 ms. This is supporting component evidence, not an additional paired native
measurement. The remaining generation cost is substantial and is not bypassed.
At 100%, median paint improves but p95 is mixed and final acknowledgement is
slower. Position coalescing/settling and differing accepted intermediate viewports
mean these samples do not prove uniformly faster seeking. All twelve runs reach
the requested final position. No uninterrupted 60 Hz or native GPU speedup claim
is made.

## Commands and artifacts

Use `_build/qa/navigation-ordering/windows-env.ps1`, then override the pinned SDK
config with `_build/media-cancel/sdk/worker-sdk-1794390611415302600/.cargo/registry.toml`.
Every Cargo command uses `--config $sdkConfig --target-dir _build --locked --offline`.

- Baseline: `cargo test ... -p midnight-viewer-render --lib`: 9/9 pass.
- Candidate: `cargo test ... -p midnight-viewer-render --lib --test raster --test source_continuity`:
  14/14 pass, including full framebuffer/cached-font/selection/source-rebase checks.
- Baseline/candidate: `cargo build ... -p midnight-viewer-engine --example seek_latency`.
- Probe: `glyph-seek-{baseline,candidate}-20261112.exe D:/dev/ov-physics-trunk/ovclash/AGENTS.md`,
  with `MIDNIGHT_VIEWER_SEEK_SCALE=1` or `2.25`, and trace variables as described above.

Artifacts are under `_build/qa/glyph-compositing-20261112/`: baseline/candidate
build and test logs, `baseline-phase-*`, `candidate-phase-2.25.*`, `paired-md-*`,
`paired-summary.json` and the preserved executables. Baseline SHA-256:
`dbb5de401a2aaae4f9787319c600e83c2f6601ae5e5e955a87fea7a2768a14b9`.
Candidate SHA-256:
`ce3e3e452523107ba9200431ae43c0fe66cf25a71c4b68fe38e8e025722b6bed`.

The same 14 renderer library/framebuffer/source-continuity tests also pass on
native Linux under WSL, with no skips. The coordinated font-catalog qualification
ran them against the integrated candidate using the existing `_build/linux`
target, one build job, locked/offline dependencies and
`_build/qa/elevation-linux-20261111/sdk.toml`. Exact script and results:
`_build/qa/monospace-20261112/ordinary.sh` and `green-ordinary-lint.log`.
That run also passed strict Clippy for the renderer library and the monospace
integration target. Direct rustfmt checks on all four glyph/raster-owned Rust
files and `git diff --check` pass; the largest owned source is 12,947 bytes.

## Final native qualification

The final application and probe were built before the quiet native batch. The
protected runtime is
`C:/Users/tjelinek/MidnightExplorer-20261112-tree-delete-07f4fcc1` with the same
qualified plugin set. Application SHA-256:
`07f4fcc14373c87cf677ec800a50217e8078f196d674b6704c2ad7f63fdfde12`.
The timing probe SHA-256 is
`0911cda23d62dec876089fd398b0afb62ebb19b86f6a403ce38e1b137561ace0`.
No package, receipt or registration changed for the probe.

`run-native.ps1` in the glyph QA directory runs wheel gestures for the actual
Markdown, BAT Source, BAT Text, JSON and XML in D3D and software, then MD/JSON/XML
scrollbar drags in both, plus one sustained Markdown D3D run. The supplied files
have sizes 213103, 22632, 2659 and 3865 bytes. Ordinary cases use a 1000x700 logical
window (2250x1575 physical at scale 2.25); sustained Markdown uses 1340x820 logical.
The text paths are the Markdown above, `D:/dev/ov-physics-trunk/ovclash/build.bat`,
`D:/temp/ovclash/results/2026-09-07-mfg/manifest.json` and
`C:/Windows/System32/WwanFeatureTests.xml`.

The original numeric gate passes **15/17**, not 17/17. Both BAT Text cases fail
its normalized-position fraction check; all other wheel, drag and sustained cases
pass. All six drags reach 0.7 with changed, nonblank retained content. Their crop
checks rerender CPU pixels; they are not GPU compositor readback.

| Renderer, file | Content updates during 60 drag moves | Maximum content gap | Final content after release |
| --- | ---: | ---: | ---: |
| D3D, Markdown | 34 | 53.738 ms | 72.807 ms |
| D3D, JSON | 30 | 80.785 ms | 72.497 ms |
| D3D, XML | 29 | 74.405 ms | 83.814 ms |
| Software, Markdown | 36 | 42.571 ms | 116.995 ms |
| Software, JSON | 35 | 83.937 ms | 125.078 ms |
| Software, XML | 32 | 83.178 ms | 120.595 ms |

Sustained Markdown records 623 moving frames: 568 gaps within an existing texture
have p95 17.271 ms and maximum 32.573 ms; 54 texture-transition gaps have p95
24.019 ms and maximum 25.852 ms. Position spans 0.099959604 to 0.173660770 with
bidirectional motion. Markdown horizontal extent stays exactly 1 throughout wheel
and drag traces. Refill and drag gaps remain observable; these results do not
establish uninterrupted 60 Hz. They are final-build qualification, not a native
baseline/candidate speed comparison.

Every wheel run records `Focused(true)` with no subsequent focus-loss event.
The probe posts native input to its own HWND and optionally clicks its text area;
it does not force OS foreground or sample `GetForegroundWindow` continuously.
Scrollbar gestures use Slint pointer dispatch. Isolated wheel aggregate logs
include the deliberate pauses between notches; their approximately 500 ms gaps
must not be read as continuous-scroll stalls. Only the sustained case supplies
the continuous pacing figures above.

## BAT Text guard diagnosis and remaining thumb follow-up

The minimal reproduction adds `MIDNIGHT_VIEWER_PROBE_MODE_INDEX=1`,
`MIDNIGHT_VIEWER_PROBE_START=0.1`, `MIDNIGHT_VIEWER_PROBE_SIZE=1000x700`, Segoe UI 13
and content focus to a normal exact-file wheel run. In both renderers, the first
notch has 13 closely spaced displayed-position samples but a normalized-position
reversal at raster replacement, triggering the existing 0.65-fraction guard.

`navigation::scroll_metrics` derives that normalized coordinate from each bounded
page's byte/height density. A replacement page can change the estimate while the
retained camera and visible glyphs continue moving. To distinguish those cases,
a separate opt-in proof samples at most 64 columns by 640 rows from the displayed
retained texture crop, using the actual fractional texture translation. It stores
at most 64 stripes per gesture, compares them after the gesture, and never uses
normalized position to calculate spatial displacement. It is disabled by default,
loads no source bytes, adds no public API and does not change the original guard.

The separate D3D/software proof runs show:

| Refill | D3D normalized delta / pixel shift | Software normalized delta / pixel shift |
| --- | ---: | ---: |
| First downward notch, upload 5 -> 6 | -0.003144629 / +13.926 px | -0.003147006 / +13.909 px |
| Second downward notch, upload 6 -> 7 | -0.004307054 / +17.262 px | -0.004307635 / +17.219 px |
| Upward reversal, upload 7 -> 8 | +0.005993716 / -17.220 px | +0.006026313 / -16.761 px |

Matching error relative to an unmoved crop is 0.00144/0.00298 at the first failing
boundary, and below 0.014 at the other listed boundaries. Each notch has 13 pixel
samples and totals approximately +135, +135 and -135 raster pixels; the largest
individual step is about 20.8 pixels (15.4% of a notch), not the reported 70%.
This demonstrates continuous glyph motion through the observed BAT page rebases.
It is a CPU-source crop correspondence proof, not a GPU readback or timing run.
The original guard still fails and all original failure reports are preserved;
its normalized coordinate is unsuitable for this cross-page motion assertion.
No production correction or threshold relaxation was made.

**Remaining follow-up:** `raster_presentation::present` also assigns the recalculated
coordinate to the visible scrollbar position when no absolute seek is pending.
The thumb can therefore move opposite the wheel by about 0.31-0.60% of its usable
track while content moves correctly. Physical thumb displacement was not measured.
This bounded-page estimate continuity issue remains separate from glyph rendering;
this change does not claim that all viewer UI continuity is fixed.

Proof flag: `MIDNIGHT_VIEWER_PROBE_SPATIAL_PROOF=1`. Reports and summary are
`spatial-{d3d,software}-bat-text.*` and `spatial-summary.json`. Their probe SHA-256 is
`104b8a6d6370573b5a6b0e3d4736215aa141b65bf2605882f43875c4e64872d7`.
These preserved reports predate a header-only correction: their dedicated
`wheel_spatial_proof` line correctly says `timing_excluded=true`, while the generic
first-line snapshot exclusion field still says false. Final source fixes that
header; no proof timings are used in this document.

The final probe target passes all 4 tests, including fractional translation across
a source rebase and rejection of uncovered/nonfinite crop coordinates. Strict
scoped Clippy and rustfmt/diff checks pass after the header correction. Exact final
logs are `spatial-proof-header-tests.log` and `spatial-proof-header-clippy.log`;
commands use the same pinned SDK and stable seven-package feature union, with
`cargo test ... --example performance_probe -- --test-threads=1` and
`cargo clippy ... --example performance_probe -- -D warnings`. The application hash
remained unchanged throughout this probe-only diagnosis. The renderer plan is
complete with the limits above; no additional native benchmark repeat was needed.
