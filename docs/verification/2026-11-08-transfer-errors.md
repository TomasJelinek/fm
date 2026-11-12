# Copy/Move error decisions and automatic progress

Scope: explicit I/O failure recovery and the user-confirmed distinction between
automatic transfer progress and manually opened Jobs. This report does not claim
that every possible device, filesystem or power-loss outcome was reproduced.

## Behavior and review

- Failure events carry job/version, phase, paths and actual Retry/Skip capabilities.
  Error Skip All is separate from overwrite policies and is limited to one job,
  phase, error kind and operation. Delayed or disallowed decisions cannot advance
  another failure or grant overwrite consent.
- Retained preparation identities survive retry. A partial failed write replays
  the pending block from the committed offset. Read/sync/readback/publication
  retries retain complete SHA-256 verification and refuse changed objects.
- Cancellation while paused closes owned staging resources before releasing the
  mutation queue; it preserves originals and lets the next queued job proceed.
- Failed Move source removal keeps its destination and reacquires a fresh native
  finalizer, rehashing source and target before another deletion attempt. Skip
  restores only the failed source and retains its ancestors. Unexpected namespace
  winners and failed restoration retain both objects and report residuals.
- Completed directory enumeration is checkpointed. Removal Retry does not ask
  an exhausted iterator for another child. Enumeration errors remain explicit
  non-retryable decisions, rather than being mistaken for EOF.
- Successful automatic progress closes; manual Jobs stays open. History is not
  cleared by closure. It retains the last 16 completed jobs and is
  not persisted across restart. Failed/cancelled session outcomes remain known
  even after older rows leave that bounded history.
- Accepted UI cancellation is monotonic: delayed Running/Waiting snapshots and
  newer failure/conflict events cannot reopen a cancelled job's prompt.

Independent review found and corrected directory re-enumeration on removal Retry,
Move proof-read errors sharing deletion's Skip All scope, loss of automatic window
ownership after history pruning, and late prompts after cancellation. A second UI
review found the ownership/cancellation fixes consistent with batched events.
The [Salamander audit](2026-11-08-transfer-error-reference-audit.md) lists source
references and explains why weaker legacy retry/overwrite behavior was not copied.

## Backend gates

All Cargo invocations used pinned local caches, `--offline`, debug information off
and incremental compilation off. Windows builds use `_build`; Linux builds use
`_build/linux`. WSL native fixtures use the existing
`/tmp/fm-interaction/_build/tmp` runner on ext storage. A first attempt on the
Windows-mounted DrvFS fixture path failed native capability/raw-name tests; this
is not a supported substitute for the ext fixture run.

| Gate | Result |
| --- | --- |
| Windows common library, `-p midnight-platform-common --lib` | 66 passed |
| Windows engine, `-p midnight-engine --lib copy_operations` | 90 passed |
| Windows native, `-p midnight-platform-windows --test transfers` | 14 passed |
| Linux engine, `-p midnight-engine --lib copy_operations` | 82 passed |
| Linux native, `-p midnight-platform-linux --test transfers` | 17 passed |
| UI presenter library, `-p midnight-ui -p midnight-explorer --lib copy` | 45 passed |
| UI callbacks/layout: `copy_errors`, `copy_dialog`, `jobs_dialog`, `file_actions_ui`, `dialog_layout`, `dialog_drag` | 31 passed |

Fault injection covers a partial disk-full write, a failure after an already
committed 256 KiB block, source read cursor movement on error, readback error after a
verified prefix, flush/sync/seek/length errors, failed publication, replacement
preparation, stale consent, source/target identity changes and source removal.
These are deterministic bounded fixtures, not destructive full-disk/disconnect
experiments on the user's devices. Native tests exercise real held handles,
publication, namespace conflicts and cleanup on Windows NTFS and Linux ext.

The initial missing-destination regression failed before recovery was implemented:
the old job terminated instead of waiting for a decision. The directory-removal
regression also failed causally before the EOF checkpoint, then passed. Older
changed-source/target tests now explicitly choose Cancel at the new decision;
separate tests verify that explicit Skip continues remaining siblings.

Exact logs are `_build/qa/transfer-errors/{common,engine,windows,linux-native-ext,
linux-engine,ui-lib}.log`. An earlier broader common integration run by the review
agent also found an unrelated stale default-command-count expectation in
`tests/user_commands_settings.rs:43` (expected 4, actual 5); the full repository test
suite is therefore not claimed green by this report.

## UI and runnable build

The callback suite covers 24 queued successes with the owner pruned before one
final refresh, sticky failed/cancelled outcomes after history trimming, manual
ownership, refused decision delivery, deferred Resolve, stale versions and real
Retry pointer input at 320×200. Normal and compact software-rendered snapshots
were inspected. Error Retry/Skip actions stay outside the diagnostic scroller;
their own row scrolls horizontally when needed. Existing conflict/merge/name
controls, dialog dragging and Jobs selection/cancellation regressions also pass.

The final debug build succeeds. It retains existing warnings in unfinished inline
tree integration; this is not a warning-free workspace claim. The executable is
84,574,720 bytes with SHA-256
`f4391f20544ad7ae50b43999edd314079c8e8da360a60745593bb5781cc2c763`.
Its matching private runtime, including existing viewer/filesystem/thumbnailer
packages, is `C:\Users\tjelinek\MidnightExplorer-20261108-185633-275f1844`.
Registration allows the original `_build/debug/me.exe` to discover that trusted
runtime without relaxing checkout/plugin ACL checks.

The original build-path executable passed its real Windows `--ui-smoke-test`
with isolated left/right fixtures, exit code0, empty stderr and a 364,649-byte
startup PNG. The rendered panels, drive icons and bottom command/function rows
were inspected; no startup/error popup appeared. Saved preferences had the same
SHA-256 before and after the self-closing run. Evidence is
`_build/qa/transfer-errors/smoke/result.json`, `startup.png` and stdout/stderr.

Final logs are `_build/qa/transfer-errors/ui-integration.log`, `build.log` and
`deployment.log`; render artifacts are under its `ui` subdirectory. Owned source
Rustfmt checks pass, and all handwritten files touched here remain below 100 KB.

The design and internal contract are maintained in
[transfer error decisions](../superpowers/specs/2026-11-08-transfer-errors-design.md)
and [native transfers](../developer/TRANSFERS.md). No dependency or plugin ABI changed.
