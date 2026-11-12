# Native tree Delete and session history — 12 November 2026

This follow-up builds on `35227750`. Copy, Move, Trash and permanent Delete continue
using the same operation progress window. The added path is typed deletion from
expanded native panel branches; no second delete-progress component was introduced.
The independent Jobs window retains session history, including metadata details
that previously disappeared after the sixteenth result.

## Boundaries and independent review

The UI captures the acknowledged projection/selection and immutable Delete mode,
validates the returned request/stamp/count, and submits an opaque native capture.
The backend preflights the entire selection before removing redundant descendants,
pins native mutation targets, and enters ordinary confirmation/FIFO cancellation.
No displayed path becomes native operation authority. Archive targets remain
read-only; there is no fallback to a temporary extraction or permanent deletion.

Two reviewers checked the frontend retirement/admission paths and backend
capture/preparation/controller lifetime. They found no remaining issue in normal capture/admission after
review corrections. A separate existing forced-shutdown ownership gap is recorded
as LIFE-01 in the [backlog](../IMPLEMENTATION_BACKLOG.md); it is not claimed fixed. An initially suspected link-ancestry overlap was not a live
reproduction: both tree expansion and projection validation reject directory-link
expansion today. A defensive ancestry rule still rejects overlapping roots across
such a boundary. It does not claim to freeze descendants between capture and
execution. Existing no-follow deletion preserves links' targets and moved-away
subtrees; see the [tree contract](../developer/TREE_PANEL.md).

Metadata results now use operation-ID lookup and have session lifetime. Each
result still retains at most 32 sampled outcomes. Replacement, explicit clearing,
late forgotten receipts and shutdown defer payload reclamation to backend workers.
No result-map ordering is used for presentation.

## Targeted evidence

- `tree-delete-20261112/ui-green.log`: eight new presenter tests passed in 0.02 s.
  Coverage includes both modes, acknowledged selection, delayed/duplicate/invalid
  receipts, rejected queue admission, ID exhaustion, cancellation and reconciliation.
- `job-history-20261112/red-direct.log`: the previous 16-result eviction fails the
  regression. The earlier Cargo invocation selected zero tests and is not evidence.
- `job-history-20261112/final-green.log`: both new history/lifetime tests passed;
  related history checks passed 4/4 plus one existing ignored benchmark.
- `job-history-20261112/cache-green.log`: all four derived-row/cache tests passed.
  Ten existing operation-presenter checks also passed.

Evidence paths above are relative to `_build/qa/`. Backend/native test results,
integrated lint/build and deployed-binary qualification are appended only after
those checks finish. This report does not claim broad-goal completion.

## Validation environment correction

The first full Windows engine suite passed 843 tests and failed 13 diagnostic
storage assertions (7 opt-in tests ignored). All failures reported access denied
while writing private warning reports. `_build/tmp/MidnightExplorer-size-warnings`
was owned by `CodexSandboxOffline`, whereas the authorized runner used `tjelinek`.
The spool's existing restrictive ACL correctly refused another identity.

The unchanged engine binary reproduced one failure with the old TEMP and passed
all 13 exact cases with a fresh same-user per-run TEMP. No ACL, security policy,
product behavior or assertion changed. `diagnostic-storage-proof.json` and
`diagnostic-storage-all-green.{json,log}` retain the evidence. Final qualification
uses the unique directory recorded in `validation-temp.txt`, after loading the
usual build environment. The old protected directory remains untouched.

## Full affected Windows units

The final same-identity TEMP run (`full-affected-units-green.log`) passed all
**2,335** enabled tests, with **27** existing opt-in/environment tests ignored,
zero failures and zero filtered cases:

| Crate | Passed | Ignored | Test-body time |
| --- | ---: | ---: | ---: |
| core | 61 | 0 | 0.08 s |
| elevation | 7 | 0 | 0.03 s |
| engine | 856 | 7 | 6.52 s |
| platform-windows | 174 | 12 | 6.47 s |
| ui | 903 | 4 | 2.91 s |
| ui-input | 29 | 0 | <0.01 s |
| viewer-engine | 184 | 4 | 0.34 s |
| viewer-render | 9 | 0 | 0.14 s |
| viewer-ui | 112 | 0 | 0.97 s |

These are full unit suites for the affected package union, not every independent
plugin/workspace integration target. The common retirement helper now has one
crate-level module imported by Copy and mutation operations; no duplicate module
instantiation or behavioral change is needed. The Windows results above include
that final module wiring.

## Integration, lint and rendering checks

`final-integrations.log` passed 26/26: shared progress 2, Jobs 1, native operation
windows 10, real UI/native filesystem operations 4, Windows monospace catalog 4,
raster 4 and source continuity 1. Actual Copy/Move/Delete and folder drag use only
owned fixtures. The Delete progress screenshot (`screenshots/delete-progress-640x420.bmp`)
was visually inspected: title/icon, discovery, chart, ETA and cancellation controls
are visible and aligned. It is the existing shared component, not a separate layout.

Strict all-target Windows Clippy passed for the same affected package union
(`final-clippy.log`). It retains two pre-existing vendored femtovg warnings. The
first all-target run found duplicate inclusion of the native link fixture; one
cfg-selected root module now serves both Delete and directory-size tests. Both
consumers were requalified (23 + 3 tests in `shared-fixture-{delete,sizes}.log`).
No lint suppression, behavior change or weakened assertion was introduced.

The final debug build of `me.exe`, the elevation helper and `performance_probe`
completed successfully; build messages are retained in `debug-build.{jsonl,stderr}`.
The [font catalog correction](2026-11-12-monospace-catalog.md),
[CI cleanup correction](2026-11-12-ci-artifact-retirement.md) and
[measured glyph optimization](2026-11-12-glyph-compositing.md) are integrated here.

## Installed debug build and startup

Final app: `_build/debug/me.exe`, SHA-256
`07f4fcc14373c87cf677ec800a50217e8078f196d674b6704c2ad7f63fdfde12`.
The verified private plugin runtime is
`C:/Users/tjelinek/MidnightExplorer-20261112-tree-delete-07f4fcc1`, registered
against this exact app hash. The previously qualified plugin payloads were copied
from the existing private runtime, alongside the newly built app and elevation
helper. Deployment verifies hashes and restrictive ownership; it does not weaken
plugin path checks or replace the previous installation.

`run-final-smoke.ps1` passed all three app checks: backend startup with Git/bin,
D3D UI startup with System32 and the repository, and the same UI startup with the
software renderer. Each exited zero with empty stderr. The user's preferences
hash was unchanged. `final-native.png` and `final-software.png` were visually
inspected: both panels, paths, toolbar, command line and F-key row are populated,
with no startup error panel. These startup checks do not establish all viewer
scrolling performance; the separate glyph report records that qualification.

The validated deployment staging copy under this QA directory was removed after
startup qualification, recovering 570,753,525 bytes (0.532 GiB). Its exact path,
non-reparse entries and ownership marker were checked before deletion. Build
outputs, installed runtime and QA evidence remain; the foreign-owned diagnostic
spool was not touched. See `deploy-staging-cleanup.json`.

## Final viewer probe qualification

The final app hash above is unchanged by the subsequent probe-only diagnostics.
The 17-case native actual-file batch passed 15 numeric gates. BAT Text mode
failed the normalized-position fraction check with both renderers. Bounded pixel
sampling independently showed 13 forward/reverse moves per notch with about
135 raster pixels total and about 20.8 pixels maximum per frame, including the
page replacements where the byte-position estimate changed in the wrong direction.
The original failures remain in the evidence; this is not reported as 17/17 green.

The estimate also feeds the scrollbar thumb, so its smaller visible position
correction remains a follow-up (SCROLL-01). Source-pixel sampling is diagnostic,
not GPU readback or uncontaminated timing. All four directly affected probe tests
and scoped strict Clippy passed after the diagnostics were added. See the
[glyph report](2026-11-12-glyph-compositing.md) for exact artifacts and limitations.
