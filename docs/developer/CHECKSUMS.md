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
An empty panel opens the Verify file page; a focused checksum sidecar pre-fills
that page. Neither action automatically verifies a manifest or follows its entries.
Folders can be selected and recursion explicitly enabled. Provider/archive content
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
retarget an existing session. Recalculate retains the captured roots and increments
the generation; it reads their current validated native content, not a snapshot
promised by the original directory listing.

The default is SHA-256. SHA-512, CRC32, MD5 and SHA-1 are optional; CRC32/MD5/SHA-1 are labelled
legacy compatibility algorithms. A single ordered read stream updates all chosen
algorithms. Failed, changed or cancelled rows contain no final digests. Copy's
mandatory independent SHA-256 verification is unaffected by these choices.

`ChecksumEvent` carries Queued, Started, Verifying, bounded Batch, Progress or Finished.
Rows are immutable shared objects with stable IDs; Finished retains the complete
bounded `ChecksumRun`. The UI keeps streamed row references until Finished, then
uses the run's shared rows directly and retires the temporary vector on the backend.
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
UI initially permits four visible result windows. Generation tags reject late
events. Close submits `ReleaseChecksums`, which cancels active calculation and
associated manifest saves and releases the actor's retained result. A full command
queue keeps the window and ownership visible with an error. Stop sends a separate
cancellation and retains completed rows. Backend tasks keep native resources until
their actual calls return; UI cancellation never releases an I/O permit early.

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

The window shows included/known-excluded counts and whether discovery was incomplete.
Saving partial results requires explicit acknowledgement of that run. Recalculation
clears the acknowledgement. A running save retains its generation; recalculation
cannot replace it until completion. The backend independently derives and checks
the counts from its own retained rows.

Exports are SFV (`.sfv`, filename then CRC32) or algorithm-specific GNU-compatible
UTF-8 manifests (`.md5`, `.sha1`, `.sha256`, `.sha512`, digest then filename). Relative native
names must round-trip exactly and remain under the output's directory on the same
device; invalid native text and aliases of selected sources are refused. The
unchanged suggested output retains its raw PathBuf even when display is lossy.
The existing staging, sync, independent SHA-256 readback and no-replace publication
contract protects output. Existing files are not silently overwritten. Overwrite
remains unavailable; verification is described below. The native Save picker offers
the chosen format's filter/default extension. Picking a name does not publish it.

Successful export receipts report actual path, included/excluded counts and verified
bytes. Matching visible native panels refresh without retargeting a later location.
An error or cancellation never invents a successful export. A post-publication
durability failure may leave a published file and is reported by the backend.

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

Input is bounded to 8 MiB, with aggregate decoded text/entry accounting under the
existing retained-byte limit and at most 16,384 rows by default. These resource
budgets do not truncate filenames or impose MAX_PATH. Native Open/Save pickers run
on their existing dedicated backend worker. Their replies are checked against the
session/generation; closure cancels the picker, and a late reply cannot start work
or overwrite another draft. Browse only fills a field; Verify/Save is explicit.

## Presentation, settings and compatibility

`files.checksums` defaults to Ctrl+F8; Ctrl+F7 stays case conversion. Saved explicit
overlapping bindings take precedence over the new default. The Ctrl footer and
User menu dispatch the same captured-selection action. Results, Options, Save
manifest, Details and Verify file are views of one movable native tool window, with Escape routed through
the shared tool-window focus scope. Inputs, result list and details scroll at the
320x200 minimum. Native editing/IME is not replaced by a global key interceptor.

Results receive most of the initial window. The selected output algorithm controls
both the displayed hash column and Copy; the chooser contains only algorithms in
the captured run. File/bytes/status stay before the hash. The complete digest uses
measured monospace width with horizontal scrolling when needed. Other algorithms
and full-row copy remain in the row context menu. Ctrl+C copies the chosen digest;
Ctrl+Shift+C copies the full row in this window, while that chord continues to
compare files in a main panel. Options holds algorithm and recursion controls and
locks them while calculation/export is active. Calculate requires a nonempty set.

Save manifest opens the destination form; Save writes only after a destination and
any partial-run acknowledgement are present. Drafts survive page changes; a new
generation clears consent. Backend validation remains authoritative. The footer
contains only actions relevant to the page and keeps Close reachable independently
of overflow. Error notices can be dismissed without losing Details. Large-font and
small-window pages scroll instead of enlarging the single-line destination input.
When height is tight, results use single-line rows and hide the header if it would
consume the last complete row. Both output selectors explicitly synchronize their
indices after native selection/model replacement; displayed and submitted algorithms
cannot diverge after switching pages.

Preferences schema 16 stores a nonempty algorithm set as stable names `sha256`,
`sha512`, `md5`, `sha1`, `crc32`. Older settings default to SHA-256. Invalid, repeated,
empty and unexpected values are rejected; default-only preferences retain the
older compatible encoding. Later schema 17 includes this field when storing
Custom Commands. Local algorithm edits made before initial settings load are
merged using the existing edited-field mechanism. Recursion is off for every new
window. Additional window geometry/column persistence remains a follow-up; the
current result layout is responsive and does not expose adjustable columns.

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
