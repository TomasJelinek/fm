# Checksum calculation, file verification and export

Backend and engine test evidence is recorded in
[checksum backend verification](../verification/2026-11-07-checksum-backend.md).

`midnight-core::checksum` is an internal Rust API. It is not part of the viewer
or filesystem plugin ABI. Independent plugins do not depend on host internals.

## Calculation contract

`StartSelectedChecksums` receives a captured `SelectedFiles` view, selection and
fallback, plus job/generation, algorithms and recursion. A run has no size limit: any
number of selected entries, files and folder levels. The UI's existing
selection-action broker waits for the specific selection revision before opening
the window. A nonempty selection is authoritative; no parent (`..`) row becomes
a file target.
Opening the window calculates nothing; Calculate starts a run. With nothing selected,
the target is the focused file or folder; on the parent row or in an empty folder it
is the shown folder itself, so Calculate covers its files (and its subfolders with
Include subfolders). That holds also when Ctrl+F8 follows a deselect so closely that
the window opens only once the empty selection is published. Ctrl+F8 therefore always
opens on Calculate with a target. A checksum file the user points at opens the Verify
page instead, with that file chosen: the focused file when it is a checksum file and
is selected (or nothing is), else, in a flat panel, the first selected checksum file
in display order. A checksum file is a regular file named with a checksum format's
extension, by the panel row's kind: a folder named like one (`photos.sfv`), focused,
selected or shown on the parent row, is calculated, never verified. The UI finds it
from the selection bitset and the displayed rows, looking only at selected rows (the
bitset skips the others 64 at a time), never by scanning the folder's index.
Neither action automatically verifies a manifest or follows its entries.
A selected folder contributes its own files; Include subfolders adds the levels
below it, whose folders are otherwise left out without rows and without being opened
(they are known by their listed kind), so a subfolder that is a mount point of another
volume (`~/nas`, `/media/<user>/stick`) or that cannot be read is left out too. With
Include subfolders a walk does not cross into another volume: a folder on another
device is never entered, it is a Skipped row and the run is Incomplete. Links and
special entries are skipped; a Windows volume mount point is a directory link and is
skipped as links are. Include subfolders walks every level, however deep, without
recursion: the walk keeps a stack of the open folders on its way down. A folder reached
again (another target, or a link that is followed in a projected tree) is recognized by
its native identity and walked once, so a walk always ends. A subfolder that cannot be
read is its own error row and the run goes on (it ends Incomplete). Rows keep the
walk's order: each folder's entries as the filesystem lists them (NTFS by name; ext4
in its hash order), subfolders where they occur; they are not sorted again.
Verification names files only, so a folder in a manifest entry's place stays a
skipped entry. Provider/archive content is not silently interpreted as a native
pathname.

In an adopted inline tree, Ctrl+F8 instead submits `TreeChecksumRequest` with the
exact acknowledged projection, selection allocation/revision and focused fallback.
The model worker resolves all occurrences together and requires native ordinary
files/directories; an unselected fallback must be a regular file. Selected expanded
directories keep their own provider paths. Parent rows, filter-only context, links,
archive members and unsupported rows are refused. A changed projection or source
navigation cancels pending capture; switching the active pane cannot retarget it.

`StartCapturedChecksums` feeds the same serial actor/controller. The window retains
opaque provider ownership for recalculation, using a shared four-slot tree action
permit. An opened window survives panel navigation. The actor resolves captured
targets on a worker, and the serial checksum worker checks their provider contexts
again before native reads. It retains those providers until actual I/O returns,
including cancellation and shutdown. Rejected/queued owners are also retired on
workers. The backend supplies suggested directory/sidecar paths; UI row captions
and displayed paths never authorize checksum reads. Formats, explicit recursion,
file verification, export and the ordinary checksum job/result budgets are shared.

The actor expands the captured selection on backend workers. `StartChecksums`
also accepts explicit captured targets for internal callers. Both feed the same
controller and budgets. Later panel navigation, sorting or selection cannot
retarget an existing session. Each Calculate retains the captured roots and
increments the generation; it reads their current validated native content, not a
snapshot promised by the original directory listing.

The default is SHA-256. SHA-512, CRC32, MD5 and SHA-1 are optional; CRC32/MD5/SHA-1 are labelled
legacy compatibility algorithms. A single ordered read stream updates all chosen
algorithms. Failed, changed or cancelled rows contain no final digests. Copy's
mandatory independent SHA-256 verification is unaffected by these choices.

Bytes are read through the pinned native handle. The source's namespace binding,
identity and revision are revalidated before the first and after the last read;
in between at most every 250 ms, because each check reopens the parent and name
(and every manifest ancestor). The intermediate checks only shorten how long a
replaced or rewritten source keeps streaming; the final check decides the row.

`ChecksumEvent` carries a `ChecksumUpdate`: Queued, Started, Verifying, Planned,
Batch, Progress or Finished. A calculation first lists the files it will read and sends them in Planned
batches, then reads them in the same order. The listing walks the same folders with the
same volume and link rules, but opens no file and reads no contents:
each folder's `TransferSource::next_listed_child` describes the children the walk does
not open, from the directory records on Windows (attributes, reparse tag, length) and
from a no-follow `statx` relative to the held folder on Linux, and opens only the
folders it enters, so a share or a disk pays no guarded open per file before the first
digest. The reading walk opens each file it reads, and each link or special entry to
report its refusal. Unreadable entries are reported only by
the reading walk: a listed file that cannot be opened gets its error row in its planned
place. Sources that cannot describe a child without opening it (archives, wrappers)
keep the default, which opens every child as `next_child` does. Progress carries the listed
count and total length, whether listing is still going on, and the file being read
with its bytes read so far. A finished file's row is sent at once unless many finish
quickly, and then within 100 ms; progress also goes out every 100 ms. Each directory's
storage class is queried once per run. Verification lists nothing: its manifest
names its files. Rows are immutable and numbered from one in the order found. They are
shared, never copied, in chunks (`ChecksumChunk`: the rows of one batch, the base folder
their names are below and the job's ownership) between the engine's retained run and
the window; Finished retains the complete `ChecksumRun` of the same chunks. While a run
goes on, the window shows the rows so
far followed by the listed files still to come: the one being read with its share
read, then those waiting. A determinate bar shows bytes read of the listed total (an
indeterminate one while listing), and the status line shows "N of M files · X of Y B
read", where a file that could not be read counts as done, as for the bar (by files
when every listed file is empty), and one file is "1 file". Details counts "1 error"
the same way. The UI keeps the streamed chunks until Finished, then uses the run's
chunks (the same ones) and retires its own list and the listed files on the backend.
A listed file's row is matched to it by its name below the run's base folder, among the
next 64 listed files that still wait: rows come in listing order except rows of entries
the listing leaves out (an unreadable folder, a link), which match none.
Only visible rows are formatted; rows keep no display text. Display text is never
reconstructed into file authority. Row callbacks contain generation plus ID (a listed
file's place in the listing while it waits), validated before copying.

The result table follows Salamander's checksum workflow: filename/folder, bytes
read, status and the chosen algorithm's hash. Other calculated algorithms are
available in the selector and row context menu. The header follows horizontal
scrolling; rows remain virtualized.

Rows are selected as in other lists (`crates/midnight-ui/src/list_selection.rs`, shared
with Find): a click selects a row alone and makes it the cursor and the anchor,
Ctrl+click toggles it, Shift+click marks the range from the anchor (Ctrl+Shift+click
adds it). Up, Down, Page Up, Page Down, Home and End select the row they reach; with
Shift they mark the range from the anchor, with Ctrl they move the cursor alone. Space
toggles the cursor's row and Insert toggles it and moves down; Ctrl+A selects every row
and Ctrl+D none. Ctrl+Space is left to the input method. The presenter keeps the
selection by row identity in a bitset (a finished row by its ID, a listed file by its
place in the listing, its mark following it to its row), so a lookup is O(1), Select
all is one pass and the marks survive rows that arrive or move. A selection change
that marks or unmarks at most 256 rows notifies those rows; a larger one (Select all,
Deselect all, a click that drops a large selection, a long range) resets the list once
instead (`MARKS_RESET` in `checksum_windows.rs`), and the list then reads again only the
rows it shows and keeps its scroll position. Before 24 November Select all notified
every row, 100,000 notifications at 100,000 rows
([record](../verification/2026-11-24-ui-perf-fixes.md#checksums-select-all)). What Copy can take is counted the same way: each row is marked once, when it
arrives, as having a checksum-file line (calculated) and a checksum (calculated or
verified), so the counts behind Copy's caption and enabled state are word operations
over the selection's bits (`ListSelection::count_in`), however many rows are selected.
Measured in a release build at 1,000,000 rows (`checksum_selection_benchmark`): Ctrl+A
takes 11 ms of the UI thread, a click or Ctrl+D after it 6 to 9 ms, a refresh during a run
0.14 ms and asking for a copy of every row 0.4 ms; the clipboard worker then builds the
1,000,000 checksum-file lines in about 0.9 s (100,000 rows: 87 ms). A new calculation of the same files (the same captured files with the same
Include subfolders) selects the same files again, by name, as it lists and reads them;
another run (Include subfolders changed, a verification, another checksum file) starts
with none. Selected rows use the selection fill and text colour with a square mark;
the cursor's row has a full-height stroke and an outline (and the cursor fill when not
selected), so the two differ without colour in every theme. Accessible rows expose full
identifying text, their selected state and a default action that selects the row
alone, without starting file operations.

Copy, Ctrl+C, Ctrl+Shift+C and the context menu act on the selection, in the order
shown. Ctrl+C copies the selected rows' checksums of the shown algorithm, one per line;
Ctrl+Shift+C their full rows (path, then every checksum), a blank line between rows.
Right-click or Shift+F10 on a selected row offers each calculated algorithm and the
full rows of the whole selection ("Copy 3 SHA-256"); on another row, that row alone,
which becomes the cursor without changing the selection. Context menus capture the row
token, close on generation changes, and cannot redirect an action to a recycled row.
A selected row that adds nothing (still waiting, failed, skipped, cancelled, without
the chosen checksum) is counted in the copy's note on the save line, by reason.

While a calculation runs and nothing is selected, the list follows the row being
calculated (the first listed file without its row): when it would leave the view, the
list scrolls to show it as the last whole row at the bottom. Scrolling stops at a row's
top, as the cursor's does: when the list jumps more than a screen and a half, Slint's
ListView puts its first shown row at the top, which would cut a row placed exactly at
the bottom edge. This runs only when that row
changes (or the selection empties), in a Slint change handler, which runs once per
frame with the latest value, so many files finishing between two frames scroll the list
once. A selection stops following. A scroll by the user pauses it until the row being
calculated is in view again with nothing selected, or until the next run.

## Ownership and cancellation

The backend retains at most 32 admitted/result jobs (job slots, a count, not bytes);
the UI permits four result windows. A job's slot is held by the run and by every chunk
of its rows, so a window that keeps results keeps its slot. Nothing limits a run's size:
its memory follows its rows. A row keeps its name below the run's base folder, its
digests at their fixed lengths in one allocation and an error only when it has one, in
chunks of up to 64 rows (`CHECKSUM_BATCH_SIZE`) with no per-row `Arc`: `ChecksumRow`
is at most 104 bytes (a test asserts it), and a SHA-256 row with a typical 25-byte name
measured 198 bytes of process memory at 100,000 rows (19.9 MB) and 196 bytes at
1,000,000 (196 MB) in a Windows release build (the ignored `row_memory_compact`
measurement in `midnight-core/tests/checksum_verification.rs`, whose row count
`MIDNIGHT_CHECKSUM_ROWS` sets; 100,000 by default). During a
calculation the window also keeps the listed files (a name and a length each) until
Finished. Generation tags reject late
events. Close submits `ReleaseChecksums`, which cancels active calculation and
associated manifest saves and releases the actor's retained result. A full command
queue keeps the window and ownership visible with an error. Stop sends a separate
cancellation and retains completed rows. Backend tasks keep native resources until
their actual calls return; UI cancellation never releases an I/O permit early.

Each native checksum or verification call (open, enumeration, read chunk,
revalidation) takes the process-wide disk gate for that call only, so other disk
users interleave between a run's chunks. Admission follows the
[storage policy](disk-operations.md#meaning-and-storage-policy): a source volume
positively identified as a local SSD is read with Shared admission beside other
disk I/O; unknown, rotational and network volumes (and every read when no storage
classifier is installed) use Exclusive admission. Calculation classifies the
directory of each selected entry once (consecutive entries of one directory share
the query) and a recursively selected directory by its own path: a walk never
leaves the root's volume, but the root itself can be a mount point. Verification
classifies the manifest's directory once, because every entry is confined to its
volume. An Exclusive call waits until no Shared holder remains and, once queued, is
served before later gate requests. A running transfer holds its retained slot only
while it executes a native step and yields it at its next step boundary while a
call-scoped request such as a checksum call is queued, so a queued checksum call
waits for at most one step of each running transfer, not for whole transfers (see
[disk operations](disk-operations.md#meaning-and-storage-policy) for the steps that
can take longer). A manifest save keeps
Exclusive admission for its few namespace calls and waits for the mutation gate
with one queued acquisition, so later Delete/Rename requests cannot overtake it.
Idle checksum actors do not poll; shutdown wakes them.

Main-window shutdown hides owned tool windows without borrowing Presenter through
an already borrowed shutdown callback. A closing flag prevents late refresh from
recreating them. The engine's shutdown barrier cancels and awaits checksum work,
including releases that could not enter a full queue. Large rows/runs/views are
sent through the existing deferred reclamation mechanism.

A native tool-window creation failure is reported once. The shared failure helper
suppresses recreation and retries a refused release after 250 ms, with weak host
ownership. Main-window shutdown ends these retries; the engine barrier remains
the final owner of pending cleanup. It never enters a zero-delay recreation loop.

## Manifest publication

`SaveChecksumManifest` references a retained job/generation. Its request contains
an algorithm, native absolute destination and optional exact partial acknowledgement.
It contains no UI-provided hashes. `ChecksumManifestId` is increasing and never
reused, including rejected saves. Results distinguish admission Rejected from
Queued/Saving/Finished; duplicate rejection is not completion of an accepted save.
`CancelChecksumManifest` cancels one export by its identity.

Saving partial results first asks in the window, naming the included and known
excluded counts and whether discovery was incomplete. The question belongs to its
run: answering it sends exactly the counts it showed, and a new calculation or
verification discards it together with a destination chosen for the old run. An
answer the window cannot act on (other checksum work still active) is reported and
the question kept. A running save retains its generation; recalculation cannot
replace it until completion. The backend independently derives and checks the
counts from its own retained rows and refuses a different acknowledgement.

Exports are SFV (`.sfv`, filename then CRC32) or algorithm-specific GNU-compatible
UTF-8 manifests (`.md5`, `.sha1`, `.sha256`, `.sha512`, digest then filename). Relative native
names must round-trip exactly and remain under the output's directory on the same
device; invalid native text and aliases of selected sources are refused. A chosen
destination keeps its raw PathBuf, also through the replace question, even when
display is lossy. Output uses exclusive staging, sync and independent SHA-256
readback before publication (shared with the Shift+F4 empty-file replacement).
An existing file is never silently overwritten: the backend refuses it with
`AlreadyExists`, the window asks, and only Replace sends an explicit `overwrite`.
A confirmed replacement pins the existing regular file and publishes through the
platform's recoverable replacement; the replaced file is kept in a private
`.midnight-recovery-…` folder beside it. The replacement keeps the replaced file's
metadata; on Windows a compressed, read-only or encrypted original is replaced too,
with the rules in [Make File List](make-file-list.md#output-lifetime-and-safety).
The native Save picker offers the chosen
format's filter and default extension; choosing a file there starts the save.

Successful export receipts report the actual path (an appended extension
included), included/excluded counts, verified bytes and where a replaced file is
kept. The window shows them in a save line under the results; Details shows the
full paths, including the recovery path to delete once the old file is no longer
needed. Visible flat native panels showing the output folder re-read it; adopted
native trees reconcile their loaded branches through tree maintenance instead.
Neither retargets a later location. An error or cancellation never invents a
successful export. A failure after publication (folder durability, the recovery
proof) is reported as an error that says the manifest was created or replaced and
names the recovery path; it never removes the published file or the kept one.

## Verification from a file

`VerifyChecksumManifest(ChecksumVerifyRequest)` captures a native manifest path,
job/generation. It uses the same serial checksum
controller as calculation, with `CancelChecksums`/`ReleaseChecksums`. Parsing and
all reads run on backend workers. `Verifying(algorithm)` reports content detection;
the extension disambiguates overlapping SFV/GNU syntax and supplies a lone hash's
sidecar filename. Records whose target remains ambiguous are rejected.

The parser accepts Salamander SFV, GNU digest-first text/binary markers, BSD tagged
`MD5/SHA1/SHA256/SHA512 (name) = digest`, `;`/`#` comments, uppercase hex, CRLF,
UTF-8 (optional BOM), and BOM-marked UTF-16. A single bare hash in `file.ext.sha256`
checks `file.ext`. GNU escaped names round-trip; malformed/unsupported encodings
are rejected without lossy decoding. Legacy ANSI files must first be converted to
UTF-8 with their correct original encoding.

All entries are parsed before file data is opened. Absolute/drive/UNC/device/ADS
paths, parent traversal, duplicate/ambiguous names, mixed algorithms and malformed
lines fail the manifest. Paths resolve below its directory; directory/file handles
pin native identities and validate each ancestor rather than following links or
crossing devices. Windows and Linux implement the read capabilities separately.
The manifest and data files are revision-checked before/after reads. These are
live-file checks, not a filesystem snapshot or a signature/authenticity guarantee.

Results distinguish Verified, Mismatch, Missing, Source changed, Read error,
Skipped and Cancelled. Mismatch may retain the actual computed digest for copying;
only calculation rows are eligible for manifest export. Counts of verified,
mismatched and missing files are shown independently; interrupted or partially
failed verification cannot be reported as complete success.

A manifest has no size, line-count, line-length or depth limit. It is read in chunks
and parsed line by line as it arrives (`ManifestParser`): only the line being read and
one read buffer are held besides the entries, each a name below the manifest's folder
and a fixed-length digest, so memory follows the entries, never the file's size (UTF-16
is decoded as it is read, a pair or surrogate split between chunks included). Case-only
duplicates are found by the hash of each name's case-folded text, settled against the
names already kept, without a second copy of them. Every entry is parsed before any
listed file is opened, and the listed length detects a manifest that changes while it is
read. The verifier then makes one read target at a time from the parsed names; each
entry gives exactly one row, so a row's ID names its entry and expected digest. A save
writes any number of lines in one pass and reads its own text back through the same
parser before publication. Native Open/Save pickers run
on their existing dedicated backend worker. Their replies are checked against the
session/generation; closure cancels the picker, and a late reply cannot start work
or overwrite another draft. On the Verify page, Browse only fills the field and Verify
is explicit; Save manifest… is itself the explicit request, so the file chosen
there is saved at once (partial results and an existing file are asked first).

## Presentation, settings and compatibility

`files.checksums` defaults to Ctrl+F8; Ctrl+F7 stays case conversion. Saved explicit
overlapping bindings take precedence over the new default. The Ctrl footer and
User menu dispatch the same captured-selection action. Calculate and Verify are
the two pages of one movable native tool window, with Escape routed through the
shared tool-window focus scope. The window opens with the keyboard focus on the
shown page: the result list, or the window itself on Verify. A control that
removes itself when used (an answer to the save question, Details, Back, a dismiss
button) first moves the focus back to the page, so Escape keeps closing the window.
Details replaces the page on request and Back returns to it. The result list, the Verify page and Details scroll at the
320x200 minimum. Native editing/IME is not replaced by a global key interceptor.

Results receive most of the initial window. The algorithm dropdown next to the page
tabs lists all five algorithms: each row's checkbox includes that algorithm in the
next calculation, and choosing a calculated one shows its column; algorithms the
current run did not calculate cannot be shown. Include subfolders is its last row.
The checkboxes lock while calculation, a save or a file chooser is active, and
Calculate requires a nonempty set. The dropdown works from the keyboard: Enter,
Space or an arrow opens it; Up/Down/Home/End move, Space ticks the row, Enter shows
the row's column (or ticks Include subfolders), and Escape or Tab closes it. Enter in
the results list (which shows a focus ring), or on the page where no field takes it,
runs the shown page's primary action under the same conditions as its footer button:
Calculate, or Verify on the Verify page. Enter never discards a verification's
results: while they are shown (after Verify the window shows its first page), Enter
in them verifies the same checksum file again, by its exact path whatever the Verify
field holds by then, and Calculate, which would replace them, is a button press only
and loses its default look. Never from Details, while a save question waits, while
work runs or on a held key. The
shown algorithm controls the hash column, Copy and the format Save manifest… offers.
File/bytes/status stay before the hash. The complete digest uses measured monospace
width with horizontal scrolling when needed. The footer's Copy button copies the
selected rows' checksums of the shown algorithm as a checksum file, in the order shown:
exactly the lines Save manifest… writes to the suggested folder
(`midnight_core::push_checksum_file_line`, shared with the manifest writer), named
relative to that folder. The window does not build the text: it snapshots the selection's
bits, shares the run's row chunks and sends a `ChecksumCopyText` to the clipboard worker
(`Command::CopyGeneratedText`), which builds the text there in one pass, into room it
reserves fallibly, and writes it; the result brings the note back. A newer copy of the
window supersedes a pending one and closing the window cancels it. A copy has no size
limit; one that does not fit in memory fails with "There is not enough memory to copy N
rows." ("1 row." for one) and the clipboard keeps what it held, as it does for a copy that
adds nothing. With several
calculated rows selected its caption counts them ("Copy 3 SHA-256"); below 460 px it is
"Copy". The footer fits its actions from their measured captions (`FooterActions` in
`footer-actions.slint`, used by `DialogFooter` in `dialog-footer.slint`): one that does not fit whole beside Close moves into More…,
Copy details and Save… first, then Copy; Back, Calculate or Stop and Verify stay
buttons (showing their glyphs alone, and last in More… too, only where even that does
not fit: see the [dialog ergonomics](dialog-ergonomics.md)). A counted caption therefore
moves Copy into More… rather than being cut. It is
enabled when a
selected row has a calculated checksum, also while a calculation runs; otherwise its
tooltip (and accessible description) says why and that Ctrl+A selects all. Save
manifest… still writes every calculated row of the run, whatever is selected (it is
the job's manifest); its tooltip says so. A row Copy cannot name there, or that has no
calculated checksum, is left out. A note on the save line, not an error, then says how
many lines were copied and how many rows were left out for each reason (not calculated
yet; failed; skipped; cancelled; without the chosen checksum; a verification's result;
not below the checksum file's folder; a name that is not valid Unicode; a name SFV
cannot hold unambiguously, such as one starting with `#`, `;` or a space; any other
reason in the encoder's own words), and Details repeats it. Ctrl+C, Ctrl+Shift+C and
the context menu report what they leave out the same way. The note neither sets nor
clears the window's error; the next copy, a save or a new run replaces it. Ctrl+Shift+C
copies full rows in this window, while that chord continues to compare files in a main
panel.

An issue line above the results names the window's error, else the first backend
diagnostic of the finished run (for example a refused manifest: absolute path,
parent traversal, malformed or duplicate line). Its Details button, and the one on the
save line, show every retained diagnostic (the first 32; each failed row keeps its own
error), the counts, the checksum file
being verified, the last copy's note and the last save's full paths; Copy details
copies that text with
exact paths. Both lines can be dismissed; a new run closes Details. The footer
contains only actions relevant to the page and keeps Close reachable independently
of overflow. When height is tight, results use single-line rows and hide the header
if it would consume the last complete row.

Preferences schema 16 stores a nonempty algorithm set as stable names `sha256`,
`sha512`, `md5`, `sha1`, `crc32`. Older settings default to SHA-256. Invalid, repeated,
empty and unexpected values are rejected; default-only preferences retain the
older compatible encoding. Later schema 17 includes this field when storing
Custom Commands. Every new window starts with SHA-256 alone, whatever that field
holds: the algorithm set is chosen per window and a run does not write it back.
The field stays readable for compatibility and travels unchanged through settings
export and import. Recursion is off for every new window. The result columns (File / folder, Bytes read, Status
and the shown checksum) resize by dragging the separator after each header, as in
Find; the window keeps the widths for its session, and showing another algorithm
fits its checksum column again. Window geometry and column widths are not saved.

The behaviour follows Salamander's checksum plugin (its list, saving, context menu and
calculation dialog). The implementation uses independent Rust/Slint code and
asynchronous cancellation; it does not copy reference code, assets, synchronous waits
or ANSI filename conversions.

The native boundary is Windows/Linux. Different-device descendants are excluded (left
out without Include subfolders, Skipped with it; see the calculation contract); a Linux
bind mount of the same filesystem is not recognized as another volume. Read guards and
hashes describe the observed streams, not an immutable snapshot or file authentication.
