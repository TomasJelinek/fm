# Detailed volume information verification — 8 November 2026

Ctrl+F1 and Ctrl+L now request measured backend volume information. The dialog
shows native filesystem/root/label/source when available, exact and readable
total/used/free/user-available capacity, conservative storage classification and
reported filesystem properties. Inside archives it identifies the hosting native
container. Used space subtracts filesystem-free space, preserving the distinction
between free space and caller quota availability. See [the contract](../DRIVE_INFORMATION.md).

The actor permits one active native query and one coalesced latest request.
Request, pane, navigation, raw path, complete archive address and notice occurrence
checks prevent obsolete replies from replacing current information. Updating the
notice body keeps its occurrence unchanged, so a dismissed loading dialog stays
dismissed. No Slint filesystem probe, shell command, dependency or plugin ABI change
was introduced.

## Completed debug/test checks

- Windows: core capacity accounting (1), engine cancellation/coalescing/shutdown
  (2), native query/error/capability tests (3) passed. Native fixtures include real
  Unicode names, an unpaired UTF-16 surrogate, a path longer than 260 UTF-16 units,
  and missing/cancelled requests. Log: `_build/qa/drive-information-windows-tests.log`.
- Linux: core accounting (1), engine actor tests (2), mount/device selection,
  optional udev label decoding, real native query and independent counter-overflow
  handling (4) passed. Real path fixtures cover Unicode, non-UTF-8 bytes and long
  paths. Logs: `_build/qa/drive-information-linux-tests.log` and
  `_build/qa/drive-information-linux-tests-final.log`.
- The rebuilt Windows UI unit suite passed all 494 tests, including the four
  volume presenter regressions and adapted shortcut request tests. Log:
  `_build/goal/archive-hotpaths/combined-ui-green.log`.
- The actual Windows software-rendered `volume_information` workflow passed
  Ctrl+L, closing the loading notice before delivery, Ctrl+F1, measured details,
  Copy details, a 1040×600 window, and Escape. The direct compiled rerun also
  passed. Logs: `_build/goal/archive-hotpaths/combined-ui-integrations.log` and
  `_build/qa/drive-information-ui.log`.
- The Linux UI unit suite passed 490 tests and the same actual
  `volume_information` workflow passed (1), including dismissed late-reply
  handling. Log: `_build/goal/find-refactor/linux-ui.log`.
- Normal and small screenshots in `_build/qa/drive-information-ui/` were inspected.
  The deliberately extreme Unicode path wraps inside the scrolling body; title,
  top-right close, Copy details and Close remain visible without footer clipping.
- Strict all-target Clippy passed for core, platform-common, the native platform
  crate and engine on Windows and Linux. Logs:
  `_build/qa/drive-information-windows-clippy.log` and
  `_build/goal/find-refactor/backend-strict2.log`.

## Limits

These are live read-only observations, not filesystem snapshots or authority for
mutation. Optional native fields can remain unavailable. A blocking OS call cannot
be forcibly interrupted, but superseding/closing prevents stale delivery and worker
growth remains bounded. Linux udev labels depend on an available device record.
The UI screenshots establish software-renderer layout, not native multilingual
shaping or a physical-media test matrix. No macOS or untested Windows-version
qualification is claimed. Application packaging/deployment remains separate.
