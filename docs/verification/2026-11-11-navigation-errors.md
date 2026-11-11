# Failed navigation and diagnostic notices — 2026-11-11

An unsuccessful folder open previously restored rows and cursor in the presenter,
but the temporary pending listing had already clamped the virtualized panel's
scroll offset. Revealing the restored cursor then chose a new top row. The panel
now captures its viewport before replacing the model and restores that viewport
when the accepted failed navigation returns to the same path. Layout settlement
preserves the offset; later scroll, cursor, path, filter context or query changes
cancel the restoration. Successful navigation retains ordinary reset/reveal.
Stale backend replies remain fenced by the presenter's existing navigation IDs.

Read-only Salamander comparison: `D:/experiments/salamander-main/src/fileswn2.cpp`
saves the list top index and caret before changing paths (around line 320), then
passes both to `RefreshListBox` when `ChangePathToDisk` fails without a changed
path (around lines 365–382). This supports preserving both viewport and focus.

The tree retry disclosure used a Unicode circular arrow that some configured
fonts cannot render. It now uses the existing refresh SVG, with the same input
routing and no permission-policy change. The stale text-glyph test was updated to
compare the rendered source against the exact embedded SVG, verify visible bounded
artwork and unchanged row geometry, and exercise accessible exact-row retry.
The final `tree_disclosure` target passed all three cases, including the exact SVG
and accessibility assertions; see
`_build/qa/viewer-settings-20261111/final-corrections-ui-midnight-ui--tree_disclosure.log`.

Red folder/panel error gestures open the application's owner-modal notice with
Copy details and Close. Full generated diagnostics are fetched through the
bounded backend diagnostic reader: 64 KiB preview, 8 KiB reads, cancellation and
exact request/notice generation checks. The clicked path/context remains visible
above reports even if a provider omits it. Large previews state their truncation;
Copy uses the retained full source and the existing 4 MiB clipboard limit rather
than copying a truncated preview. No diagnostic source is read on the UI thread.
The amber folder-size warning report retains its existing report-viewer behavior.

A capability-gated Retry with authorization button emits an explicit action only
for the current error gesture. The backend/elevation implementation is separate.
Rendering a permission error does not itself request elevation. Stale press/release
pairs are rejected when either error identity or text changes.

The visual Loading notice now starts after 500 ms. The loading state, backend
request, viewport capture and listing updates remain immediate. Completion and
path replacement cancel the visual timer. A controlled-clock regression checks
that the notice is absent at 499 ms and present at 500 ms for initial navigation,
refresh and network listing, without changing the panel geometry.

The new tests cover failed double-click and Enter navigation, repeated denial,
exact viewport after deferred layout, later manual scrolling, successful opening,
stale failure, modal routing/full-source copy, bounded preview and closed/stale
replies, plus explicit authorization gesture fencing. The integrated
`navigation_loading` target passed both rendered tests, including
exact failed-open viewport preservation and the 499/500 ms visual delay. Its log
is `_build/qa/viewer-settings-20261111/progress-ui-first-navigation_loading.log`.
The `error_notice` rendered target passed all four tests, including explicit
current-error authorization gestures and complete long-error copying; its log is
`_build/qa/viewer-settings-20261111/progress-integrated-error_notice.log`.
All seventeen `panel_warnings_tests` and the red-folder modal-routing regression
passed in `progress-integrated-lib-midnight_ui.log`. The final full UI-library run
also passed (`final-ui-unit.log`); the unrelated shortcut fixture from the earlier
run was corrected. The backend diagnostic reader/controller tests were separately
reported passing by the backend agent.
