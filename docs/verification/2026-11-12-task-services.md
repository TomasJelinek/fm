# Child-task service lifetime verification

Date: 12 November 2026. Follow-up to baseline `5388a97` and LIFE-01.

## Reproduced defect

On a current-thread Tokio runtime, retire the original service owner while a real
Find, preference or font actor still exists, then process actor cancellation.
All three original regressions failed: final service destruction ran on the
executor thread. `_build/qa/task-services-20261112/red.log` records three failures
in 0.00 s after a 61 s Windows test build. These tests do not mutate user files.

## Change and first targeted results

A separately reserved shared service owner now follows the EngineRun and the
migrated actors, including nested blocking closures. Their outputs are ordinary
metadata, preferences and font strings; captured provider outputs are outside the
scope of this helper. Guarded task envelopes preserve destruction order on Ready,
panic, cancellation and never-started native work. One additional startup slot is
used, independent of the checksum and Custom Command additions.

The first integrated Windows test binary passed:

- Eight service ownership tests in 0.02 s, including the three real actor regressions,
  completion/panic, uninvoked blocking closure and running native work surviving a
  bounded runtime shutdown (`green-2.log`).
- Eight existing public engine lifecycle cases in 0.34 s.
- Nine settings persistence cases in 0.04 s and four font cases in under 0.01 s.
- First Custom Command group: 14 passed in 0.59 s; first checksum group: three passed
  in under 0.01 s. Separate owner reports track the additional final gates.

Logs are under `_build/qa/task-services-20261112/`. The initial integrated compile
found a stale hand-built checksum fixture; it was migrated before the passing build.
No failing regression was disabled. The read-only
[ownership review](2026-11-12-task-services-review.md) found no actionable issue in
these migrated boundaries.

## Limits

This is not complete LIFE-01 closure. Desktop/watch/clipboard were open at this
checkpoint and are subsequently covered by the
[native-service batch](2026-11-12-native-service-retirement.md). Other independently
spawned work and repeated builder replacement remain separate gaps. The shared service guard does not protect raw newly captured
return values. Arbitrary native calls/destructors may still block; the application's
outer shutdown grace bounds the wait, not the provider implementation itself.

## Final qualification

On Windows the final integrated engine library suite passed 919 tests, with seven
existing ignored tests and no failures (6.74 s). Eleven directly affected integration
targets passed 162 tests: checksum actor/manifests/backend, Find actor/current-directory/
sorting-after-stop, and Custom Command actor/contracts/preparation/process/templates.
Path-included integration helpers overlap unit coverage; these are execution counts,
not distinct behavioral guarantees. The failed first run and corrected checksum
fixture are retained in the checksum evidence rather than presented as a clean run.

Windows strict engine all-target Clippy passed (21.35 s). Debug `midnight-explorer`
and `midnight-elevation` compiled in 1m13s. The only build warnings were the two
existing vendored FemtoVG unused-code warnings. The viewer keyboard and UI-polish
integration targets also passed four tests (0.43 s and 0.64 s). Their compact-PDF
fixture correction is documented [separately](2026-11-12-viewer-keyboard-fixture.md).

Native Linux under WSL passed the six affected library filters: checksum 58,
Custom Command 17, shared-service ownership 8, lifecycle 8, preferences 9 and fonts 4.
Five integration targets passed 141 tests: checksum actor 4, manifests 64, backend 67,
Find actor 5 and sorting-after-stop 1. Strict engine all-target Clippy passed (1m31s).
The Linux gate compiled the selected targets once and reused those exact fresh
binaries for each filtered run. Logs:
`_build/qa/checksum-lifetime-20261112/linux-green-*.log`.

Formatting of all 33 changed Rust files, LF endings and `git diff --check` passed.
The repository source-size check passed for 2,033 files, all at most 100,000 bytes.
No dependency, lockfile, platform policy or operation admission quota changed.

## Installed executable and startup

The rebuilt `D:/experiments/dev/FM/_build/debug/me.exe` has SHA-256
`0d73139064eb925a4971dd81eb1bc8ca045b80ed3bdabdd737e9f31f4ed6a088`.
It and the matching elevation helper were deployed with unchanged existing plugin
artifacts to `C:/Users/tjelinek/MidnightExplorer-20261112-owners-0d731390`; the private
runtime registration matches this executable hash. Deployment did not relax plugin
ACL validation. The temporary deployment stage was removed after registration.

Three fresh-executable smoke cases passed with exit 0 and empty stderr:
backend listing of Git/Git-bin, normal-renderer UI, and explicit software-renderer UI.
The UI cases displayed System32 and the repository, with both panels, columns,
command line and bottom buttons visible. Both captured images were inspected.
The real user-preferences hash was unchanged. Logs and images are under
`_build/qa/task-services-20261112/final-smoke-*` and `final-{native,software}.png`.

These smoke cases establish startup/rendering and basic backend listing, not a
complete interactive operation or viewer-performance sweep. Windows LockApp still
owns the foreground. [D3D timing qualification](2026-11-12-d3d-scroll-pacing.md)
remains open; this batch does not claim smooth foreground scrolling was measured.
