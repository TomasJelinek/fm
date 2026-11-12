# Native desktop, watcher and clipboard retirement qualification

Date: 12 November 2026. Baseline: `b9f8ee4` on `codex/viewer-plugins`.
This is one completed LIFE-01 implementation batch, not completion of the full
application goal or a claim that every engine actor has been audited.

## Change and reproduced failures

Desktop asynchronous/native task pairs retain the common service lifetime. Returned
terminal/editor/opener children are guarded before ID queries or result delivery.
Eight shared terminal/editor/Custom Command slots and sixteen separate opener slots
remain bounded through actual native cleanup. Captured editor targets keep one owner
from intake through rejection or execution. No live child is implicitly killed.

Both watcher panes guard their shared state, including captured inline-tree mounts.
Subscriptions remain local to their native worker. Clipboard guards its private inbox
before accepting requests, retires rejected captures independently of Tokio, and
cancels a started pump on Controller Drop, including result backpressure. Sessions
still open, pump and close on one thread. The startup domain now reserves 153 payload
slots and six waiters; the two process trackers are separate from this ledger.

Focused baseline runs reproduced two desktop lifetime failures, three watch lifetime
failures and two clipboard failures. The clipboard queued-worker cancellation probe
already passed; it is additional coverage rather than a claimed baseline defect.
See [desktop evidence](2026-11-12-desktop-ownership.md),
[watch evidence](2026-11-12-watch-retirement.md) and
[clipboard evidence](2026-11-12-clipboard-retirement.md) for gates and exact logs.
Tests use owned fixture resources; no user files were deleted or modified.

## Stable implementation qualification

Windows used the project toolchain and the existing worker SDK registry configuration,
with isolated test TEMP/TMP. The final affected gate produced:

- Engine library: **935 passed, seven existing ignored, zero failed**, 6.83s.
- Eight directly affected integrations: **30 passed**, including native watch events.
  Targets: checksum_actor, clipboard, desktop, editor, find_actor, user_command_actor,
  user_command_process and watch_native. Build: 27.18s.
- Strict engine all-target Clippy: passed, 19.29s.
- Debug application and elevation helper build: passed, 1m15s. Two existing vendored
  FemtoVG warnings remain; no new dependency or workflow was introduced.

After that gate, one test-only completion wait was made independent of whether its
cleanup slot or native launch permits become observable first. Assertions while Drop
is blocked remain unchanged. A fresh Windows binary reran that exact case: **1 passed
in 0.02s**, compile 23.64s. No production change followed the final Windows suite.

Linux built the final source, including that corrected fixture, once in 1m33s. The
six library filters executed 124 passing cases: desktop 16, clipboard 35, watch 39,
user_command 18, engine_lifecycle 8 and task_service_tests 8. The same eight integration
targets passed all 30 cases. Strict engine all-target Clippy passed in 1m32s. This was
an affected Linux gate, not a full Linux unit suite. Detailed timings and executable
identities are in the watch evidence above.

[Independent review](2026-11-12-remaining-actor-peer-review.md) found no additional
actionable defect in these changed boundaries. It is source review, not another test
run. The full-channel desktop cancellation test establishes cancellation after launch;
it does not instrument the exact point at which Send becomes Pending.

Rustfmt passed for all 20 changed Rust files. The source-size gate checked 2037 files,
all within 100000 bytes. Changed-source/document LF and Git whitespace checks passed.

## Debug deployment and startup

The new `D:/experiments/dev/FM/_build/debug/me.exe` SHA-256 is:

`2da12aa577aef0b8903af3059f839a2fc5a7badc10875d58856be8a231afb003`

It and the matching elevation helper were deployed with unchanged plugin binaries to
`C:/Users/tjelinek/MidnightExplorer-20261112-native-2da12aa5`, with a registration matching
this exact executable. Only the owned deployment staging copy was removed afterward,
reclaiming 0.531 GiB; existing private installations and user content were retained.

The newly built executable passed backend startup on Git/bin, ordinary UI startup on
System32/repository panels, and the same UI smoke with forced software rendering. All
three exited 0 with empty stderr. Both rendered snapshots were visually inspected;
panels, drive bars, command line and bottom actions appeared. User preferences were
unchanged. These smoke checks do not establish foreground drag/scroll latency or
interactive performance on the locked desktop.

Evidence: `_build/qa/native-services-20261112/`, including `windows-final.ps1`,
`windows-final-tests.log`, `windows-clippy.log`, `debug-build.log`,
`editor-fixture-final.log`, `deployment.log`, `me.sha256`, `runtime-path.txt`,
`run-final-smoke.ps1`, `final-smoke-*.log` and `final-{native,software}.png`.
Generated evidence/build artifacts are intentionally excluded from Git.

## Remaining work

The [remaining actor inventory](2026-11-12-remaining-actor-ownership-inventory.md)
keeps File List captures/spools, dynamic network/archive ownership, PathPicker shutdown,
other service task pairs and repeated builder replacement open. Existing protections
at those boundaries are recorded individually. There is no blanket LIFE-01 closure,
no cancellation-latency promise for an arbitrary blocked provider destructor, and no
claim that unrelated UI/performance backlog items are complete.
