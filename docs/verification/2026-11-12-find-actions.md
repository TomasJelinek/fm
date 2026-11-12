# Find result action verification

The source addition provides separate explicit View, plain-text View, Edit and
Open paths from retained native Find results. Native activation can execute a
program; Enter/double-click continue to navigate. View capture validates identity
and revision around the installed native filesystem provider. External desktop
consumers retain the existing live-path race limits, with no snapshot guarantee.

## Causal missing-feature regression

`midnight-ui --test find_actions` compiled against the previous implementation
and failed at `Find is missing its explicit View action`. The test used the actual
Slint modeless window, software rendering and accessibility items. The RED log is
`_build/qa/find-actions-20261112/red.log`.

## Targeted verification

Linux native integration: all three new `find_actions` tests and all five existing
`find_actor` tests pass. Coverage includes changed source revision, raw Unicode
paths, directory/unknown result rejection, Stop retaining actions, restart/Close/
shutdown revocation, cancellation and the sixteen-live-capture limit. Evidence:
`_build/qa/find-actions-20261112/green-engine.log`.

The Linux Find presenter/library filter passes all 21 tests, including the five
new action tests: capability-only dispatch with frozen editor preferences, stale
and substituted reply rejection, directory-only navigation, current configured
shortcuts and a viewer launch carrying only retained capability authority. Existing
Find session/queue/sort, non-UTF-8 path and tooltip checks also pass. Evidence:
`_build/qa/find-actions-20261112/green-presenter.log`.

Software-window evidence does not establish native GPU/desktop qualification or
successful external application launch on a user's files. No real user file is
executed by these regression tests.

## Coherent Linux engine and viewer gates

The affected engine all-targets run passed 1329 tests (11 opt-in tests ignored)
across 44 suites. All four Find native action tests passed, including provider
final-drop after runtime shutdown. Both desktop action worker tests passed;
validation precedes native Open/Edit and aborted async delivery retires the source
on a worker. Native-host exact identity/revision checks passed on both encoded
platform formats (2 tests), and all 4 viewer Find capture tests passed. These gates
include the final native viewer cancellation bridge and metadata changes; the
later Custom Commands separator work is qualified separately.

Evidence: `_build/qa/find-actions-20261112/linux-final/engine.log`,
`native-host.log`, `viewer-find.log`, and `status.tsv` in the same directory.

## Final Linux software-window checks

The final Find library filter passed all 21 tests. The actual software-window
integration targets passed all 14 tests: explicit actions (1), interaction and
focus behavior (7), and modeless window geometry/lifecycle (6). The compact Details
page hides the three optional action buttons so the existing navigation controls
remain reachable at 320 × 200; ordinary result mode retains the explicit buttons.
The compact regression passed without relaxing its geometry assertion.

Evidence: `_build/qa/find-actions-20261112/linux-final/ui-find.log` and
`ui-windows.log`. Screenshots are in that directory's `render` subdirectory.
Linux installed-provider qualification is not claimed by these software tests;
the root integration gate records the separate actual Windows provider evidence.
