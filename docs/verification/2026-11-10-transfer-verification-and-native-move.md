# Native Move and selectable transfer verification

Current integration evidence, 2026-11-10. The policy is defined in the
[accepted design](../superpowers/specs/2026-11-10-transfer-verification-and-native-move.md).
The dated checkpoints below do not claim that every broader project feature has
been qualified. The subsequent [staging and cancellation qualification](2026-11-10-staging-cleanup.md)
records the latest shared/native/UI regression results and executable status.

- Shared transfer tests passed after the three-block refinement, including
  boundary corruption, exact small/empty files, unaligned streaming, truncation,
  sync/source-change/cancel failures and retried short writes/readback failures.
  Late proof guards and middle/tail retry regressions also pass. The final Linux
  common-package gate passed 174 tests across 27 binaries and strict Clippy.
- Settings round trips: quick-mode persistence, legacy default full SHA-256,
  explicit malformed/missing-field refusal passed alongside four existing panel
  preference tests. Settings schema28 is needed only for explicit quick mode.
- Windows core/engine:51 +729 tests passed,7 existing opt-in tests ignored.
  Real-queue tests cover native rename, whole directories, explicit overwrite,
  late cancellation, no content reads, same-batch protection and sampled modes
  for path/selection/captured/clipboard/archive routes.
- Windows platform:158 library tests passed,9 ignored;5 permission tests passed,
  including moving files without content-read permission and directories without
  listing rights. Strict all-target Clippy passed; an additional actual D: to C:
  volume classification test passed using exclusively owned temporary fixtures.
- Linux platform:168 library +26 integration tests passed,14 existing opt-in
  tests ignored. Strict all-target Clippy passed. Actual post-commit sync errors
  now retain the committed destination and recovery receipt, attempt all three
  directory syncs and cannot replay Copy. This was preceded by two failing tests
  showing swallowed fsync errors and short-circuited sync attempts.
- The broader Linux engine run exposed a genuine same-batch overwrite-protection
  gap for native renamed children. The engine now protects their identities just
  like verified copies; the later Windows all-library run includes this fix.

The first integrated UI compilation exposed a preferences fixture missing the new
field. It is corrected. The final UI library (786 tests), strict all-target UI
Clippy, Windows debug executable, protected deployment and native startup/UI
smokes have since passed; see the [final artifact record](2026-11-10-staging-cleanup.md#debug-executable-and-startup).

Artifacts stay under `_build/qa/operation-speed/`, `_build/qa/operation-speed-charts/`,
`_build/qa/native-move/` and `_build/qa/native-fast-move/`. All destructive tests use
owned temporary fixtures. No third-party dependency was added or changed.

## Subsequent Windows drag/drop backend qualification

The Windows platform library now passes **162 tests with 11 opt-in tests ignored**,
and strict all-target Clippy passes. The separately invoked real Shell tests pass
same-volume default Move/Ctrl-Copy and cross-volume default Copy/Shift-Move for
files and nested directories. The real owned-window input-thread/worker-STA
handoff passes success/cancel/success. Exact evidence and limits are in the
[dated drag/drop report](file-drag-drop.md#2026-11-10-folder-targets-and-windows-shell-move).

Internal automatic drops use actual backend volume identity, choosing Copy when
evidence is unknown or mixed. Windows external moves are performed by the Shell
receiver; its feedback never authorizes deletion by this application. Linux
outgoing X11/XWayland remains Copy-only, and native Wayland external dragging
remains unavailable. Integrated component tests and Windows debug deployment are
complete; the separate final-app-to-Explorer physical-input workflow remains a
native qualification item.

Native same-volume rename performs no CRC, SHA or content reads. Copy and
cross-volume Move retain full SHA-256 as the default; explicit quick mode compares
size and independent first/middle/last 64 KiB samples, or all bytes up to 192 KiB.
Quick mode can miss corruption between samples and does not replace identity,
publication, cancellation or source-removal guards. No BLAKE mode or dependency
change is part of this update.
