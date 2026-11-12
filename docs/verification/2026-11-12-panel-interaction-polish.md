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

## Validation still in progress

Final folder-notice, palette selection and native shell action validation, the
combined debug deployment and the final integration review will be recorded here
when their fresh artifacts pass. Native UAC/polkit prompts have not been accepted
by these automated checks.
