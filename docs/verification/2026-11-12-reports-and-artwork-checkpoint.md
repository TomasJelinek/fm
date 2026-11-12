# Report ownership and executable artwork checkpoint — 12 November 2026

This checkpoint combines the shared report arena, automatic native executable artwork for
Custom Commands, and their application bridge. It does not claim completion of the full goal.

- [Report storage contract and qualification](2026-11-12-report-arena-retirement.md).
- [Producer plumbing](2026-11-12-report-arena-plumbing.md).
- [Executable artwork](2026-11-12-command-executable-icons.md).

## Final affected gates

Windows engine unit gate: 988 passed, 7 existing ignored, 6.95 seconds. Five report/tree
integration targets: 57 passed, 1 existing ignored. Windows native executable artwork: 2 passed,
0.09 seconds. Application thumbnail/icon bridge: 11 passed, 0.55 seconds. Strict Windows
core/engine/UI/platform/application all-target Clippy passed in 1m11s.

Linux report/producer and native-file cases, five integration targets (57 passed, 1 ignored),
command-icon actor (4 passed), and final native icon cases (19 passed) passed. Strict Linux
engine/platform all-target Clippy passed in 1m20s. Component evidence above records exact filters,
RED runs, native temporary roots and limitations. These are the affected gates, not a new full
workspace/plugin-matrix run. Existing vendored femtovg warnings remain; first-party strict checks pass.

Fresh read-only review found no actionable issue in report ownership, startup reservation accounting,
or UI icon delivery. A separate follow-up confirmed that repeated Engine builder replacement can
still release a displaced last service owner on the caller. Current app composition installs each
service once. That API follow-up remains open; no unbounded deferred-owner collection was introduced.

## Debug deployment

Fresh app and elevation-helper build passed in 7.13 seconds. Executable:
`D:/experiments/dev/FM/_build/debug/me.exe`, 98,880,512 bytes, SHA-256
`89a4a1095d1e48b3a355044630b0594eed9f6d7f2fd011246c9693da37d82690`.

Registered private runtime:
`C:/Users/tjelinek/MidnightExplorer-20261112-reports-icons-89a4a109`.
Unchanged independent plugins came from the previous verified mailboxes runtime. No plugin was
rebuilt or represented as newly qualified by this app-only batch. Owned deployment staging was
removed, reclaiming 0.532 GiB. Earlier obsolete release-build cleanup reclaimed 4.29 GiB.

Backend, native-renderer and forced-software startup smokes all exited zero with empty stderr and
unchanged user-preference hash. Both rendered main-window PNGs were inspected: panels, drives,
status/command bars and the function toolbar are present without startup errors. These automated
snapshots do not establish interactive compositor smoothness. Logs, scripts and screenshots remain
under `_build/qa/report-arena-20261112/` and are intentionally not committed.

The unchanged shared Copy/Move/Delete progress component also passed its focused visible-progress,
current-file-meter and Delete-chart case. `TRANSFERS.md` now explicitly identifies the shared
component and reiterates that X/Escape cancels the exact active operation.

## CI settings-test correction

The previous commit's GitHub run 43155735332 completed all 18 plugin jobs successfully;
its Ubuntu main job failed the custom-command form scrollbar assertion. The local RED reproduced
that exact failure. Measured geometry and the pinned widget showed the horizontal bar was disabled;
the test incorrectly reserved its former bottom overlay. The corrected assertion checks containment
and clearance from the actual right-hand vertical bar, retaining wheel and real pointer save checks.
No application layout was changed to accommodate the obsolete assertion.

The complete affected settings target passed 14/14 on Windows in 2.61 seconds and 14/14 on Linux in
10.84 seconds. See [measured CI diagnosis](2026-11-12-command-settings-scroll-ci.md). The previous
run's Windows main tests passed and its release build was still in progress at this checkpoint.
A new remote CI run is not claimed green by these local results.
