# Retained complete-page seeking — 2026-11-12

The same-input foreground comparison reduces JSON drag content gaps and raster
replacements by preserving the existing retained surface across uncovered seeks
within a complete page. XML's median and maximum gaps also improve, but its p95
increases slightly. These observations support the specific cache-reuse change;
they do not establish universal 60 Hz presentation.

## Cause and scope

Investigation started from `1c1b0b6` in `D:/experiments/dev/FM`, following the
[foreground refill qualification](2026-11-12-foreground-scroll-resume.md). Its
remaining JSON content gap reached 105.554 ms. The retained camera rejected a
thumb target beyond current pixel coverage even when both document ends and its
complete coordinate map were already known. The fallback absolute-position
route repeatedly reset the worker's raster context. Closely spaced drag requests
therefore repainted and replaced a complete bounded raster instead of reusing
coverage along the same surface.

The unchanged baseline diagnostic `followup-paint-json-diagnostic` separates this
from input dispatch: pointer dispatch p95 is approximately 0.06 ms, while repeated
backend paints take approximately 18–22 ms. Optional phase instrumentation shows
about 3.5 ms clearing and 6–9 ms glyph work per paint; the remaining time is not
attributed further. Instrumented diagnostics are not the performance comparison.

Only `midnight-viewer-ui` camera, presentation, their tests, and the loading status
binding change. There is no renderer-policy, engine, provider, paging, raster
budget, shaping, Unicode, or dependency change.

## Coordinate and pixel ownership

An uncovered target uses the existing raster `PanTo` request only when both
vertical document endpoints are known and the target axis has explicit bounds.
A partial page keeps the original coverage-only rule on both axes, including the
horizontal axis because its width can change on page refill. Unknown, nonfinite,
or out-of-bounds targets retain the existing fallback.

The camera stores requested and displayed coordinates separately. Old pixels stay
at their actual coordinates while the thumb reflects the requested target and
the status says “Loading requested position…”. A matching-surface reply that
covers the latest target can supply those pixels, including an older request's
reply when it already covers the newer position. It is displayed at the latest
requested coordinates, never at that older reply's requested origin. A reply
that covers neither the requested nor displayed coordinates is refused.

Newer seeks supersede pending coordinates on either axis. Cancellation, wheel
reversal and a new surface clear the old pending target; relative movement starts
from displayed pixels. Final settlement keeps the existing exact latest-sequence
acknowledgment and subpixel reconciliation. Pixel allocations remain owned by
frame retirement and Slint images; the camera retains metadata only.

## Focused regression evidence

Two production-source camera regressions fail on the original covered-only seek
behavior before the change. The final direct harness includes the production
camera and wheel controller and passes all **34 tests**. It covers late replies,
newer targets, cancellation, simultaneous horizontal/vertical pending intent,
reversal, new surfaces, source scroll maps, exact final settlement, and partial
page fallback. Raw RED/GREEN logs are `pending-camera-red.log` and
`pending-camera-final.log` under `_build/qa/d3d-scroll-20261112/`.

The final Cargo-built viewer artifact passes **76 tests**, selected by `raster_`,
`text_position` and `text_presentation`, with overlapping matches deduplicated.
The UI regression verifies that the desired thumb can move while displayed
coordinates and upload identity remain unchanged, that pending state clears only
with covered pixels, and that a covered subsequent seek reuses the upload. The
existing paging, cancellation, epoch, raster retirement and position tests remain
included. The artifact SHA-256 is
`0CF493963B1E30C569E8F05EFAB567A50C324A32E51D7A21BAB9B33C757873DB`.

Build receipts and per-artifact logs are in
`_build/qa/retained-seek-20261112/final-ui-build.jsonl` and
`final-ui-regressions/summary.json`. The reusable targeted runner uses one child
and six libtest threads. Its first launch without the build environment failed
before running any tests; the documented Windows loader environment was then
applied and the complete selected artifact passed. Strict Clippy for
`midnight-viewer-ui --all-targets -- -D warnings` also passes (32.06 seconds);
the existing vendored femtovg dependency emits two non-fatal unused-code warnings.
The log is `viewer-clippy.log`. All five changed source files use LF and pass
`git diff --check`.

## Matched foreground comparison

Both executables are preserved beside the same installed providers under
`C:/Users/tjelinek/MidnightExplorer-20261112-lifetime-dfbafca6/`:

| Variant | Executable | SHA-256 |
|---|---|---|
| Baseline | `scroll-pacing-candidate-3d7157c2.exe` | `3D7157C298C8B57B84793BA6CE3C9BA9F00E93B1A3686865A3D03216B2C3EA16` |
| Candidate | `scroll-pacing-pending-c93d9ec2.exe` | `C93D9EC2C39A9750BF6C37C31858A679035D796D88566EC3CF8773F850BAB598` |

The baseline already contains the earlier first-covered-redraw correction. The
candidate adds complete-page pending seeks. Later edits before final verification
are additional tests and formatting only. The two binaries use the same input
files, installed providers, Segoe UI 13, 1000×700 logical viewport at scale 2.25,
DX12 backend, initial position 0.1, and the original 60-move scrollbar sequence.
There is no GPU readback or optional paint-phase instrumentation in these runs.

All build owners reported idle before a 30-second quiet interval. All four
original probe guards pass, with unchanged assertions. Every exact-file-caption
sample is foreground, visible, unminimized and uncloaked: respectively 10, 10,
12 and 10 samples. Compiler lists before and after every case are empty. The
runner verifies its own process and caption before input, restores cursor and
foreground, and does not touch the lockscreen or preferences. Initialization
samples before the exact file caption remain in the raw metadata.

| Content measurement | JSON baseline | JSON candidate | XML baseline | XML candidate |
|---|---:|---:|---:|---:|
| Pointer moves | 60 | 60 | 60 | 60 |
| Presented pointer positions | 50 | 52 | 59 | 50 |
| Content updates during drag | 30 | 51 | 29 | 45 |
| Moving-content frames | 33 | 52 | 32 | 46 |
| Content gap median (ms) | 30.822 | 16.581 | 33.218 | 16.495 |
| Content gap p95 (ms) | 39.753 | 22.337 | 34.381 | 36.851 |
| Content gap maximum (ms) | 73.091 | 24.624 | 78.467 | 41.918 |
| Final content response (ms) | 71.149 | 16.425 | 85.378 | 16.805 |
| Raster replacements during motion | 28 | 6 | 30 | 9 |
| Final normalized position | 0.7 | 0.7 | 0.7 | 0.7 |

Raster replacements are the difference between the first and final moving
`raster_uploads` counters: JSON 3→31 versus 3→9, XML 2→32 versus 2→11. The final
settled frame has the same counter as the last moving frame. Repeated positions
using one pixel identity establish actual reuse, independently of scheduling
noise; metadata publications are not counted as new raster uploads.

These are single matched observations per file, not a statistical throughput or
universal frame-budget claim. XML p95 increases from 34.381 to 36.851 ms despite
its lower median and maximum. The original pointer and content measurements are
retained separately; no weaker derived gate replaces the original assertions.

Inputs and SHA-256 values, unchanged before/after:

- `D:/temp/ovclash/results/2026-09-07-mfg/manifest.json`:
  `CA5E4B26FEBA253568C17A62869471A2AEB2646D194593F18D2F9611263D21E7`.
- `C:/Windows/System32/WwanFeatureTests.xml`:
  `92396D10906CD3E606AB1598B69927AB895C7BEF2D6D0C1A1C613C579DA37C08`.

Raw reports, stderr and foreground metadata are
`_build/qa/d3d-scroll-20261112/pending-ab-{json,xml}-{baseline,candidate}.*`;
source hashes are `pending-ab-inputs-{before,after}.json`. The exact runner,
staging receipt and parsed comparison are in
`_build/qa/retained-seek-20261112/` as `run-drag-comparison.ps1`,
`candidate-staging.json` and `drag-comparison.json`.

## Wheel and partial-page qualification

The final candidate-only regression interval followed a second coordinated
30-second quiet period. At 11:02:12 UTC the read-only foreground observation
identified the ordinary ChatGPT window and no compiler processes. The existing
runner then launched and guarded only its own probe windows. BAT Text at 0.7 and
partial Markdown at 0.1 both pass the unchanged original assertions. All 41
exact-file-caption samples (29 BAT, 12 Markdown) are foreground, visible,
unminimized and uncloaked. Compiler lists remain empty and input hashes remain
unchanged. Previous foreground/cursor state is restored after each probe.

| Candidate regression | Original report |
|---|---|
| BAT down/down/reverse | 15 / 13 / 13 presented positions; first response 18.075 / 17.105 / 17.057 ms |
| BAT movement gaps | Median 16.676 / 16.646 / 16.841 ms; maximum 39.093 / 24.488 / 21.706 ms |
| Partial Markdown drag | 60 pointer moves, 59 presented positions, final normalized position 0.7 |
| Partial Markdown pointer gaps | Median 16.786 ms, p95 22.017 ms, maximum 22.866 ms |
| Partial Markdown content | 34 moving frames; median 33.019 ms, p95 34.438 ms, maximum 34.554 ms; final response 87.284 ms |

Markdown's original CPU crop check remains changed and nonblank. Its partial-page
fallback is intentionally unchanged and still produces roughly 33 ms content
intervals. BAT's measured 39.093 ms refill gap is lower than the earlier 56.804 ms
observation, but this candidate-only check is not a matched wheel optimization
comparison; no additional wheel speedup is attributed to the complete-page seek
change. Remaining refill, partial-page paint and native upload costs still exceed
one refresh period in some frames.

The two additional exact-file hashes are:

- `D:/dev/ov-physics-trunk/ovclash/build.bat`:
  `8EF5E6ACB593E5352778286868A8406B665CC403ABB7BDBB04FBD34870F0CED6`.
- `D:/dev/ov-physics-trunk/ovclash/AGENTS.md`:
  `6656EEA6CE179C31D7C2C1E08AB24443BF44AFC4FAE5DCC0E9E8462229D633AD`.

Raw files are `pending-final-bat-text-07.*` and `pending-final-md-drag.*` under
`_build/qa/d3d-scroll-20261112/`. The safe two-case runner, desktop observation,
before/after source hashes and parsed summary are under
`_build/qa/retained-seek-20261112/` as `run-final-cases.ps1`,
`final-desktop-condition.json`, `final-inputs-{before,after}.json` and
`final-regression-summary.json`. The candidate executable hash is unchanged.

## Final review: rounded boundary acknowledgment

A subsequent coordinator review found a separate settlement edge. A final
matching-sequence reply can round the requested origin slightly (within the
existing 0.25-pixel reconciliation tolerance), placing the acknowledged origin
inside coverage while the original desired coordinate lies just outside it. The
camera accepted those settled pixels but retained its pending desired position,
leaving the loading indicator active and generating another unnecessary seek.

The new regression exercises both boundary directions at +/-0.125 pixels. It
fails before the correction with "settled pixels must clear loading". Clearing
pending intent for either a covered seek acknowledgment or the already-validated
exact final acknowledgment fixes both cases. The direct production-source camera
and WheelMotion harness then passes **35/35 tests** using four test threads.
Logs and executables are `rounded-camera-{red,green}*` under
`_build/qa/retained-seek-20261112/`.

The foreground timings above remain measurements of the explicitly identified
candidate executable before this one-condition settlement correction. They are
not presented as a new measurement of the updated source. Final Cargo UI
integration qualification is recorded separately after rebuilding the changed
artifact.

The post-review Cargo artifact passes **77 viewer regressions**, selected by
`raster_`, `text_position` and `text_presentation`, with no failures or ignored
cases. Its SHA-256 is
`95F228F2A21F406ED376844C0EE13FBE634D1F283009AE40BA0D3A75AC4ED153`.
The same bounded runner also passes the `salamander_shortcuts` and
`selected_navigation` window targets concurrently (79 cases total, three
processes, six libtest threads). Exact receipts, artifact identities and logs are
in `_build/qa/retained-seek-20261112/rounded-cargo-and-shortcuts/summary.json`;
the successful build receipt is
`_build/qa/tree-user-commands-20261112/windows-final-build.jsonl`.
This final correctness gate includes the rounded-settlement fix. The earlier
foreground timings still describe the separately hashed measured candidate.
