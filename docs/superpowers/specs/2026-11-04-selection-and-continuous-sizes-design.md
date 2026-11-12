# Selection shortcuts and continuous folder sizing

The user explicitly authorizes implementation of Ctrl+A/Ctrl+D/Ctrl+I, removal
of folder-size exclamation prefixes, inline drive shortcuts, and scans that keep
running during sorting and ordinary work in a pane. This extends FM0.1 without
enabling file-writing operations or changing platform boundaries.

## Selection

Ctrl+A selects top-level files only, Ctrl+Shift+A includes top-level folders.
Ctrl+D clears all selection, including folders and expanded descendants.
Ctrl+Shift+D has no default assignment.
Ctrl+I inverts
**files only** in the focused pane, preserving both selected and unselected
folders and directory links. Archives remain file entries in their parent
listing; archive members use the same selection policy. The synthetic parent
`..` is never selected. This matches Salamander's `InvertSelection(FALSE)` with
directory inclusion disabled (`src/fileswn1.cpp:1785`). The
opposite pane, cursor, paths, sort and command draft remain unchanged. Exact
selection chords apply in file lists; path/command/options editors retain
normal text editing, including Ctrl+A.

Ctrl+A is the configurable `selection.files` action: replace selection with
displayed top-level non-directory entries, clearing folders and nested marks.
Ctrl+Shift+A uses `selection.all`, including top-level folders and directory links.
Ctrl+D uses `selection.clear`, removing every mark at every displayed depth.
Ctrl+Shift+D is unassigned. Select files, select files and folders, and clear all
appear in F10 → Selection and Options → Keyboard without numeric-keypad defaults.
The files-only clear action remains configurable without a default shortcut,
preserving deliberate saved assignments.

An authoritative backend selection controller serializes bulk and individual
toggle intents for each pane. Idle actors await commands asynchronously; each
pane has at most one active `spawn_blocking` CPU job and never holds an idle
blocking worker. The UI moves its cursor immediately and reads an
immutable selection snapshot; it never builds a million-entry set, clones that
set on a click, or releases its large storage synchronously. Backend bitsets
bound storage to approximately one bit per entry, plus existing shared indexes.
Snapshots answer membership by original raw paths so refreshes cannot select a
different entry merely because its numeric ID was reused. New entries appearing
after a refresh are not selected implicitly.

Requests/results carry navigation and selection revisions. Rapid all→invert,
repeated hotkeys, mouse toggles and navigation must preserve intent order;
stale results never overwrite newer work or another tab. Queue refusal is
visible and does not silently change selection. Streaming selection tracks
independent file/folder transformations and reports typed defaults for unseen
rows. Invert and ClearFilesOnly affect only the file transformation;
All/Clear affect both.
Explicit snapshot bits override those defaults. The UI applies the same policy
immediately to visible rows and later arrivals without scanning the listing.
Final rebase resolves all paths; actions never block input or apply after leaving
the location. Individual streaming toggles and failed-navigation restoration
retain their existing semantics. Filtered views constrain inversion to their
captured entries; the action remains user configurable.

Rebasing selection across sorted/refreshed listings and expanding selected paths
for Properties belongs to backend workers. Existing bounded queues, cancellation
and backend reclamation remain in use. No native filesystem call is introduced
into UI code. Windows and Linux share platform-neutral selection behavior.

## Folder-size display and scan lifetime

Remove `!` prefixes from running, queued and unfinished folder-size strings.
Keep the contrasting active-scan color, approximate `≈` known totals, queued `…`
and unfinished `—`. Completed partial totals retain `≈` with ordinary styling,
so they differ both from a running scan and from a clean final value. File/folder
rows have no hover tooltips; F1 explains these states. Errors/cancellation never
masquerade as complete data.

Selection, cursor movement, active-pane switching, column sorting and same-path
refresh do not cancel an active directory-size scan. Scan identity is separate
from display/sort generations. Results remain bound to the scan's native path,
original entry identity and scan ID, and are mapped to the currently visible
raw entry path. A reused numeric ID is insufficient. Explicit cancellation,
leaving/retiring the location or application shutdown can stop the scan.

Sorting uses cached entries and the ongoing scan's shared size results, with no
new directory traversal merely because a header was clicked. Filesystem changes
can invalidate completed estimates, but do not restart a healthy active scan
after every notification. Totals remain live estimates rather than atomic
filesystem snapshots. Existing SSD/HDD/network budgets are unchanged.

Intermediate backend progress is limited to250ms per folder. One shared UI
presentation timer merges size-only changes from both panes and repaints at
most four times per second while idle; completion is delivered at the next
pulse. Input and other backend events repaint immediately using the current
cache. The presentation timer never delays selection, navigation or errors.
Progress and scan completion never trigger sorting or replace the displayed row
order, even when the selected sort column is Size. The user can sort explicitly
using the header or its shortcut to apply the newly completed totals. A normal
directory reload still uses the chosen sort specification.

## Drive controls

Move the quick drive/location buttons from their separate top row directly next
to the drive dropdown. Keep native system icons, short drive captions, complete
volume/free-space tooltips, accessible labels and raw location identities. The
strip can scroll horizontally when space is limited; Back/Forward/Up/Refresh
and bookmarks remain usable. Desktop and compact Touch metrics are preserved.

## Verification and repository contents

Meaningful tests cover both panes, editor/popup isolation, excluded parent row,
rapid operation order, queue pressure, streaming, refresh/sort identity changes,
newly created/deleted paths, Unicode/native paths, stale results and reclamation.
Use blocked-scan fixtures to prove repeated sorting/refresh retains progress and
does not cancel/restart traversal. Exercise actual keyboard/pointer events.

The user also authorizes an initial Git commit. Commit source, tests, Cargo
manifests/lockfile, CI/build scripts and intended documentation. Standard Windows
build outputs are under `target/`; local Linux outputs/toolchains and helper
logs/caches are under `.tools/`, including `.tools/linux-target/`. Both directories
and `.superpowers/` execution evidence are ignored. No executables, objects,
libraries, dependency caches or temporary QA captures are staged. Intentional
documentation screenshots are distinct from temporary build output.
