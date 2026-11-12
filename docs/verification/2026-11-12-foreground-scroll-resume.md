# Foreground scrolling and refill resume — 2026-11-12

The unlocked foreground baseline passes all ten original exact-file guards. The previous
200–550 ms DX12 waits are absent in these observations. A smaller BAT refill pause has a
separate, reproduced controller cause: restarting its pending wheel tail discarded the last
redraw timestamp and necessarily left the first newly covered redraw stationary. The narrow
candidate preserves that timestamp; qualification of its native timings is recorded below.
Uniform 60 Hz remains unproven.

## Valid foreground baseline

Repository at investigation start: `7251ec2`, `D:/experiments/dev/FM`. The preserved probe is
`scroll-pacing-baseline-abdaa338.exe` in the qualified lifetime runtime, SHA-256
`abdaa33884cc14a657ebb5024446e67beb85927fafc5c7ae3ff496eaceb2f85e`. Production viewer sources
are unchanged between the earlier `5388a97` evidence and this investigation; the intervening
viewer diff is a compact-mode integration test correction. See the
[earlier locked-session attribution](2026-11-12-d3d-scroll-pacing.md).

The initial read-only foreground check resolved to Claude, not LockApp. Windows still denied
`SetForegroundWindow`; the existing guarded click correctly refused a point covered by Claude.
These first attempts remain diagnostic. The final runner temporarily raises only its launched
viewer, verifies process/caption and `WindowFromPoint` ownership before clicking, removes its
topmost flag, then observes native foreground, visibility, minimization and DWM cloaking.
It restores the previous foreground after each case. No input is sent to another application's
window or to the lock screen. No renderer policy, plugin package, receipt or settings changed.

The `idle-fg-*` batch ran from 07:00:49 to about 07:02:11 UTC during the coordinator's reserved
Windows/Linux build-idle interval. Every file-caption sample is foreground, visible, not minimized,
and uncloaked (11–29 samples per case). Compiler process lists before and after each case are
empty. All ten cases exit zero with the existing guard assertions unchanged:

| Exact supplied file / action | Positions per notch | Median movement gaps (ms) | Largest reported gap (ms) |
|---|---|---|---|
| BAT Text 0.1, down/down/reverse | 14 / 13 / 13 | 15.962–16.921 | 25.850 |
| BAT Text 0.7, down/down/reverse | 15 / 13 / 13 | 16.428–16.867 | 45.348 |
| BAT Text EOF, up/up/reverse | 12 / 13 / 13 | 16.258–16.828 | 18.984 |
| BAT Source 0.1, down/down/reverse | 13 / 13 / 13 | 16.445–16.479 | 24.194 |
| Markdown 0.1, down/down/reverse | 13 / 13 / 13 | 16.369–16.704 | 25.865 |
| JSON 0.1, down/down/reverse | 14 / 13 / 13 | 15.837–16.509 | 73.202 |
| XML 0.1, down/down/reverse | 14 / 13 / 13 | 16.339–16.731 | 72.166 |

Markdown, JSON and XML absolute drags each deliver 50–53 presented positions for 60 pointer
moves and finish at 0.7. Their p95 gaps are 20.835–23.811 ms. The original wheel statistics
above are preserved: JSON/XML's largest gap is the final exact-raster repaint after motion
has stopped, with unchanged pixel offset and a roughly 1.5e-8 normalized float adjustment.
It is not evidence of a 73 ms moving-content stall. No secondary calculation replaces the guard.

Exact paths are `D:/dev/ov-physics-trunk/ovclash/build.bat`, that directory's `AGENTS.md`,
`D:/temp/ovclash/results/2026-09-07-mfg/manifest.json`, and
`C:/Windows/System32/WwanFeatureTests.xml`. Geometry remains 1000×700 logical at scale 2.25,
Segoe UI 13, DX12, with spatial/GPU readback disabled. Raw reports, source hashes, process
metadata and `idle-foreground-summary.json` are under `_build/qa/d3d-scroll-20261112/`.

## Reproduced refill cause and bounded correction

In `idle-fg-bat-text-07.txt`, content reaches its coverage boundary at 3282.206 ms. The next
redraw at 3293.791–3294.941 remains there. Refill pixels publish at 3305.014, but the next
redraw at 3309.767–3321.582 still has the same position. Motion only resumes at 3327.565.
This locates an unnecessary stationary redraw after new coverage arrives, independently of
backend refill duration and upload cost.

`Camera::tick` canceled its wheel motion and pushed the residual at `now`. The immediately
following tick therefore had zero elapsed time. `WheelMotion::restart_remaining` now rebases
the residual distance while retaining the last actual redraw timestamp. The existing 16 ms
maximum elapsed increment still bounds a delayed frame. No input is invented, no pixels are
skipped, and direction, exact final distance, cancellation and direct touchpad/drag handling
keep their existing routes. No page, raster, cache, memory budget or Unicode shaping changed.

The new first-redraw regression fails against the original implementation with
“the first covered redraw must move.” With the correction, all 28 camera and wheel-motion
tests pass in a direct Rust harness that includes the production source and links the existing
production core library. It covers both directions, 1×/2.25× scale, short and long waits with
and without redraws, no duplicate movement at one timestamp, bounded initial advancement,
and exact endpoints. Existing reversal, cancellation, drag and delayed-frame tests also pass.
Logs: `refill-camera-red.log`, `refill-camera-green.log`, `refill-camera-final.log`.

## Actual application qualification support

The normal app resolves its preferences through the native known-folder service, so overriding
`LOCALAPPDATA` does not isolate writes. Existing `--ui-smoke-test` already uses in-memory
preferences. Its dwell can now be requested through child-only
`MIDNIGHT_UI_SMOKE_DWELL_MS`, only within that smoke branch: default/minimum 350 ms, maximum
15 seconds, malformed/overflow values fall back to 350 ms. The existing 30-second overall
smoke timeout remains. Normal startup/settings behavior is unchanged.

The standalone duration test fails against the old fixed delay and passes after the bounded
parser is added (11 default, malformed, range and overflow cases). Logs: `smoke-timing-red.log`
and `smoke-timing-green.log`. The prepared `run-actual-app.ps1` uses exact file selection/F3,
owned foreground wheel/reversal, owned-window captures and Escape, checks the native preference
hash before/after, and restores cursor/foreground. Those screenshots are functional evidence,
not a native frame-pacing measurement.

## Final build-idle foreground qualification

After both Windows and Linux test/build owners reported idle, the machine had a
30-second quiet interval. The read-only check at 10:16:05 UTC found an ordinary
unlocked foreground application and no compiler processes. No lockscreen was
changed. The 11-case sequence ran at approximately 10:16:20–10:17:47 UTC: one fresh
preserved-baseline BAT Text 0.7 run followed by the matched candidate and all ten
candidate wheel/drag cases. **All 11 original probe guards passed.** The four exact
supplied input hashes are unchanged. Every one of the 258 samples carrying the
exact file caption was foreground, visible, unminimized and uncloaked; the earlier
untitled/default-caption initialization samples are retained in the raw metadata.
All per-case compiler lists before/after are empty.

Both probes were staged beside the same installed providers in
`C:/Users/tjelinek/MidnightExplorer-20261112-lifetime-dfbafca6/`:

- Preserved baseline SHA-256:
  `ABDAA33884CC14A657EBB5024446E67BEB85927FAFC5C7AE3FF496EACEB2F85E`.
- Final candidate SHA-256:
  `3D7157C298C8B57B84793BA6CE3C9BA9F00E93B1A3686865A3D03216B2C3EA16`.

The matched BAT 0.7 first-notch trace reproduces the old stationary redraw after
refill publication. Baseline publication at 4282.442 ms is followed by a stationary
redraw at 4323.699 ms; motion resumes at 4325.006 ms. Candidate publication at
4014.494 ms is followed by a moving redraw at 4042.911 ms. Publication-to-motion is
42.564 versus 28.417 ms, and the original first-notch maximum gap is 73.177 versus
56.804 ms. This supports the specific first-covered-redraw correction. It is one
matched observation, not a statistical speedup or a guarantee that refill/upload
work fits one refresh period. Both retain 15/13/13 positions for down/down/reverse.

Final candidate values retain the original probe's statistics and gates:

| Exact supplied file / action | Positions per notch | Median movement gaps (ms) | Largest reported gap (ms) |
|---|---|---|---|
| BAT Text 0.1 | 13 / 13 / 13 | 16.191–16.647 | 22.685 |
| BAT Text 0.7 | 15 / 13 / 13 | 16.142–16.747 | 56.804 |
| BAT Text EOF, up/up/reverse | 12 / 13 / 13 | 16.370–16.419 | 17.784 |
| BAT Source 0.1 | 13 / 13 / 13 | 16.271–16.545 | 23.435 |
| Markdown 0.1 | 13 / 13 / 13 | 16.396–16.544 | 21.838 |
| JSON 0.1 | 14 / 13 / 14 | 16.177–16.292 | 68.298 |
| XML 0.1 | 14 / 13 / 13 | 16.081–16.844 | 64.515 |

JSON/XML's largest reported wheel gap again precedes the settled exact-raster
repaint: pixel offsets remain exactly -112.0000/-129.7778 logical pixels, while
normalized positions differ by only 1.5e-8. Those values remain in the unmodified
statistics. BAT 0.7's 56.804 ms gap does include resumed content movement across a
coverage refill; it is a remaining measurable cost. EOF's first response is
38.294 ms even though its subsequent movement gaps are lower.

| Absolute drag | Positions / 60 moves | Pointer position p95 gap (ms) | Content p95 / maximum gap (ms) | Final content response (ms) |
|---|---|---|---|---|
| Markdown | 48 | 16.822 | 33.885 / 35.430 | 97.568 |
| JSON | 54 | 25.515 | 37.344 / 105.554 | 118.239 |
| XML | 49 | 20.289 | 33.802 / 71.668 | 82.864 |

All drags finish at normalized position 0.7, and the existing CPU crop checks report
changed, nonblank retained content. Pointer presentation and content publication
are reported separately. These remaining refill/drag gaps mean uniform 60 Hz
content presentation is **not** established. There is no physical display/GPU
readback proof in these timing runs, and no renderer-policy change is justified by
this evidence. Paging, raster bounds and Unicode handling retain their existing
implementation and guards.

Raw evidence is under `_build/qa/d3d-scroll-20261112/`: `final-fg-*.txt`, matching
`.metadata.json` files, `final-foreground-summary.json`, before/after input hashes,
`final-desktop-condition.json` and `final-candidate-staging.json`. The existing
`analyze-refill.ps1` produces the publication/first-redraw comparisons above; it
adds causal detail without replacing any guard.

## Fresh application and physical-key evidence

The deployed application was
`C:/Users/tjelinek/MidnightExplorer-20261112-user-menu-d5cf41d7/me.exe`, SHA-256
`D5CF41D7F8A7529E08A0E4A94D3FBFD5229888FEA5A81C488DF49BC37386BA17`.
Three separate bounded in-memory smoke instances all exit 0, emit zero stderr
bytes, and leave the native preferences hash unchanged. Only verified owned
foreground windows receive input; foreground/cursor state is restored afterward.

- BAT: native Alt+2, Alt+5 and Alt+3 captures visibly show the active left panel
  changing Brief → Thumbnails → Detailed while the right panel stays Detailed.
  Exact filename selection/F3 opens the supplied `build.bat` in Source mode;
  captured content scrolls down and returns to the first line after reversal.
- XML: F3 opens the exact `C:/Windows/System32/WwanFeatureTests.xml` in the fresh
  installed XML viewer, with visible XML syntax coloring in Source mode. Down and
  reverse captures show movement and return to the first line; Escape closes it.
- Controlled navigation: an owned directory contains only `a.txt`, `b.txt` and
  `c.txt`, with distinct A/B/C content and Unicode `文档`. Exact caption checks
  acknowledge Space a→b, Backspace b→a, Backspace wrap a→c, Alt+Right wrap c→a,
  Alt+Right a→b and Alt+Left b→a. Later captures visibly show settled C, B and A
  contents. The first Space capture occurs while the new caption is already set
  but the prior A raster is still explicitly marked `Opening…`; it proves routing,
  not settled B pixels. This transition is retained, not relabeled as completion.
  Fixture hashes are unchanged after the run.

Results/captures are `final-app-{bat,xml,cycle}.*` and corresponding PNG files in
the same QA directory. Screenshot inspection is functional evidence only; it is
separate from the timing matrix. No additional desktop input is needed to infer
routing from an unknown user directory or to alter saved settings.
