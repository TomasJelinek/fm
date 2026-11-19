# Panel diagnostic details

Every red/yellow diagnostic icon in a panel is a separate accessible control.
Hover shows the specific reason. Click, Enter or Space on the diagnostic control
opens its message box without selecting, opening, expanding or retrying the row. The containing panel
becomes active. A failed tree branch keeps a separate Retry/Collapse control.

## Size reports

`FolderSize.warnings` retains an optional `SizeWarningReport`. This is a cheap
shared owner with a cached `SizeWarningSummary` for presentation and a private
`SizeWarningSource` for backend snapshots. Borrow size results during rendering;
clone ownership only when retaining a result or requesting details.

Native, archive and inline-tree scanners append complete diagnostic records to
private temporary storage: a file in this user's private report root, which listing
and presence error reports share, that is anonymous on Linux and deleted on close
on Windows. On Linux the root is `$XDG_RUNTIME_DIR/midnight-explorer/reports`, then
`midnight-explorer-<uid>-reports` (effective user ID) and a per-process
`midnight-explorer-reports-<pid>` in the temporary directory; on Windows
`MidnightExplorer-<hex of the account name>-reports` and then
`MidnightExplorer-reports-<pid>` in the per-user `%TEMP%`. On Linux, links in the
runtime and temporary bases are resolved first. Each root must then be an owner-only
directory of this user reached without links; a refused root yields to
the next candidate, and a per-process root is removed after its last report closes
(`private_spool::private_spool_roots`). A record includes the operation, captured
path or provider identity and the original reason. Raw native units and control
characters are escaped losslessly. Report storage failure does not stop useful
counting: the summary and report distinguish observed issues from successfully
saved records. Cancellation alone is not an issue, but cannot erase an actual
concurrent failure.

`Command::OpenSizeWarnings` carries the retained report, request ID and cancellation.
Two report readers on the engine's resident worker threads (outside Tokio's blocking
pool) and eight pending slots bound report snapshot and preview requests. The
matching `Event::SizeWarningsReady` returns an immutable `GeneratedTextSource`.
Stale results, queue rejection and shutdown retire ownership on backend workers. No
source reads, report writes or final native-file destruction belong on the UI
thread.

Native scans keep one lazy log per I/O lane (four lanes by default, up to 32 with a
Manual **Disk I/O concurrency** setting); archive scans share a depth-first log with
exact subtree ranges. A snapshot never sees partial records or later
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

## Watch status

`Event::WatchStatus` reports how an accepted flat view or tree session is watched:
`PollingOnly` (with the registration error) after native registration failed, so
only the periodic check (about every 15 seconds) finds changes; `Restarting` while
a failed watch worker waits for its rate-limited restart; `Native` only to clear an
earlier status of the same view. Each status is sent once per change of a view's
status, never blocks the watch worker (a full event queue keeps the newest status
until it can be sent), and a status for a view the pane no longer shows is
dropped. It is non-fatal: rows, actions and manual Refresh stay usable, and it is
never shown as a panel error or a modal dialog. The presenter
(`Presenter::watch_notice`) shows it only while its view is the pane's current
view (a flat navigation, or the tree's navigation and session), in flat and tree
mode alike, as one line of secondary text above the pane's details bar. Its tooltip
and accessible description give the registration error of `PollingOnly`.
`TreeEvent::WatchFailed` keeps reporting scope enrollment failures and epoch
exhaustion.

A flat folder registers again with a refresh; a tree batch with a later scope
update, at most once per poll interval. A pane's watch worker that panics restarts
in place on its resident worker thread: at once the first time, afterwards no sooner than a
minute after its previous restart, reporting `Restarting` meanwhile. A registration
that panicked twice in a row is skipped until another one is made, so its view is
only polled and reports `PollingOnly`. Panics on platform notification threads,
such as the process-wide Windows watch service thread, are outside this policy.

## Presentation and viewing

Yellow size warnings mean that a total is incomplete because of actual failures.
Running or cancelled totals retain their estimate styling without an error icon.
Red listing errors remain distinct. Both controls provide a specific tooltip;
ordinary filename tooltips remain disabled.

Yellow size-warning reports use the ordinary asynchronous basic text viewer,
including search, selection/copy and Escape to close. Red listing and panel errors
open the themed owner-modal error dialog; they do not launch a viewer. A folder
error shows one concise reason and a separate folder path. Diagnostic reports and
technical context are collapsed behind **Details** by default, so report counters,
repeated paths and debug formatting do not become the main message. Path display
preserves native units and control-character escaping; literal backslash sequences
are never decoded or interpreted as a navigation target.

Folder Details uses the original bounded diagnostic lines, omitting the reason
and native path already displayed above. It does not decode escaped log records;
ordinary backslashes in paths retain their exact meaning. Copy details retains the
complete generated report. Other panel diagnostics can load bounded report previews
asynchronously; the presenter accepts only the matching request and notice generation. Closing or replacing the
notice cancels pending presentation and cannot revive it later.

## Explicit folder opening and authorization

Enter or a double click on a folder is ordinary explicit navigation, including a
folder whose background presence probe already reported an error. A failed native
navigation retains the last usable listing and opens **Cannot open folder**. Its
exact error occurrence is presented in the message box instead of a duplicate
panel banner. Background probes and same-directory refresh failures do not open
this modal or request authorization.

**Retry with authorization** appears only after the backend offers a denied
navigation receipt for that exact pane, generation and destination. Showing or
expanding the message never invokes authorization. The button sends only the
retained receipt IDs through the existing directory-listing service; no displayed
path grants authority. Closing/replacing the message, a new error occurrence or
navigation prevents an obsolete dialog callback from authorizing anything.

The existing Windows helper uses the OS UAC prompt; Linux uses the desktop polkit
agent through `pkexec`. An engine without an authorization service does not
advertise the button. Denial, cancellation and queue failure remain explicit errors
and preserve the existing listing; no silent retry, ACL change or privileged host
launch is introduced. The helper grants one read-only directory listing and does not
grant file content or mutation permissions. It runs only from an installation that
standard users cannot change (administrator-protected on Windows, root-owned on
Linux). Elsewhere the button still appears and the retry is refused with an
explanation of that requirement, which **Cannot open folder** shows. A missing
helper, as in the current per-user installations, is refused as not found. A helper
that is present but unprotected, as in build output or in a release folder extracted
where standard users can change it, is refused as access denied. The presenter
(`show_navigation_error`) shows an ordinary access-denied failure as "Access to this
folder was denied.", but for the navigation that an authorized retry started it
shows the failure's own reason, so that refusal states the requirement too. When
the folder was opened from a row of a flat native listing, that row's red
diagnostic icon also shows it. See the
[directory authorization helper](../../SECURITY.md#directory-authorization-helper)
for the current contract; the
[first qualification record](../verification/2026-11-11-directory-authorization.md)
keeps its dated scope.

A generated report title is only a display label and never authority to reopen a
native path. Missing size-report viewer support and diagnostic read failures are
reported explicitly. If the initial settings read failed, the disabled-viewer
policy is unknown and viewers stay unavailable for the session; reports, F3,
Find, Make File List and Compare then say so instead of asking to try again.

See the [implementation plan](../superpowers/plans/2026-11-10-size-warning-reports.md)
and [tree backend evidence](../verification/2026-11-10-tree-diagnostic-details.md).
