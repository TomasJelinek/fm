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
pool) and eight pending slots (`WORKERS`, `PENDING` in `size_warning_requests.rs`) bound
report snapshot and preview requests. The
matching `Event::SizeWarningsReady` returns an immutable `GeneratedTextSource`.
Stale results, queue rejection and shutdown retire ownership on backend workers. No
source reads, report writes or final native-file destruction belong on the UI
thread.

Native scans keep one lazy log per I/O lane (four lanes by default, up to 32 with a
Manual **Disk I/O concurrency** setting, `DiskIoConcurrency`); archive scans share a depth-first log with
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
`PollingOnly` (with the registration error) when native registration failed, so
only checks find changes; `Restarting` while a failed watch worker waits for its
rate-limited restart; `Native` only to clear an earlier status of the same view.
A flat folder whose registration fails is checked about every second (less often
for folders with more than 4,000 entries, and never more often than ten times as
long as a check takes) while the registration is retried after 1, 2, 4 and 8
seconds, then every 15 seconds; a successful retry switches to notifications
without a status, and `PollingOnly` is reported only once registrations kept
failing for a minute. A tree batch whose registration fails reports `PollingOnly`
at once and is polled at the regular interval. On Linux every subscription of the
filesystem service shares one inotify instance (`fs.inotify.max_user_instances` is
a small per-user limit shared by all programs); a directory's native watch ends
with its last subscription, and a directory replaced since its watch was added
(renamed away and created again) is watched anew, without the old one's watch,
when a subscription registers it again. inotify watches folders, not paths: a
registration that names a folder the shared instance already watches under another
path (a bind mount, or a folder renamed since and opened under its new name) gets an
instance of its own, so that neither path's watch can end or redirect the other's. Each status is sent once per change of a view's
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

A flat folder's accepted rows are checked right away when the listing that produced
them read the folder to its end before the folder's watch took effect, as a small or
empty folder usually is: nothing counts a change in between, which would otherwise
wait for the 15-second check. Rows listed while the folder's watch was in place, such
as a refresh or re-sort of a watched folder, are not checked again; neither are rows
of a folder whose registration fails, which is checked often anyway and right away
when a retry succeeds.

A flat folder also registers again with a refresh; a tree batch with a later scope
update, at most once per poll interval. A pane's watch worker that panics restarts
in place on its resident worker thread: at once the first time, afterwards no sooner than a
minute after its previous restart, reporting `Restarting` meanwhile. A registration
that panicked twice in a row is skipped until another one is made, so its view is
only polled and reports `PollingOnly`. Panics on platform notification threads,
such as the process-wide Windows watch service thread, are outside this policy.

A check compares a flat folder with the rows the pane accepted. With Show hidden
files off, it leaves out the hidden entries that the listing left out, by the same
rule (`hidden_entries`: dot-names on Unix, the hidden attribute on Windows), so a
folder with dotfiles or `desktop.ini` does not look changed at every check. A change
of hidden entries alone is then found only by a notification, whose re-read shows
nothing new. Accepted rows keep the choice they were listed with (a resort keeps
that of the rows it sorts) until the re-read that follows a change of the choice is
accepted.

An adopted inline tree's root is checked the same way while its rows are still the
flat listing the tree was adopted from: what that listing left out (it counts them;
with nothing to leave out, the current choice decides) is left out of the root's
checks, so adopting a tree in a folder with dotfiles requests no maintenance pass.
Once a maintenance pass lists the root again, the tree's model holds its hidden
entries (only the projection leaves them out), and the root's checks keep them, as
an expanded branch's always do: a change of a hidden entry there is found, and the
re-listing it causes shows nothing new.

## Free space

A pane's free space ("N free" in its details strip) comes only from
`Event::SpaceUpdated` for the pane's current navigation; the presenter drops an
answer for any other one. One engine worker (`free_space::Controller`) owns every
read of it:

- **Navigation.** Each Navigate, Refresh and Resort reads the folder's storage kind
  (`Event::StorageKindUpdated`, sent before the space query, which can block on a
  network share) and its free space at once, under the navigation's cancellation.
  A watch hint lists the folder again (`DirectoryChanged` → Refresh), so a change of
  the shown folder's rows already reads its free space again this way. Archive and
  Network navigations clear the pane's target.
- **Requests.** `Command::RefreshSpace { pane, navigation }` asks for another read of
  the current navigation; one for any other navigation is ignored. The presenter
  sends it for both panes when the window gains the focus; for each panel whose
  folder is on the volume of a folder whose watch reported a change (a flat watch
  hint's re-read or a tree refresh; `Presenter::folder_changed_space`), so that
  another program's deletion also shows in the other panel on that volume; and for
  each panel whose folder is on the volume of a Copy or Move destination, a Move's
  source folders or a Delete's folder, with every progress event from Running on
  and with the terminal one. The volume is matched by text:
  the longest listed volume root that contains the path (the Home location is a
  folder, not a volume), else its drive, UNC share or root. A link or mount the list
  does not name can hide another volume; a fixed disk's periodic read, the next
  focus or the next listing then shows the change. A request for a pane that is
  loading waits until its listing commits, because that listing's read may predate
  the change; a request for a location the pane has left is dropped.
- **Periodic reads.** While the window is visible (`Command::SetWindowVisible`,
  sent when a native focus or resize event finds it minimized or restored; Wayland
  cannot tell, so the window counts as visible there), a pane whose storage kind is
  `LocalSsd` or `Rotational` (an internal SATA, ATA or NVMe disk) is read every 5
  seconds. `Local` (USB disks, card readers, virtual disks), `Network` and `Unknown`
  storage is never read periodically.
- **Following a request.** While the window is visible, a request keeps a pane on a
  local disk (`LocalSsd`, `Rotational` or `Local`) followed for four seconds: its free
  space is read again 0.2, 0.6, 1.4 and 3 seconds after the request (each gap twice
  the previous one). A filesystem can report the space a deletion frees only after
  the deletion returned: on ext4 (Linux), `statvfs` right after `unlink` of a large
  file can still report the old value, and reports the freed space about 50 ms per
  GiB later, longer while the disk is busy. The read that a Delete's completion or a
  watch hint's re-read makes at once can therefore miss it; the following reads show
  it. A later request starts the sequence again, so a folder whose watch hints keep
  coming costs a few reads a second.
- **Cost bounds.** Each pane has at most one read in flight. Requests that arrive
  meanwhile, or sooner than the pane's spacing after its previous read ended (a fifth
  of a second on a local disk, whose free space the kernel answers from its counters
  in microseconds; a second on network and unclassified storage), leave one read
  owed, which starts once that spacing has passed, so the last change is always read
  and a slow share is never read back to back. Reads run on the blocking pool
  under the engine's auxiliary budget (two permits), never on the command loop or
  the UI thread; a newer navigation aborts the waiting part of an older read, whose
  native call keeps its permit until it returns. Inputs are a watch slot, so no
  queue grows. An answer equal to the last one sent for the navigation is not sent,
  so periodic reads of an unchanged disk send the UI nothing; the window's text
  property also ignores an equal value.
- **Display.** A re-read of the shown folder keeps showing the previous value until
  its own answer arrives; another folder shows "Free space —" until its first one.

The drive buttons' tooltips and the drive menu use a separate per-volume cache
(`volumes::Controller`, `Event::VolumeDetailsUpdated`) that reads every listed
volume's label and free space again every 45 seconds, and a new volume's (or one
whose read failed) when a drive-change notice arrives; it is never matched to a
panel's value by path. The drive list itself follows native drive notices
(`WatchService::watch_volumes`; see
[when drives come and go](../drive-information.md#when-drives-come-and-go)).

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
ordinary backslashes in paths retain their exact meaning. Copy details copies the
message the notice shows (its title, the reason and the folder) and then the complete
generated report, which the clipboard worker reads; the preview's size line counts
both. Other panel diagnostics can load bounded report previews
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
retained receipt IDs through the directory-listing service; no displayed
path grants authority. Closing/replacing the message, a new error occurrence or
navigation prevents an obsolete dialog callback from authorizing anything.

The Windows helper uses the UAC prompt; Linux uses the desktop polkit agent through
`pkexec`. An engine without an authorization service does not advertise the button.
Denial, cancellation and queue failure remain explicit errors and keep the current
listing; there is no silent retry, ACL change or privileged host launch. The helper grants one read-only directory listing and does not
grant file content or mutation permissions. It runs only from an installation that
no other account can change: with the helper's digest recorded in the executable, the
shipped helper in a folder that only the user and administrators (root) can change;
without it, an administrator-protected (Windows) or root-owned (Linux) installation.

Before the offer, the listing worker asks the service's `availability()` (the same
checks, without starting anything; cached per process) and sends it in
`Event::DirectoryElevationAvailable { availability, .. }` right after the
navigation's `ListingFailed`. With `ElevationAvailability::Available` the engine keeps
the receipt and the presenter offers the button and the hint that the operating system
will ask. With `Unavailable { reason, details }` the engine keeps no receipt; the
presenter (`explain_unavailable_elevation`) appends `reason` to the shown message
("Access to this folder was denied. Opening it needs administrator permission, which
this installation cannot ask for: …"), `details` (the refused file and why) to its
details and copied report, and shows neither the hint nor the button. Hiding the button
rather than disabling it keeps the explanation readable without hovering a disabled
control, for keyboard and screen-reader users alike, and in Copy details. A missing
helper, as in per-user deployments without it, is reported as missing; an altered one
as not the shipped helper; a folder that other accounts can change as such. The
retry runs every check again and can still be refused (for example when no polkit
agent runs); the presenter
(`show_navigation_error`) shows an ordinary access-denied failure as "Access to this
folder was denied.", but for the navigation that an authorized retry started it
shows the failure's own reason, so that refusal states the requirement too. When
the folder was opened from a row of a flat native listing, that row's red
diagnostic icon also shows it. Contract:
[directory authorization helper](../../SECURITY.md#directory-authorization-helper).
Evidence: [authorization record](../verification/2026-11-11-directory-authorization.md).

The rows of an authorized listing are a static snapshot of a folder that the host
cannot read. In 0.6.3 and earlier the pane's watch subscribed and checked that folder like any
other; the check was denied, a failed check counts as a change, the hint re-read the
folder with ordinary rights, and the backend's recovery of that denial
(`navigation_fallback`) moved the panel to the parent folder about a second after the
listing appeared. Now the engine dispatches the retry with
`watch::Controller::authorized_listing_started` (a re-sort of its rows carries the mark,
`rows_resorted`), the listing reports no read, and the watch keeps no target, check or
status for those rows once accepted (`Visible::authorized`). The presenter
(`presenter/authorized_listing.rs`) records them per committed navigation: automatic
re-reads (`refresh_if_changed` for watch hints, Show hidden files, removed drives and
dropped trees; post-operation re-reads) skip them, the status row says *Listed with
authorization; not updated automatically. Refresh offers authorization again.*, Refresh
sends a `Navigate` (only a navigation's denial is offered a retry) whose denial opens
**Cannot open folder** while the rows stay, and a denied re-read never follows the
backend's parent fallback. Opening a subfolder is an ordinary navigation with its own
offer. Tests: `crates/midnight-engine/tests/elevation.rs`
(`an_authorized_listing_is_neither_watched_nor_checked_with_ordinary_rights`),
`crates/midnight-engine/src/watch/authorized_tests.rs` and
`crates/midnight-ui/src/presenter/tests/elevation.rs`.

A generated report title is only a display label and never authority to reopen a
native path. Missing size-report viewer support and diagnostic read failures are
reported explicitly. If the initial settings read failed, the disabled-viewer
policy is unknown and viewers stay unavailable for the session; reports, F3,
Find, Make File List and Compare then say so instead of asking to try again.

Evidence: [tree diagnostic details record](../verification/2026-11-10-tree-diagnostic-details.md).
