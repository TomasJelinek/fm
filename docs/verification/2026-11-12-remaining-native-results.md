# Remaining native results and service children: qualification

Date: 12 November 2026. Baseline: `b3fa92e`. This batch implements the accepted
[plan](../superpowers/plans/2026-11-12-remaining-native-results.md). It advances
LIFE-01 without claiming completion of all actors or the wider feature/UI goal.

## Changes and bounds

Make File List now guards native destination pins, captured size reports and generated
Spools through cancellation, replacement, confirmation and externally retained output.
Six explicit retention tokens each reserve two cleanup leases before accepting work.
The four visible jobs and two workers remain separate limits. Refused replacement
preserves the previous generation. Both job and generated output retain their shared
token until actual native cleanup finishes. See the
[File List evidence](2026-11-12-file-list-retirement.md).

Network discovery adopts its newly created provider inside the native call, before
returning a completed result through a JoinHandle. Its OnceLock, controller and
requests retain only the guarded wrapper. Exactly one startup discovery uses one
lease; the existing two-request quota is unchanged. See the
[network evidence](2026-11-12-network-provider-retirement.md).

Original service ownership now spans independently surviving native children for
path choosers, shell menus, volume discovery/details, listing, storage information,
locale/network settings and folder-size/presence work. PathPicker Drop cancels the
existing native chooser/reply token, including a publisher with a full live result
queue. This is cooperative cancellation, not forced termination of an arbitrary OS
modal dialog. Plain metadata/error delivery needs no additional native owner.

The startup ledger is **166 payload slots and six waiters**: 153 preceding slots,
12 File List slots and one discovered-provider slot. Process quotas remain eight
shared terminal/editor/custom-command children and sixteen independent openers.
No dependency, plugin ABI, confirmation, checksum or operation-authority change was
made. No cleanup thread per Drop, unbounded retry queue or executor-thread cleanup
fallback was introduced.

## Reproduction and review

Two PathPicker tests first failed against the prior implementation: final service
release happened on the caller, and Controller Drop did not cancel the native
chooser/reply with a full live result queue. Build: 18.10s; two failures in 0.01s.
Evidence: `_build/qa/file-list-lifetime-20261112/path-picker-red.log`.
The cancellation fixture records failure and explicitly releases its native gate;
it does not leave a hung worker on RED. Its final version checks both chooser-wait
and result-delivery cases; the baseline stopped at the first failing case.

Six additional actual-controller cases cover Picker, ShellMenu, Volumes,
VolumeInformation, FolderPresence and EntrySizes wiring. Together with eight existing
service-task cases they make 14. These are wiring coverage, not six independently
reproduced baseline defects. Existing TaskLifetime tests cover unpolled futures,
completion, panic and independently surviving blocking children.

File List reproduced three failures before implementation; network discovery
reproduced three. Their owner notes distinguish these RED cases from further
admission/cleanup regressions. First coherent combined runs passed them all.

The [independent source review](2026-11-12-service-boundary-peer-review.md) found an
additional transient archive-mount owner in File List size binding. The correction
borrows the mount under the original Snapshots lock and does not clone or relock.
A semantic test retains the accepted size map across root retirement and rejects
a different mount with the same numeric ID. Review accepted this correction and
reported no remaining actionable finding within the reviewed batch.

## Final affected gates

Source was frozen before final qualification. One Cargo owner per platform selected
coherent artifacts; builds were locked/offline. Tests used owned temporary fixtures.

Windows driver: `_build/qa/remaining-native-results-20261112/windows-final.ps1`.
The full engine library passed **956 tests**, with seven existing ignored cases,
zero failures, in 6.55s; compilation took 28.49s. All 30 File List and seven new
network lifetime cases appear as passing in this run.

| Windows integration | Passed | Time |
| --- | ---: | ---: |
| folder_size_warnings | 2 | 0.23s |
| folder_sizes | 20 | 0.25s |
| navigation | 7 | 0.23s |
| navigation_fallback | 2 | 0.21s |
| navigation_ordering | 5 | 0.21s |
| selected_folder_sizes | 5 | 1.28s |
| volumes | 5 | 0.22s |

All 46 integration cases passed without ignored tests. Strict engine all-target
Clippy (`--no-deps -- -D warnings`) passed in 19.77s. Debug application and elevation
helper built in 1m20s. The debug build retains two pre-existing vendored femtovg
warnings; this does not change the clean strict engine lint result. Logs are
`windows-final-tests.log`, `windows-clippy.log` and `debug-build.log` beside the driver.

Linux's coherent artifact build took 1m34s. Eight scoped library filters passed
85 tests: File List 30, Network 15, Picker 6, Shell 3, VolumeInformation 3,
FolderPresence 6, service-task wiring 14 and engine lifecycle 8. The same seven
integration targets passed 46 tests. Strict engine all-target Clippy passed in
1m32s; the driver exited zero. Exact timings, commands and binary hashes are in the
[network qualification](2026-11-12-network-provider-retirement.md#final-affected-linux-gate).
This was not a full Linux library or whole-workspace test run.

Rust formatting, whitespace and LF checks passed for changed sources. The source-size
gate checked 2,041 files; every handwritten source remained within 100,000 bytes.

## Remaining boundaries

Folder-size/presence diagnostic reports may contain native file-backed SpoolPrefix
and AppendSpool owners. Anchoring original services does not establish safe final
destruction of every new report clone. Those output/mailbox paths remain a separate
audit, alongside archive navigation outputs, tree mailbox unwind and repeated engine
builder replacement. Existing protection is recorded without treating reports as
plain metadata. LIFE-01 and the full product/UI goal remain open.

## Debug deployment and smoke checks

The fresh debug executable is `D:/experiments/dev/FM/_build/debug/me.exe`.
SHA-256: dd95b93683ee2e3e8350d0363b99d6f6ab435ae5ce26ba93799bedeef20cfc3b.
The matching private installation is `C:/Users/tjelinek/MidnightExplorer-20261112-results-dd95b936`, registered for this exact
executable hash. Unchanged viewer, filesystem and thumbnail plugins were reused
from the preceding qualified private installation; no plugin rebuild was required.
Deployment removed its own 0.532 GiB staging copy after containment/reparse checks.

`_build/qa/remaining-native-results-20261112/run-final-smoke.ps1` ran the backend
listing smoke, normal-renderer UI startup and explicit software-renderer UI startup.
All three exited zero with empty stderr. Both 3015 x 1845 snapshots were inspected:
two populated panels, toolbars, command line and bottom function buttons rendered.
User preferences were unchanged by hash. Logs and `final-native.png` /
`final-software.png` are beside the script. These are startup/render checks, not a
claim of foreground interaction coverage, smooth scrolling performance or Linux GUI
qualification on the locked desktop.

A separate read-only check confirmed the accepted Delete UI contract already uses
the same JobProgressWindow as Copy/Move: discovering, items/s, conditional ETA,
pause/resume and exact-job cancel. Native close, header close, footer Cancel and
Escape converge on cancellation; rejected cancellation keeps the window visible.
Jobs remains a separate session-history window. Its existing native tests and prior
visual evidence are in [tree/Delete integration](2026-11-12-tree-delete-integration.md);
this batch did not rerun or change that UI.
