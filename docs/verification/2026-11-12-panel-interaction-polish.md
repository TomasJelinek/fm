# Panel interaction and diagnostics qualification — 12 November 2026

This report qualifies the current panel-polish batch, not completion of TREE-01
or the wider application audit. All generated logs and rendered frames are under
`_build/qa/`; they are not repository assets. Local builds use debug configuration.

## Tree publication and admission

The first failure was reproduced without a full queue: a background probe advanced
the actor projection before the visible UI projection was acknowledged. Tests now
cover delayed acknowledgements, an unrelated probe overtaking Expand/Navigate,
and exact refusal of removed, hidden, replaced or foreign occurrences. One emitted
publication and one coalesced pending publication bound ownership.

A second regression filled all eight command slots with viewport observations.
The original executable failed at its ninth observation. The repaired mailbox
coalesces observations and lets explicit Branch/Navigate evict speculative work;
a ninth explicit command is still refused. Superseded observations retire on a
backend worker. A production Reproject case proves that its strict admission
cannot overtake an unacknowledged provider publication and later regress revisions.

Linux current-source targeted run:
`_build/qa/tree-mailbox-pressure/green-results/summary.json`: **57 passed**
(4 acknowledgement integrations, 19 controller cases, 11 filter cases,
23 comparison/worker/mailbox/actor cases). One installed-provider case was ignored
in that Linux run. This replaces the earlier narrower 42-test ACK run.

`_build/qa/tree-replacement/green-results/summary.json` separately records **27 core
projection/selection cases**, including the bounded replacement lifetime proof.
The original replacement retained a complete source projection; the final numeric
payload does not. Comparison result previews are bounded to 512 KiB and contain no
provider, path, projection or report owners. Extra cases in that earlier summary
are repeated by the later engine run and are not additional coverage counts.

## Installed native provider on this Windows machine

The opt-in read-only test
`installed_navigation_tests::installed_native_expansion_and_navigation_survive_viewport_pressure`
uses the protected installed native filesystem plugin. It expands the requested
folder, submits 64 viewport observations, and verifies that navigation returns the
same retained native path. It does not mutate, delete or elevate anything.

- `C:\Windows\System32\drivers`: passed in 0.48 s;
  `_build/qa/panel-polish-20261112/installed-drivers.log`.
- `C:\Program Files\Git\bin`: passed in 0.22 s;
  `_build/qa/panel-polish-20261112/installed-git-bin.log`.

These are whole test durations, not representative browsing benchmarks. Genuine
child-listing diagnostics are permitted; an explicit queue/stale-view rejection is
a test failure. No child diagnostics were delivered before navigation in these
runs. This does not prove that restricted children can be read without elevation.

## Shortcuts, comparison and settings regression

Windows core shortcut target: **24 passed** in 0.02 s
(`_build/qa/panel-polish-20261112/deselect-green.log`). The default Ctrl+D dispatch
first failed against the previous files-only binding. Ctrl+D now clears all marks,
including expanded folders; Ctrl+Shift+D is unassigned by default. Saved deliberate
shortcut overrides remain authoritative.

Windows UI: **51 passed** across the 11-test main-menu target, selection binding,
22 keyboard-options cases, 6 user-menu forms and 11 comparison/navigation units.
Receipts/summaries: `_build/qa/panel-polish-20261112/menu-green/summary.json` and
`windows-ui-rest/summary.json`. The initial broad menu check still expected the
removed files-only deselect item; that obsolete assertion was removed and the
entire 11-test menu target was rerun successfully.

Linux UI: **17 passed**, the same 6 user-menu forms and 11 comparison/navigation
units (`_build/qa/panel-polish-linux-20261112/ui-results/summary.json`). This includes
the exact former GitHub failure: the test now expands Toolbar appearance before
looking for its icon controls. Production controls were already intentionally
collapsed; the test no longer assumes hidden controls are visible. Completed
GitHub run `43218163366` failed on that same assertion on Windows and Linux;
all independent SDK/plugin jobs passed. `_build/qa/panel-polish-20261112/ci-all-failed.log`
contains both job failures. A future pipeline result is not inferred from local tests.

Tree comparison tests cover mixed flat/tree panes, immediate bounded preview,
backend acknowledgement, stale either-side rejection before either preview changes,
and retry after a rejected selection. Comparison remains nonrecursive and native
only; virtual entries fail closed. The initial comparison test build did not
contain the new module, so no comparison RED result is claimed from that artifact.

## Final batch checks and main integration

The final pre-merge Windows artifact passed 28 core tree cases, 16 comparator
cases and 76 UI comparison/tree-action cases. The first runner request included
an unregistered test-module filter and refused to execute that UI group; the
corrected registered filters passed in `panel-polish-final-20261112/root-ui-results`.
The combined engine/notice run passed 38 notice tests and 18 native/capture tests.
Selection rendering passed 156 layouts (26 themes, both pane states, three views).

Two new UI fixtures needed corrections: ClipboardFilesRead must enter the real
`apply_with_sender` event route, and MinimalSoftwareWindow does not synchronize
native window size when a Slint height property changes. The folder-dialog fixture
now inspects the requested WindowItem height, as the native winit backend does,
then renders that size. Neither correction changes production behavior.

The Linux archive-report test initially failed because its temporary directory
was on an NTFS mount that cannot satisfy the required private Unix directory mode.
It passed against the same executable using an owned ext4 temporary directory.
The protection was preserved. Subsequent Linux gates use isolated ext4 temporary
fixtures, with logs and render results under repository `_build/qa/`.

Main's audit changes were reconciled with the newer plugin branch rather than
replacing current implementations. Worker admission now survives terminal-event
backpressure; abandoned requests cancel; completed tasks are reaped. Lazy history
models preserve current archive/network identities. Current automatic Cargo test
discovery and bounded CI batches supersede the old static integration-test wrapper;
its substantive font/history regressions remain. The separate-filesystem Linux
Trash qualification step is retained.

Focused Python CI discovery/batching checks: 29 passed, two Windows symlink cases
skipped because the process lacks symlink creation privilege (31 collected, 0.596 s).
Formatting and source-size checks passed; the source-size gate checked 2168 files.

## Merged-source validation

Windows complete core/common/comparator library suites passed 72, 95 and 31
cases respectively; nine Windows settings cases also passed. The shared runner
used three independent processes with six total test threads. The complete UI
library first passed 983 cases and exposed one obsolete keyboard expectation:
Ctrl+D still expected the former files-only action and Ctrl+Shift+D still expected
the removed default. Both expectations now follow the requested catalog behavior;
no production change was needed. The fresh full UI rerun is recorded below when
complete.

The merged Windows folder-dialog/backend gates passed 83 cases, including all
13 desktop integration tests and cancellation/admission lifetime regressions.
Clipboard/Paste/Delete gates passed 22. PDF palette and migration checks passed
13; the three theme/search/selection tests include 156 rendered selection layouts.
The final PDF color is a separate coral rule; DOCX/Markdown retain their brighter
neutral document color. Actual dark and light palette/notice images were inspected.

Merged Linux targeted checks passed 235 backend/platform/core cases and 171 UI
cases. This includes the retained font regression, native capture boundaries,
worker admission and lazy history models. The two installed-Windows-path tests
and one benchmark remain ignored on Linux. UI render tests run serially; backend
cases use bounded parallel workers and owned ext4 temporary fixtures.

Receipts and per-target logs are under:

- `_build/qa/panel-polish-final-20261112/merged-core-results/`
- `_build/qa/folder-error-notices-20261112/merged-dialog-tests/`
- `_build/qa/folder-error-notices-20261112/merged-notice-backend/`
- `_build/qa/panel-visuals-20261112/merged-color-tests/`
- `_build/qa/panel-visuals-20261112/merged-panel-tests/`
- `_build/qa/panel-visuals-20261112/merged-clipboard-tests/`
- `_build/qa/panel-polish-merged-linux-20261112/`

The Linux cross-filesystem Trash test also passed on two distinct owned Linux
filesystems, with the required `QUALIFIED cross_filesystem_trash` marker (one test,
0.04 s). The source remains untouched when that transfer is unsupported. Its
executable hash and qualification receipt are in `cross-filesystem-trash.json`.

The final complete Linux UI library passed 979 cases, with two explicit timing
benchmarks ignored and no failures. It used the refreshed `build-ui-final.jsonl`
receipt; results are in `full-ui-lib-results/summary.json`. The Windows debug
application and elevation helper also built successfully.

The final complete Windows UI library passed 984 cases (five intentionally
ignored) and the separate font integration check passed. Results are in
`panel-polish-final-20261112/final-ui-results/summary.json`; the UI suite took
17.2 seconds including runner overhead. A strict Clippy pass identified one
8-argument captured-open method. Pane/navigation are now passed as the existing
coupled scope; validation and ownership behavior are unchanged. Strict Clippy then passed all targets for core, common, Windows, engine, UI,
comparison, app and elevation-helper packages. Two further mechanical test/UI
lints were corrected (single-character append and fixed-size slice iteration);
existing dependency-capped vendored renderer warnings remain. The final successful
receipt is `panel-visuals-20261112/merged-clippy.jsonl`. The six Windows captured-action regressions passed after the signature cleanup
(`merged-final-captured-tests/summary.json`). The refreshed executable is verified
separately below.

A private debug runtime was registered and its deployed EXE SHA-256 matched the
build output. The first backend smoke rejected an executable snapshot change
while hashing. Both file hashes subsequently matched, and a repeat of all three
smoke modes passed without code or security-policy changes: backend, native UI,
and software-rendered UI (exit 0, empty stderr). Native/software screenshots were
inspected and actual user preference bytes remained unchanged. A QA-only probe used the identical locked-handle flags and four deliberately
slowed hash reads across build/private EXEs. All four identity/size/write/change
snapshots stayed stable, with matching SHA-256. Ordinary LastAccessTime changes
did not alter the guard fields. Source ChangeTime did record a metadata transition
at the initial failed smoke timestamp; the actor remains unknown. No integrity
check was weakened, and this is not described as a fixed production bug. Probe
output and initial rejection text are retained under the final QA directory.
This digest path applies to the shared development EXE that resolves a registered
private runtime; a directly launched trusted private installation does not take
that fallback digest path.

No full workspace/remote CI success or accepted UAC/polkit prompt is claimed here.

## Final executable and handoff

Post-lint Linux checks passed all six captured-action and 38 folder-notice tests
with no failures or ignored cases (`postlint-results/summary.json`). The final
Windows application/helper refresh passed, with generated UI bindings reused.

Final debug `me.exe` SHA-256:
`d731397f751647c5feb6ff2ba6ec822a598711bc48ee738e452f364363138984`.
It is available at `_build/debug/me.exe` and in the verified private deployment
`C:/Users/tjelinek/MidnightExplorer-20261112-panels-d731397f/me.exe`.
The desktop shortcut and matching runtime registration reference this build.

All three smoke modes passed on that final executable: backend, default native
UI and forced software UI, each exit 0 with empty stderr. User preferences were
unchanged. Both final screenshots were visually inspected. The final run is
`_build/qa/panel-polish-final-20261112/smoke-20261112143652518/`.
Each run now uses a fresh QA output directory because the snapshot writer
correctly refuses to overwrite an earlier image.

The batch is ready for main integration. Broader feature/audit work remains
paused pending scoped assignments through the documented six-role workflow.
Remote full-workspace CI is reported separately; local checks are not a claim
that every independent plugin suite or native elevation flow was rerun.
