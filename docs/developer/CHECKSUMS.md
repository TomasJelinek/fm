# Checksum calculation, file verification and export

Backend and engine qualification is recorded in
[checksum backend verification](../verification/2026-11-07-checksum-backend.md).
The modeless presentation is being qualified separately; source implementation
does not by itself establish release/native-desktop availability.

`midnight-core::checksum` is an internal Rust API. It is not part of the viewer
or filesystem plugin ABI. Independent plugins do not depend on host internals.

## Calculation contract

`StartSelectedChecksums` receives a captured `SelectedFiles` view, selection and
fallback, plus job/generation, algorithms, recursion and limits. The UI's existing
selection-action broker waits for the specific selection revision before opening
the window. A nonempty selection is authoritative; no parent (`..`) row becomes
a file target. With no selection, calculation uses a focused regular native file.
Opening the window calculates nothing; Calculate starts a run. With nothing selected,
the target is the focused file or folder; on the parent row or in an empty folder it
is the shown folder itself, so Calculate covers its files (and its subfolders with
Include subfolders). Ctrl+F8 therefore always opens on Calculate with a target. A
checksum file the user points at opens the Verify page instead, with that file
chosen: the focused file when it is a checksum file and is selected (or nothing is),
else, in a flat panel, the first selected checksum file in display order. The UI
finds it from the selection bitset and the displayed rows, looking at no more
selected rows than one calculation could hold (16,384), never by scanning the
folder's index. Neither action automatically verifies a manifest or follows its
entries.
A selected folder contributes its own files; Include subfolders adds the levels
below it, whose folders are otherwise left out without rows. Links and special
entries are skipped. Verification names files only, so a folder in a manifest
entry's place stays a skipped entry. Provider/archive content
is not silently interpreted as a native pathname.

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

`ChecksumEvent` carries Queued, Started, Verifying, Planned, bounded Batch, Progress or
Finished. A calculation first lists the files it will read (a walk that opens entries
and lists folders, within the same depth, volume, link and size limits, but reads no
contents) and sends them in Planned batches, then reads them in the same order.
Unreadable entries are reported only by the reading walk. Progress carries the listed
count and total length, whether listing is still going on, and the file being read
with its bytes read so far. A finished file's row is sent at once unless many finish
quickly, and then within 100 ms; progress also goes out every 100 ms. Each directory's
storage class is queried once per run. Verification lists nothing: its manifest
names its files. Rows are immutable shared objects with stable IDs; Finished retains
the complete bounded `ChecksumRun`. While a run goes on, the window shows the rows so
far followed by the listed files still to come: the one being read with its share
read, then those waiting. A determinate bar shows bytes read of the listed total (an
indeterminate one while listing), and the status line shows "N of M files · X of Y B
read". The UI keeps streamed row references until Finished, then uses the run's
shared rows directly and retires the temporary vector and the listed files on the
backend.
Only visible rows are formatted. Display text is never reconstructed into file
authority. Row callbacks contain generation plus ID, validated before copying.

The result table follows Salamander's checksum workflow: filename/folder, bytes
read, status and the chosen algorithm's hash. Other calculated algorithms are
available in the selector and row context menu. The header follows horizontal
scrolling; rows remain virtualized. Right-click or Shift+F10 offers each calculated
hash and the full row. Ctrl+C uses the output algorithm (or the full-row option).
Context menus capture the row token, close on generation changes, and cannot
redirect an action to a recycled row. Accessible rows expose full identifying
text, selection state and a default select action without starting file operations.

## Ownership and cancellation

The backend retains at most 32 admitted/result jobs under a process quota; the
UI initially permits four visible result windows. Admission reserves worst-case
memory from a shared 64 MiB budget: expanded targets plus rows (16 MiB for a
selection) or, for verification, the request plus three row budgets (at least
24 MiB). A finished run returns everything beyond its actual bounded rows, captured
roots and diagnostics, so four retained result windows still leave room for a
recalculation or a new window. Generation tags reject late
events. Close submits `ReleaseChecksums`, which cancels active calculation and
associated manifest saves and releases the actor's retained result. A full command
queue keeps the window and ownership visible with an error. Stop sends a separate
cancellation and retains completed rows. Backend tasks keep native resources until
their actual calls return; UI cancellation never releases an I/O permit early.

Each native checksum or verification call (open, enumeration, read chunk,
revalidation) takes the process-wide disk gate for that call only, so other disk
users interleave between a run's chunks. Admission follows the
[storage policy](DISK_OPERATIONS.md#meaning-and-storage-policy): a source volume
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
[disk operations](DISK_OPERATIONS.md#meaning-and-storage-policy) for the steps that
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
`.midnight-recovery-…` folder beside it. The native Save picker offers the chosen
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
job/generation and resource limits. It uses the same serial bounded checksum
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

Encoded input is bounded to 8 MiB and reserved separately. Decoded UTF-16 text
and every entry share the 8 MiB retained-byte limit; an entry is charged like a
calculated row, four bytes per absolute-path byte plus 1 KiB (about 5,900 entries
of 100-byte paths), with at most 16,384 rows and 64 path components below the
manifest's directory by default. A calculation also keeps the verifier's per-line
headroom, so every completed or acknowledged partial run fits these limits when
saved; a manifest saved far above deeply nested sources can still exceed the
component limit and is refused before publication with that budget named. These
resource budgets do not truncate filenames or impose MAX_PATH. Native Open/Save pickers run
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
the row's column (or ticks Include subfolders), and Escape or Tab closes it. The
shown algorithm controls the hash column, Copy and the format Save manifest… offers.
File/bytes/status stay before the hash. The complete digest uses measured monospace
width with horizontal scrolling when needed. The footer's Copy button copies the
calculated checksums of the shown algorithm as a checksum file: exactly the lines Save
manifest… writes to the suggested folder (`midnight_core::checksum_file_line`, shared
with the manifest writer), named relative to that folder. A file it cannot name there
is left out and the attention line says how many were. Other algorithms and full-row
copy remain in the row context menu. Ctrl+C copies the chosen digest of the selected
row; Ctrl+Shift+C copies the full row in this window, while that chord continues to
compare files in a main panel.

An issue line above the results names the window's error, else the first backend
diagnostic of the finished run (for example a refused manifest: absolute path,
parent traversal, malformed or duplicate line, a limit), else the resource limits
the run reached. Its Details button, and the one on the save line, show every
retained diagnostic, the limits reached in words, the counts, the checksum file
being verified and the last save's full paths; Copy details copies that text with
exact paths. Both lines can be dismissed; a new run closes Details. The footer
contains only actions relevant to the page and keeps Close reachable independently
of overflow. When height is tight, results use single-line rows and hide the header
if it would consume the last complete row.

Preferences schema 16 stores a nonempty algorithm set as stable names `sha256`,
`sha512`, `md5`, `sha1`, `crc32`. Older settings default to SHA-256. Invalid, repeated,
empty and unexpected values are rejected; default-only preferences retain the
older compatible encoding. Later schema 17 includes this field when storing
Custom Commands. Every new window starts with SHA-256 alone, whatever that field
holds: the algorithm set is chosen per window and a run no longer writes it back.
The field stays readable for compatibility and travels unchanged through settings
export and import. Recursion is off for every new window. The result columns (File / folder, Bytes read, Status
and the shown checksum) resize by dragging the separator after each header, as in
Find; the window keeps the widths for its session, and showing another algorithm
fits its checksum column again. Persisting window geometry and column widths
remains a follow-up.

Reference behavior was checked against Salamander's
`src/plugins/checksum/dialogs.cpp` (`InitList`, `SaveSizes`, `GetSaveFileName`,
`SaveHashes`, `OnContextMenu`, `CCalculateDialog::DialogProc`) and
`help/hh/checksum/using_calcchecksum.htm`. The implementation uses independent
Rust/Slint code and asynchronous cancellation; it does not copy reference code,
assets, synchronous waits or ANSI filename conversions.

The current native boundary is Windows/Linux. Different-device descendants are
excluded; same-filesystem Linux bind mounts require a future mount-ID capability.
Read guards and hashes describe the observed streams, not an immutable snapshot or
file authentication. Exact budgets, dependency licenses and native test evidence
remain in the backend verification document.
