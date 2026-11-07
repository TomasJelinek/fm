# Metadata, filter and provider checkpoint — 2026-11-07

This follows the pushed Custom Commands checkpoint `a728d01`. It adds two usable
features and one internal migration boundary:

- Panel **Expression** filters alongside Masks/Regex, with typed name, size,
  modified time, kind and attribute predicates; bounded backend compilation,
  precise diagnostics, preserved valid filters and preference schema 18.
- Optional **Preserve exact modified time** in native Copy/Move dialogs. The
  first slice accepts at most 64 native regular files; content remains independently
  SHA-256 verified. Exact timestamp failure never silently rounds or downgrades.
- An internal provider registry and transitional native/archive Browse/Read
  adapters. Production panels and operations have not been switched to generic
  native plugins; their original safeguards remain in place.

Details and limits:
[expressions](../developer/METADATA_FILTERS.md),
[transfer options](../developer/TRANSFER_OPTIONS.md),
[transfer contract](../developer/TRANSFERS.md),
[provider registry](../developer/PROVIDER_REGISTRY.md).

Windows affected UI tests passed **479**: library 466, commander dialogs 3,
Copy dialogs 5 and draggable dialogs 5. Linux passed the same scopes **476**:
library 463 plus 13 integration cases. The compact 320 × 200 Move dialog was
rendered and inspected; destination, checkbox and both action buttons are visible.
These counts describe fresh affected scopes, not a full workspace or plugin run.
See the [expression evidence](2026-11-07-metadata-expression-filters.md) and
[provider evidence](2026-11-07-provider-registry.md).

Focused UI strict Clippy passed on both platforms. Final Windows all-target
core/common/native/engine strict Clippy, workspace formatting and source-size
checks passed: 1,011 handwritten source files, all at most 100,000 bytes.
The complete Custom Commands settings target was then freshly rebuilt against
the current bindings: 12 tests passed on each platform, including drag ordering,
cancellation, clipped drop targets and automatic scrolling. Logs are
`_build/goal/vfs/command-order-windows.log` and `command-order-linux.log`.

## Release and native startup

The locked, offline Windows release build completed successfully in 6m39s.
`_build/release/me.exe` is 42,960,896 bytes, SHA-256
`D6EB6C6284569568F52E3DD77CE2FEAFB05F984FB12DFC0BFC4231B4BD748F8F`.
The verified private installation is
`C:\Users\tjelinek\MidnightExplorer-20261107-192543-2656d48c`; the redirected
Desktop shortcut targets it, and the direct checkout executable has a matching
hash-bound runtime registration. Existing plugin packages were reused; their API
and sources did not change in this checkpoint.

The direct checkout executable passed `--ui-smoke-test` on Windows/WGPU DX12
(exit 0). Its captured main window was inspected: both panels, command line and
bottom controls render, with no startup error dialog. The test did not change
preferences: their before/after SHA-256 is
`EE8B6D629D9931443C1BD39A9B8BD65C791E54D5F11EF86C7640E8C04E1DC6CB`.
Evidence is under `_build/goal/vfs/native-release/` and the build log is
`_build/goal/vfs/windows-release.log`. The existing WGPU
`MAPPABLE_PRIMARY_BUFFERS` warning remains nonfatal; this startup check is not a
fresh viewer-worker or performance qualification.

## Remaining scope

Move still retains captured originals in protected recovery and does not reclaim
source disk space. Permissions/ACLs, all timestamps, links and directory Move are
not completed by this option. Failures before capture keep original names; failures
during/after capture may leave original bytes in recovery, reported by Jobs.

The broader implementation goal remains active: generic filesystem public API
and native plugin cutover, inline trees/network discovery, remaining transfer
semantics and scheduling, and the outstanding platform/media qualifications remain
listed in the implementation backlog. The original native keyboard-settings abort
has not been reproduced and must not be claimed fixed by these unrelated changes.
