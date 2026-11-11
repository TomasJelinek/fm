# Integrated follow-up — 12 November 2026

This batch follows qualified commit `a6d90f3`. Copy, Move and Delete continue to
share one `JobProgressWindow`. X/Escape requests cancellation of its exact active
operation; Jobs remains a separate retained ledger. The follow-up adds truthful
sampled-Copy/cross-volume-Move ETA, a measured HTML frame-pacing improvement,
default-off image startup diagnostics and a compact Find test correction.

## Qualification

- Full affected Windows unit suites: core 61 passed; engine 833 passed / 7 ignored;
  UI 893 passed / 4 ignored. Zero failures and zero filtered cases. The ignored
  cases are existing opt-in checks. See the [ETA report](2026-11-12-transfer-eta.md)
  for exact binaries, test-body durations, remaining-work invariants and evidence.
- Final Windows Find integration: 6 passed. Shared operation-progress integration:
  2 passed, including unknown totals, Delete's shared chart, per-file Copy progress,
  all tested themes and compact 320 × 200 layout.
- Strict all-target Clippy passed for the stable application/UI/viewer/platform/
  elevation package union plus core and engine; the final Find fixture was linted
  again after its correction. Two existing vendored femtovg warnings remain.
- Independent HTML provider: 13 tests, strict all-target Clippy, formatting and
  six native before/after samples passed. See [frame pacing](2026-11-12-html-frame-pacing.md).
- Image startup diagnostics: 29 Windows checks, targeted all-target Clippy and
  ordinary/enabled Linux library checks passed. Instrumentation compiles out by
  default and did not change production startup or sandbox policy. The cold-start
  delay is attributed to native process creation, not claimed resolved; see the
  [image attribution report](2026-11-11-image-loading.md).
- Workspace formatting, whitespace validation and the 100,000-byte handwritten
  source limit passed (1,997 checked source files).

The fresh debug application SHA-256 is
`17E2147A51E932F6D9713E8AD01AAC0D846186E8AD68A60EC4DDCC32D1268B79`.
`_build/debug/me.exe` is registered to the new private runtime
`%USERPROFILE%/MidnightExplorer-20261112-eta-17e2147a/`, containing the matching
helper, filesystem/viewer/thumbnailer packages and the qualified HTML DLL.
Backend, native-renderer and explicit Slint-software startup smoke cases each
exited zero with empty stderr and unchanged user preferences. Both UI snapshots
were visually inspected. Slint-software selection is not a test with physical GPU
hardware disabled in Chromium.

Local integration evidence is `_build/qa/followup-20261112/`: build JSON/stderr,
all-target and final Find Clippy logs, final UI tests, deployment/registration log,
three smoke logs and both screenshots. No build artifacts are committed.

## CI regression and remaining status

GitHub run `43117240558`, for `a6d90f3`, failed the Ubuntu compact Find lifecycle
case with `Missing Copy`. Native Linux reproduction failed identically: the
Details button's center was x=229 while its scrollable clip ended at x=228.
Clicking that clipped point did not open Details, so Copy could not exist.

Two fixture clicks now use the existing helper to scroll an action into view and
send a real pointer click, preserving the fixed Close area. No product behavior or
assertion was weakened. The complete Linux Find target then passed 6/6 in 1.46 s;
Windows passed 6/6 in 1.37 s. Original CI and failing geometry logs are retained
under `_build/qa/viewer-settings-20261111/` alongside `ci-find-linux-fixed.log`.

At this report's creation the old run's Windows Test step was still running, and a
new hosted run had not yet qualified the follow-up. The locally corrected Linux
case does not mean the entire hosted pipeline is green. HTML residual latency,
variable cold native image-process creation and other open design work remain
tracked separately; the broad implementation/audit goal is not complete.
