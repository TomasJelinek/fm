# Parallel queue follow-up, 11 November 2026

This follows the committed [first queue batch](2026-11-11-queue-batch.md).
It combines three independent work areas with cross-review:

- Native Rename/Create/Delete confirmations and Copy/Move destination forms,
  including exact cancellation identity, retained edits and owner focus.
  [Contract and regression evidence](2026-11-11-native-operation-windows.md).
- Cooperative archive preview cancellation through public filesystem API 0.4
  and explicitly negotiated protocol 3. Canceling one thumbnail read preserves
  the shared archive and verified cached members; old protocols retain their
  safe drain behavior. [Worker and cache evidence](2026-11-10-archive-thumbnails.md).
- CI test-artifact disk usage and a stale Find fixture that treated valid
  BOM-marked UTF-16 as malformed text. Production text decoding is unchanged.

This is a bounded implementation batch. Native migration of Jobs, conflicts,
metadata, comparison, filters, Options/color picker and Hot Paths remains open.
The existing implementation backlog continues to track those tasks and wider
platform/interactive qualification; this record does not close the overall goal.

## Independent archive qualification

Windows host/thumbnail tests passed 78 cases with 20 existing opt-ins excluded.
Two added terminal/watchdog cases also passed in a nine-case focused rerun;
previously passing cases are not counted twice. The public API passed 13 tests
and the independent archive plugin passed 41, with four opt-ins excluded. Strict
all-target checks passed for the host, thumbnailer and independent plugin.

A fresh private runtime then passed seven installed-worker checks: three image
helper cases and four archive cases. The latter cover all 11 supported formats,
corrupt inputs, cooperative ZIP/solid-7z cancellation, unchanged cached members,
retry on the same mount, ordinary hard cancellation and pathless sources.
The standalone archive executable SHA-256 is
`92ef3b1c52e945a28ddadc278373173fc69955866e8211d9af7fba172dd94c37`.

Linux core/host/thumbnail/API checks passed 117 unique tests, including the two
later host regressions. Six trusted-installation/measurement opt-ins were excluded.
Native private-cache fixtures used ext4 beneath `/tmp/fm-interaction/_build`;
checkout DrvFS was not treated as evidence of Unix mode-0700 support. No production
permission checks were weakened.

## Integration gates

The final Windows UI library gate passed 828 tests in 18.16 seconds; two desktop
opt-ins were excluded from that ordinary run and passed in separate processes.
The actual Rename and Copy adapters were checked with real HWNDs: correct owner,
no separate taskbar style, movement/resizing beyond the owner, exact native-close
cancellation, and owner re-enabling. The separate generic owned-dialog probe also
passed. Eight Find backend cases passed in 0.19 seconds.

The affected Windows integrations passed 99 cases across 24 targets. The corrected
delete capture and four compact-operation checks passed; inspected images show
real child content and action artwork. After improving the native fixture's
cancellation synchronization and ext4 override, only its two affected Windows
targets were repeated: eight tests passed. These reruns are not added again to
the unique integration count. Strict all-target Clippy passed for the six affected
application/host packages; the two existing vendored FemtoVG warnings remain.

The debug executable is `_build/debug/me.exe`, SHA-256
`b0b2b6cdff7adbbe4703a396a11a1b09d4f99adb1b57b2c98b8a5e0281bd9697`.
Its matching private runtime is
`C:/Users/tjelinek/MidnightExplorer-20261111-operations-b0b2b6cd`, with the archive
worker above. Backend, software-rendered and native-rendered startup checks all
exited zero with empty stderr. Both startup images were inspected; the normal
debug app was then opened successfully. This is a debug deployment, not a release.

The initial Linux gate passed 823 UI library cases and 94 affected integrations,
but failed four checks in two targets. Three native-operation fixtures incorrectly
used DrvFS rather than the configured ext4 root; the helper now honors that root.
The three native-operation cases then passed on ext4, as did the additional
corrected delete-summary case. Linux has 823 passing UI library cases and 98
unique passing affected integration cases, with one remaining compact comparison
Escape failure. The focused diagnostic identifies an invisible zero-width control
holding focus on first opening; its narrow initial-focus correction is queued for
the next combined Slint integration. That failure is not an operation-window
regression, but remains an application defect to fix. Neither the original gate
nor the complete Linux application suite is claimed green.
Earlier behavioral failures and their exact scope remain documented in the
feature-specific records; compilation errors or fixture corrections are not
reported as successful product regressions.

## Iteration policy and next parallel batch

The user's latest instruction defers full local suites until implementation is
complete. Subsequent changes use focused regressions and directly affected
integration targets. The already-running initial gates above are retained as
historical evidence rather than rerun wholesale. Jobs/decision native hosts and
Filter/Hot Paths native hosts are being implemented in parallel, with shared UI
hooks integrated once and the compact comparison focus repair included.
