# Panel diagnostic details

Every red/yellow diagnostic icon in a panel is a separate accessible control.
Hover shows the specific reason. Click, Enter or Space opens its full details
without selecting, opening, expanding or retrying the row. The containing panel
becomes active. A failed tree branch keeps a separate Retry/Collapse control.

## Size reports

`FolderSize.warnings` retains an optional `SizeWarningReport`. This is a cheap
shared owner with a cached `SizeWarningSummary` for presentation and a private
`SizeWarningSource` for backend snapshots. Borrow size results during rendering;
clone ownership only when retaining a result or requesting details.

Native, archive and inline-tree scanners append complete diagnostic records to
private temporary storage. A record includes the operation, captured path or
provider identity and the original reason. Raw native units and control characters
are escaped losslessly. Report storage failure does not stop useful counting:
the summary and report distinguish observed issues from successfully saved records.
Cancellation alone is not an issue, but cannot erase an actual concurrent failure.

`Command::OpenSizeWarnings` carries the retained report, request ID and cancellation.
Two blocking workers and eight pending slots bound snapshot requests. The matching
`Event::SizeWarningsReady` returns an immutable `GeneratedTextSource`. Stale results,
queue rejection and shutdown retire ownership on backend workers. No source reads,
report writes or final native-file destruction belong on the UI thread.

Native scans share four lazy worker-lane logs; archive scans share a depth-first
log with exact subtree ranges. A snapshot never sees partial records or later
appends. Rerunning, navigating or evicting a cache does not change an opened report.
Backing storage remains until the lane/generation owner and all readers release it;
navigation does not promise immediate deletion of every shared backing file.

## Listing errors

`TreeDisplayRow.error` and `TreeDiagnostic::as_str()` are bounded summaries. Their
separate retained sources contain complete details. Refresh reports compose exact
failed-branch sources, bounded by retained tree nodes, rather than collecting an
unbounded in-memory list of copied error strings. Repeated cached watch failures
reuse the same source identity so they do not cause repeated dirty publications.

The presenter correlates full panel errors with the exact pane generation and
error occurrence. A new error invalidates the preceding source even when its short
text is identical. Mouse presses capture row, report revision and context: refresh
or virtualized rebinding between press and release cancels the gesture.

## Presentation and viewing

Yellow size warnings mean that a total is incomplete because of actual failures.
Running or cancelled totals retain their estimate styling without an error icon.
Red listing errors remain distinct. Both controls provide a specific tooltip;
ordinary filename tooltips remain disabled.

Yellow size-warning reports use the ordinary asynchronous basic text viewer,
including search, selection/copy and Escape to close. Red listing and panel errors
open the themed error dialog, including retained full diagnostic sources; they do
not launch a viewer. The backend loads dialog details asynchronously, and the
presenter accepts only the matching request and notice generation. Copy details
retains the full captured source even when the visible text is bounded. Closing
or replacing the notice cancels pending presentation and cannot revive it later.

A generated report title is only a display label and never authority to reopen a
native path. Missing size-report viewer support and diagnostic read failures are
reported explicitly.

See the [implementation plan](../superpowers/plans/2026-11-10-size-warning-reports.md)
and [tree backend evidence](../verification/2026-11-10-tree-diagnostic-details.md).
