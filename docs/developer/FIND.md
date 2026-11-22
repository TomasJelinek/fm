# Recursive Find service and UI

The source implementation exposes Masks, Regex and typed Expression matching,
reusing the panel compilers, metadata semantics and budgets. Results use one
compact aligned row per entry. The mode applies to names and metadata; the
separate content query is literal text or, with **Regular expression**, a bounded
regular expression matched within each line (see
[Content regular expressions](#content-regular-expressions)). **Search inside
archives** also matches archive members, and **Feed to panel** shows the results as
a list in a panel (see the sections at the end). The design record is
[panel/viewer follow-up requirements](../superpowers/specs/2026-11-07-panel-viewer-followups.md).

`midnight-core::FindService` is an internal Rust backend capability, not a stable
external plugin ABI. Windows and Linux implementations live in separate native
modules. The engine accepts `StartFind`, `SortFind`, `CancelFind`, `CloseFind`,
`ValidateFindResult`, `CaptureFindFile` and explicit `ExecuteFindFile`; the UI only
captures input and displays shared results.

Each of at most four modeless windows owns a monotonic session ID. Every accepted
search increments its generation. Up to two workers stream ordinary batches of at
most 64 stable-ID results through bounded channels; a smaller pending batch is
published with each progress sample (at most every 250 ms), so sparse matches
appear while traversal continues. Duplicate groups publish as
one complete batch, bounded by the 10,000-result limit, so Stop cannot leave half
a group. Presenter and controller both
reject late generations. Stop retains results: the final partial batch found
before the worker observed Stop is still published for the same generation, and
the retained results receive one automatic sort in the current header order.
Close releases the session, and host shutdown closes the windows. Queue rejection never pretends cancellation
or close succeeded. Large result allocations are reclaimed on backend workers.
Replaced row-model order owners enter a small interior-mutable retirement queue:
refresh already holds a shared presenter borrow, so it must not request a mutable
presenter borrow while notifying Slint. The normal `take_retired` handoff drains
that queue for worker disposal, including streamed reorder and search restart.

Roots retain raw native `PathBuf`s independently of their display text. The
Search folders dropdown lists at most 16 roots, each with a checkbox: only ticked
roots enter `StartFind`, and Find with none ticked is refused before the backend
queue. Unticked roots still count toward the 16-root and 1 MiB request bounds.
Add, on the Search folders line before Remove, opens a one-line entry for a typed
absolute path (Enter or OK) with Browse…, which sends `PickPath` for a native
directory bound to that Find session; a late or foreign chooser answer is only
retired, and closing the window cancels an open chooser first. Adding a listed
path ticks it instead of duplicating it. Either way the host reports the row
(`root-added`), and the window chooses that folder, closes the entry and moves
the focus to Search folders; a refused path keeps the entry open with its
error. A narrow window puts Browse… and OK under the typed path. The dropdown
fits the room below or above the picker and scrolls the highlighted folder into
view. Remove removes the folder chosen by clicking its name in the dropdown. Unchanged paths never round-trip through a
lossy text editor, so names that cannot be typed keep their original native path. Masks and Regex use their existing filename compiler; Expression
uses captured native name/kind/size/time/attribute facts and preserves unknown
metadata semantics. A nonmatching directory is still traversed. Patterns are
bounded non-executable input. Content is literal UTF-8 (optional BOM) or UTF-16
little/big endian with a BOM, streamed through a 64 KiB read buffer with strict
incremental decoding and KMP. The compiled query and read/decoder workspace are
reused across files. Optional whole-word matching checks both surrounding word
boundaries, including Unicode combining marks/connectors and case-fold expansions;
plain substring matching remains the default. A found match never skips trailing
encoding validation or native revision checks.
On Linux a chosen location may be reached through symbolic links in its ancestors
(merged-/usr `/lib`, a folder entered through a directory link); procfs magic
links are refused and the location itself must not be a link. On Linux a
directory's visible binding is checked once when its enumeration starts, so
entries created, removed or renamed inside it during the search do not end its
traversal; directory results stay valid while their contents change, and each
result is still revalidated by its own path, identity and (for files) revision.
See the [backend design](../superpowers/specs/2026-11-07-recursive-find-design.md)
for budgets, skipped mount/link policy, revisions and sampled diagnostics.

Modified-within accepts a positive whole number of minutes, hours or days (at most
366 days). The backend freezes the reference time when accepting the request,
before waiting for a worker. A captured modification time matches the inclusive
interval from that time minus the chosen age through that time. Future and
unavailable modification times do not match; a long scan does not move the cutoff.
This predicate combines with name, metadata expression and content filtering.

Duplicate mode considers only eligible regular files and counts each native
identity once, so repeated/overlapping roots and hard-link aliases do not create
false duplicate groups. Names and extensions may differ. It first removes
singleton size buckets, then hashes candidates with SHA-256 and compares complete
byte streams within each equal-size/hash bucket. Binary and empty files are
supported. Each comparison reopens and validates both captured identities and
revisions; all members are validated again before publication. A failed comparison
or changed representative discards that digest bucket, reports an incomplete
search, and cannot leave an earlier pair presented as verified duplicates. These
are live validated reads, not a filesystem snapshot or exclusion of other writers.

Duplicate candidates have a separate 100,000-entry ceiling and share the 64 MiB
retained-memory budget with directory frames, diagnostics and results. Candidate
vector/hash capacity, path ownership, comparison/group scratch and result
ownership are charged before retention. Hashing and comparison use two reusable
64 KiB buffers, never whole-file allocations or one open handle per candidate.
Every actual read counts against the shared 16 GiB content budget; each file is
limited to 1 GiB. All limits can be lowered. A group that exceeds remaining result
capacity is omitted as a whole with an explicit limit; later (larger-size) groups
that still fit are compared and published. Group size describes the
eligible identities discovered by this search, whose incomplete status remains
visible when traversal or resource limits omit part of the requested scope.
Duplicate results use the same explicit navigation, View, Edit and Open actions as
ordinary results, with no original/redundant-file or reclaimable-space designation
and no automatic deletion.

`files.find` defaults to Ctrl+F and Alt+F7 and appears in the configurable command catalogue.
An existing explicit Alt+F7 assignment takes precedence over this new default;
the magnifier and configurable action remain available without resetting settings.
The magnifier opens a movable native window; both panels remain usable. Lazy
Slint row models format only requested rows. Form drafts stay in their own window
and are not reset when progress or another panel changes. Presentation revisions
avoid repeatedly formatting unchanged diagnostics during ordinary panel input.

The separate scanning line shows the current native directory, or current file
while hashing/comparing duplicates, above the count progress. Scanning, hashing
and comparison phases expose candidate/group/read-byte counts. A worker updates a
single raw-path slot before opening a root, enumerating a directory or processing
a duplicate candidate. The async controller samples it at most four times per
second even while native directory/content I/O is blocked. The slot has a 1 MiB
resource bound; no path is truncated or used as an operation target. Status
samples skip a full output queue, and generation/cancellation guards prevent old
searches from reviving after Stop, completion, restart or Close.

Result modification times use the native date and time format of panel
listings. The search worker fills a missing localized time through the engine
locale once when it retains an ordinary result or publishes a duplicate group,
so a duplicate candidate that joins no group, or whose group the result limit
omits, is not formatted. A group is formatted before its strings are charged, so
the one group that the retained-bytes budget refuses, which ends the search, is
formatted without being shown. Go to file formats the validated entry it reports
the same way. A time the provider already formatted is kept; the native Windows
and Linux providers format none, so examined nodes that never match cost no locale
call. Without an engine locale, an unformatted time shows the labelled UTC fallback.

Name, Extension, Path, Size and Modified headers toggle ascending/descending
sorting. Text keys use the same captured engine locale as panels; numeric sizes
and native timestamps are compared directly, with unknown values last in both
directions. Extension uses the panel Type semantics: directories first, missing
extensions last in either direction, native locale keys with raw fallback, then
ascending locale filename and raw filename ties. Missing locale keys stay after
keyed text in either direction. Dotfiles without an extension and explicit empty
extensions retain the same distinction as panel entries. Native raw paths/IDs
finish deterministic ties; unrepresentable names stay raw.
Duplicate groups stay adjacent under every column sort: the first member in the
selected sort order positions the group, and members follow that same order.
Immutable group numbers/counts stay attached to rows when ordering changes.
One fixed, coalescing backend sort task per session prepares keys, bounded to
16 MiB aggregate and 4 MiB per key. Keys prepared for a generation and column are
reused by its later re-sorts, so a streaming search derives native locale keys
only for new results. The 10,000-result authority remains in discovery
order for revalidation. A separate immutable ID permutation and inverse index
lets the UI preserve the selected stable ID in constant time during reorder,
including newly streamed tails. Stale generation/revision/count orders are
discarded, and a failed sort retains the previous visible order with an error.
Extension and secondary filename keys share the aggregate key budget; sorting
and native locale calls remain entirely on the backend worker.
Narrow windows horizontally scroll the aligned columns; Close stays separate
from the scrolling actions. Stop retains results for subsequent read-only use.
Every result column (Name, Extension, Path, Size, Modified and, for identical
files, the group column) resizes by dragging the separator after its header; the
widths are kept by that Find window for its session, and a separator drag never
sorts. Path also takes the list's spare width, and a drag of its separator
starts from that shown width. Only header cells carry separators; result rows
just keep the 10 px gap. Help beside the mode chooser (or F1) covers the results
with syntax help for the chosen Masks, Regex or Expression mode and takes the
keyboard focus, so result keys never act on hidden rows; Escape, F1 or Close
help returns the focus to the results. Clicking an example puts it in the query
without searching, and Find closes the help. Escape first closes the help or the
Add entry, and only then the window.

Go to file sends retained session/generation/result IDs, not authority reconstructed
from displayed paths. The backend reopens and validates identity/revision. The UI
checks the pending token/generation again and navigates the originating panel to
the containing directory with a wanted cursor. This is read-only navigation; it
does not execute or edit the found file. The separate View, Edit and Open controls
request `CaptureFindFile` with the retained session/generation/result IDs. The
capture preserves the exact native result and provider, and rejects directories,
changed identity/revision and obsolete sessions. A navigation validation receipt
never authorizes one of these actions.

View uses the configured `files.view` binding (F3 by default), with
`files.view-basic` (Alt+F3) selecting the ordinary plain-text viewer. Edit uses the
configured `files.edit` binding (F4). Hints and keyboard resolution use the current
shortcut settings. These shortcuts operate while the result list has focus.
Open is an explicit button and may execute a program through its system association.
Enter and double-click retain Go to file behavior. Folder actions use Go to file;
View/Edit buttons require a regular file. No Find action automatically deletes a
file or interprets a displayed path as native authority.

The viewer backend validates the capability before and after capture through the
installed native filesystem provider and matches its captured stable identity and
revision. Only the selected result enters the viewer; no panel path or unrelated
panel ordering is substituted. The ordinary bounded viewer pipelines handle
content; Find does not read an arbitrary file into memory. Native Open/Edit
validate immediately before launch on the existing desktop workers, reuse the
ordinary launch/process limits and pass the captured raw path as a literal target.
External applications still consume a live native pathname: validation is not an
atomic filesystem snapshot and cannot prevent namespace substitution after that
check or edits by another writer. The existing platform launch limits remain.

Sixteen retained action captures share the startup-reserved engine retirement
domain. Each capture holds its admission permit until actual worker disposal,
including stale replies, failed launches and runtime shutdown. The existing
16-request validation queue admits capture requests too; two backend workers
validate concurrently, so one unresponsive network open cannot stall the actions
of every Find window. Stop retains file action availability; accepted restart,
Close and shutdown invalidate obsolete generations and cancel their queued
validations' cooperative provider checks. UI generation/token checks and cancellation prevent a late
capture from opening a different search result. Queue rejection leaves a visible
retryable error. See the [action verification](../verification/2026-11-12-find-actions.md).

On Windows only name-surrogate reparse points (symbolic links, junctions, mount
points) are skipped as links. Cloud Files/OneDrive placeholders, deduplicated and
WOF-compressed entries are searched as ordinary files and directories; their content
is read through the filesystem filter, so a content or duplicate search downloads
online-only placeholders it has to read. A directory's identity binding is
revalidated once per native enumeration batch and at its end; entries created,
removed or renamed inside it meanwhile do not abort its enumeration, because a
directory's revision carries no content state. Each result is still revalidated
before publication.

Current scope is native Windows/Linux roots; archive members are searched only with
Search inside archives (below). Other providers, legacy content encodings, BOM-less
UTF-16, hex content queries and automatic file execution are not enabled. System file activation requires the explicit Open action. Unsupported content, access errors and reached budgets produce
an explicit incomplete result, counts and details; no silent complete claim. An
error appears as red text with an error icon, and the status line under the
results summarizes progress; clicking either (tooltip "Click for more details")
shows the full error, counts and diagnostics with Copy and Back. There is no
separate Details button. A window shorter than 360 px with text larger than
20 px shows the status in the query area instead, pinned to its last line so that it
is in view whatever the query area scrolls to, except while a search runs: the query
area is then hidden, so the status stays under the results. The scrolled query form
starts at the top of the query area and ends above that status line, so the line covers
no control of the form.

The footer shows its actions through `DialogFooter`'s fitted actions (see the
[dialog ergonomics](DIALOG_ERGONOMICS.md)): every action that fits whole beside Close is a
button, 8 px from the next, and the others are in a More… menu at the end of the row,
with their captions, glyphs, shortcuts and enabled states. On the query page Copy
details moves first, then Open, Edit, View and Go to file; Find or Stop stays. On
the details page Copy moves first, then Find or Stop; Back stays. At 320×200 the
query page shows Find and More…; at 1000 px every action is a button. Where even Find
(or Back) and the More… glyph do not fit beside Close, as at 320×200 with a 32 px font on
Linux, Find (or Back) shows its glyph alone, then Close does too, and in a footer too
narrow even for that every action is in More… (the dialog ergonomics list the steps).

Tests cover blocked I/O, sorting, Stop before a sort and header geometry, and
software-window pointer, key and geometry checks on Windows and Linux (the mode
chooser, populated rows, all five headers and minimum-size views); software tests do
not establish native desktop or GPU behaviour. The backend tests cover relative
minute/hour intervals, future/unknown times, same-size unequal content, binary and
empty duplicates, hard links and overlapping roots, group-preserving sort, 70-member
atomic publication, Stop/publication boundaries and explicit budgets, with native
Windows fixtures for contents, hard links and modification times and simulated read
denial, short reads, substituted captures, equal-hash/unequal comparison streams and
late representative changes. Dated evidence: the
[refactor record](../verification/2026-11-08-find-refactor.md), the
[crash and extension record](../verification/2026-11-08-find-crash-extension.md) and
the [operations record](../verification/2026-11-10-operations-find.md).

## Selecting several results

The result list selects several results at once, with the conventions of the Checksums
results list; both use the shared model in `crates/midnight-ui/src/list_selection.rs`.
A click selects one result and makes it the cursor and the anchor; Ctrl+click toggles a
result; Shift+click selects the range from the anchor to the clicked result, replacing the
selection (Ctrl+Shift+click adds the range). Up/Down, Page Up/Down, Home and End move the
cursor and select the result they reach; with Shift they select the range from the anchor,
with Ctrl they move the cursor alone. Space toggles the cursor's result, Insert toggles it
and moves the cursor down, Ctrl+A selects every result shown and Ctrl+D none (the cursor
stays). Ctrl+Space is not bound. Selected rows look as in a panel, through the theme tokens:
`Theme.selected-background` and `Theme.selected-text` with a `Theme.selected-edge` square at
the left (7 px, 9 px in touch density); the cursor's row has a 3 px `Theme.cursor-edge`
stroke and a 1 px `Theme.cursor-edge` frame, and `Theme.cursor-background` when it is not
selected. Every row reports its selected state to accessibility
(`accessible-item-selected`), and an accessible activation selects the row as a click does.

The presenter keeps the selection by result identity (`FindSession::selection`; a result's
slot is its ID minus one, its index in `FindSession::results`), as one bit per result, so a
lookup is one bit test. Results that stream in arrive unselected, a re-sort moves the
selected rows without changing what is selected, and a new search clears the selection,
cursor and anchor. The window does not put the marks into the formatted result rows: a
separate `[bool]` model (`Marks` in `find_windows.rs`) answers each visible row from the
selection, and every selection change notifies it once (`ModelNotify::reset`), whatever its
size, so Select all and a range re-read only the visible rows' marks and never format a row
again. Selection changes refresh the Find windows only, not the whole application. The
status line adds how many results are selected and the size of their files ("· 3 selected
· 12 B"; folders count without a size).

An action acts on the selection when the row it acts on is part of the selection, and
otherwise on that row alone:

- **Ctrl+C** copies the full paths of the selection that holds the cursor's row, else of
  that row. The **context menu** (right click, Shift+F10 or the Menu key) offers Go to file,
  View, Edit, Open, Copy names, Copy full paths, Copy row details (the columns separated by
  tabs, the identical-files group first in a duplicate search), Select all and Deselect all;
  its copy actions name how many results they copy. A right click on a row outside the
  selection selects that row alone first, so the menu acts on what is shown selected.
  Copies use the raw names and paths, one line per result in the order shown, built in one
  pass over the shown order.
- **View** (F3, Alt+F3), **Edit** (F4), **Open** and **Go to file** (Enter, double click)
  act on the cursor's row only, as F3, F4 and Enter act on the focused file in a panel with
  several selected files; the menu's versions act on the row it opened on.
- **Feed to panel** feeds every result, as before. When the selection is more than the
  cursor's row, the selected results arrive marked in the panel's list, so F5 and F6 (the
  Copy/Move dialog counting the marked results: "Copy 3 files (12.4 MiB) to:"), F8 and
  Shift+F8 (the Delete confirmation naming the count), the clipboard and Properties act
  on them there. A
  selection that only follows the cursor marks nothing; the list's cursor starts on the
  Find's cursor result either way.
- The Find window itself offers no delete and no copy or move: those go through Feed to
  panel. Results cannot be dragged, from the Find window or from the list fed to a panel.

Two ignored measurement tests, `find_selection_measures_100k_results` and
`find_window_marks_measure_100k_results` (run with `--ignored`), time the selection with
100,000 results, ten times what a search keeps. In a Windows release build, best of
five: a click 1 µs; Ctrl+A 1.4 ms; a Shift+click range over every result 1.4 ms; a
Ctrl+click in a full selection 1.1 ms (the status line's byte total is summed again);
100,000 mark lookups 89 µs; after Ctrl+A or such a range, the one notification and
re-reading a visible page of 40 marks 1.6–1.7 ms in all (formatting those 40 rows takes
48 µs and is not repeated); copying 100,000 full paths 21 ms and their row details
37 ms.

The selection's presenter tests are `presenter/find_selection_tests.rs` (identity while
results stream and re-sort, clicks and ranges, keys, Space and Insert, Ctrl+A and Ctrl+D,
the menu, copies on the selection or the acted-on row, Feed to panel's marks and its
Delete of exactly those results), the mark model's
`marks_read_the_selection_by_identity_and_notify_once_per_change` in
`find_window_order_tests.rs`, and the window tests in `tests/find_windows.rs`: pointer,
keys, accessibility state and the status line, Ctrl+C, the menu by pointer and by
keyboard and Feed to panel, all through the real presenter, and the selected and cursor
rows told apart in every theme. The footers' fit is
`find_footers_show_whole_evenly_spaced_actions_and_move_the_rest_into_more` in
`tests/find_narrow_layout.rs` (320×200, 560×420 and 1000×620, both densities, a 32 px
font at 320×200, More… by pointer and from the keyboard).

## Content regular expressions

**Regular expression**, beside Match case and Whole words under Containing text,
turns the content query into a regular expression (`FindContentQuery::regex`). It
uses the workspace's `regex` crate through
`midnight-platform-common::content_pattern::LinePattern`, with the panel filter's
bounds: a 2 MiB compiled-program and lazy-DFA ceiling, nesting depth 64 and the
4096-byte query limit; matching is linear in the searched text. The pattern is
compiled once per search on the worker. An invalid pattern ends the search as
Failed with "Invalid content regular expression: …" and the library's explanation
(with a caret under the offending part) on the details page that the red status
line opens.

The decoded text (UTF-8, BOM-marked UTF-16) is split into lines like grep: each line
is matched on its own, a trailing carriage return is not part of the line, the empty
remainder after a final newline is no line, and a match never spans lines. Only one
line is retained; a line longer than 1 MiB is matched in 1 MiB windows that overlap
by 4 KiB, so a match longer than the overlap that crosses a window edge can be
missed. Match case switches case-insensitive matching (Unicode simple case folding)
off; Whole words requires a word boundary on both sides of the match (the pattern is
validated alone first, then wrapped as `\b(?:pattern)\b`). The per-file and total
content budgets, revalidation and encoding rules are those of literal content
search.

## Search inside archives

**Search inside archives** (More options) also matches the members of archives that
the walk meets: files with a built-in archive suffix (ZIP, 7z, TAR and its
compressed forms, GZIP, XZ, BZIP2, Zstandard) for which an installed archive plugin
is available. Each archive gets its own provider session on the search worker,
opened from the native capture (`find_installed_archives.rs`; the plugin catalogue is
discovered once, on first use), searched, and released on the same worker; panels
and viewers never share these sessions. The plugin host counts them in an allowance of
their own (`Mount::open_for_search`): at most two at once, since at most two searches
traverse at once and each opens one archive at a time, beside the two archives that
panels, tabs and viewers may have open and the three archives inside archives. So Find
searches inside archives while both panels show archives, and a running search never
makes a panel refuse one. The archive file itself is revalidated after opening, so a
replaced file is not searched. Members are matched like native
entries: name masks or regex, metadata expressions (kind, size, the archive's time
when it carries a time zone), Modified within, and content streamed through the
provider in the content reader's chunks. Identical files searches native files only;
the option is disabled with it. Subfolders limits recursion into ordinary folders only:
an archive found in a searched folder is searched completely, its member folders
included, whatever Subfolders says, while with Subfolders off an archive in an ordinary
subfolder is not reached.

Find sets no limit of its own on archives: every archive the walk meets is opened,
however large, and every member is examined, until the search ends or Stop. The
search's budgets (`FindLimits`) apply to members as to native entries: members count
as visited entries, folder levels inside an archive count against the depth limit,
folders waiting to be listed count against retained memory like native directory
frames, and the result, memory and content budgets are shared with the native walk.
Neither the bundled archive plugin, the plugin host nor the engine (Find, panels,
copy-out and folder sizes alike) limits how many members an archive has or a folder
lists, or how long their names are: the host pages through every folder, and only
the worker's 1 GiB of memory, the most the worker SDK grants, bounds an archive's
index (about nine million TAR, two million 7z or one and a half million ZIP members;
see the [archive worker](../../plugins/midnight-fs-archive/README.md)). A version 1 plugin
keeps its legacy limits, including sources of up to 64 MiB. An archive or folder
that cannot be opened or listed, for example one that needs more memory than its
worker has, is reported in the details, and the search goes on.

Stop reaches the provider's work through the search's cancellation, so it stays
prompt inside a huge archive: it ends the opening of an archive (when the worker
indexes the whole archive, reading all of a TAR or compressed stream) by ending its
worker, a listing between pages, and the staging of a member for its content at the
worker's next staging checkpoint (protocol 3; older plugins finish that member
first). The walk checks Stop before each folder and member. A stalled provider is
stopped by the plugin host's own watchdog. Nested archives (an archive inside an
archive) are not opened. Without an archive provider (no plugin, or a host without
archive capture) the first archive met adds "Search inside archives is unavailable"
to the details once, and the search is Incomplete rather than silently complete.

A member result (`FindResult::archive`) shows the archive's path joined with the
member's names in the Path column; that joined path is presentation only and is
never used as a native target. Its `FindArchiveMember` holds the provider address of
the member's folder, the member's raw name and the archive file's captured identity.
Go to file revalidates the archive file (identity and revision) on a backend worker,
then opens the folder in the panel that started the Find through the ordinary
archive navigation (`ArchiveAction::List` of that address, which reopens the archive
at the searched revision) and puts the cursor on the member once the listing
arrives. Leaving the archive returns to the folder the panel showed before. View,
Edit and Open are disabled for members, and a capture request for one is refused
with "An archive member opens from its archive"; view or copy members from the
archive panel. Archive search has unit and integration coverage with a fake provider
session (`crates/midnight-engine/tests/find_archives.rs`), including 1,001 archives
and an archive with over 100,000 members searched completely, waiting folders charged
to retained memory, and Stop while a slow provider opens, lists or stages a member;
the acceptance scenario `find_archives` searches many and large archives through the
installed archive plugin, and `tools_find_results` checks the explained "unavailable"
outcome without one ([Find record](acceptance/find.md)). The plugin host's tests list a folder of 100,001 members across
pages, the plugin's tests open ZIP, TAR and 7z archives of 100,001 members, and an
ignored host test (`real_sandboxed_worker_lists_archives_of_millions_of_members`)
lists generated archives of millions of members through the sandboxed worker.

## Feed to panel

**Feed to panel** (on the More options line, right above the results; enabled when a
finished or stopped search has
results) shows the results as a list over the panel that opened the Find, like Total
Commander's feed to listbox, and closes the Find window. The list is a virtual
"search results" location: the panel's own folder stays loaded underneath, and
leaving the list (Backspace, Back to folder, Alt+Left or the parent and root
commands) shows it again unchanged. Any navigation of that panel to another location
(a path, a drive, a bookmark, a tab, history) also leaves the list. The rows are the
native results in the Find window's sort order; archive members are left out and
counted in the status line ("1 archive member not shown", "2 archive members not
shown"). The results selected in the Find window arrive marked, unless only the cursor's
result was selected, and the list's cursor starts on the Find's cursor result (see
[Selecting several results](#selecting-several-results)). The list is a snapshot of the
results: it is not updated by later file changes.

Keys keep their configured meaning: Up/Down, Page Up/Down, Home/End move; Insert
marks and moves, Space marks in place, the bulk selection keys mark with a panel's
rules (`SelectionOperation`): select files only (Ctrl+A) replaces the marks with the
files, invert files only (Ctrl+I) keeps folder marks, and Ctrl+Shift+A, Ctrl+D and
Ctrl+Shift+I select, clear and invert files and folders; Ctrl+click and Shift+click
mark with the pointer. Enter (or a double click) goes to the focused result: a
folder is opened, a file is shown in its folder with the cursor on it, and the list
is left. The actions act on the real files through the ordinary backend flows:

- View (F3) and plain view (Alt+F3) open the focused file in the viewer, whose Space
  and Backspace walk the results; Edit (F4) opens the configured editor.
- Copy (F5) and Move (F6) open the ordinary Copy/Move dialog with the marked results
  (or the focused one) as captured native paths, to the other panel's folder.
- Delete (F8, to the Trash) and permanent delete (Shift+F8) enter the ordinary
  captured-Delete confirmation and progress (`CapturedPathsDelete` in
  `midnight-core`). A result inside another marked folder is covered by that folder.
  The mutation service pins each target on a backend worker and refuses one that is
  no longer the kind (or, for a file, the size) the search found. When the delete
  completes without failures, its rows leave the list.
- Copy and Cut to the clipboard and Properties use the marked results' paths.

Not available in the list, and explained with a notice instead of acting on the
folder hidden underneath: create folder or file, rename, paste, sorting (sort in the
Find window before feeding), attributes, folder sizes, checksums, Make file list,
Compare files, Compare directories, Select and Deselect by mask, Show hidden files,
the panel filter, custom commands (from the toolbar, its popup and the F9 menu) and
Ctrl+Enter (the focused name into the command line); Ctrl+Shift+P inserts the focused
result's full path. The toolbar and menus refuse these the same way the keys do.
Keypad * (and its F10 Selection entry) inverts as in a panel: the files only, like
Ctrl+I, or the folders too when Select by mask last included folders
(`Presenter::mask_invert_operation`). The hidden rows start
no drag, and nothing is dropped onto a panel that shows results. After Move
the list still shows the old locations (their actions report that the file changed
or is gone); feed the search again to refresh it. At most 4096 items are deleted at
once.

The list lives in `presenter/find_feed.rs` (state and actions) and `find_feed.rs`
with `ui/find-feed.slint` (models, routing and the overlay). The panel's ordinary key
routing (`file_key_bridge::dispatch`) hands a shown list its actions first, so the
function bar and remapped shortcuts reach the results as well.
