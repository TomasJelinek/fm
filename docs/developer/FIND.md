# Recursive Find service and UI

The source implementation exposes Masks, Regex and typed Expression matching,
reusing the panel compilers, metadata semantics and budgets. Results use one
compact aligned row per entry. This is name/metadata matching only; the separate
content query remains literal. See the current
[panel/viewer follow-up requirements](../superpowers/specs/2026-11-07-panel-viewer-followups.md).

`midnight-core::FindService` is an internal Rust backend capability, not a stable
external plugin ABI. Windows and Linux implementations live in separate native
modules. The engine accepts `StartFind`, `SortFind`, `CancelFind`, `CloseFind`,
`ValidateFindResult`, `CaptureFindFile` and explicit `ExecuteFindFile`; the UI only
captures input and displays shared results.

Each of at most four modeless windows owns a monotonic session ID. Every accepted
search increments its generation. Up to two workers stream ordinary batches of at
most 64 stable-ID results through bounded channels. Duplicate groups publish as
one complete batch, bounded by the 10,000-result limit, so Stop cannot leave half
a group. Presenter and controller both
reject late generations. Stop retains results; Close releases the session, and
host shutdown closes the windows. Queue rejection never pretends cancellation
or close succeeded. Large result allocations are reclaimed on backend workers.
Replaced row-model order owners enter a small interior-mutable retirement queue:
refresh already holds a shared presenter borrow, so it must not request a mutable
presenter borrow while notifying Slint. The normal `take_retired` handoff drains
that queue for worker disposal, including streamed reorder and search restart.

Roots retain raw native `PathBuf`s independently of their display text. Add/remove
edits a list of at most 16 roots; unchanged paths never round-trip through a lossy
text editor. Masks and Regex use their existing filename compiler; Expression
uses captured native name/kind/size/time/attribute facts and preserves unknown
metadata semantics. A nonmatching directory is still traversed. Patterns are
bounded non-executable input. Content is literal UTF-8 (optional BOM) or UTF-16
little/big endian with a BOM, streamed through a 64 KiB read buffer with strict
incremental decoding and KMP. The compiled query and read/decoder workspace are
reused across files. Optional whole-word matching checks both surrounding word
boundaries, including Unicode combining marks/connectors and case-fold expansions;
plain substring matching remains the default. A found match never skips trailing
encoding validation or native revision checks.
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
capacity is omitted as a whole with an explicit limit. Group size describes the
eligible identities discovered by this search, whose incomplete status remains
visible when traversal or resource limits omit part of the requested scope.
Duplicate results use the same explicit navigation, View, Edit and Open actions as
ordinary results, with no original/redundant-file or reclaimable-space designation
and no automatic deletion.

`files.find` defaults to Alt+F7 and appears in the configurable command catalogue.
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
16 MiB aggregate and 4 MiB per key. The 10,000-result authority remains in discovery
order for revalidation. A separate immutable ID permutation and inverse index
lets the UI preserve the selected stable ID in constant time during reorder,
including newly streamed tails. Stale generation/revision/count orders are
discarded, and a failed sort retains the previous visible order with an error.
Extension and secondary filename keys share the aggregate key budget; sorting
and native locale calls remain entirely on the backend worker.
Narrow windows horizontally scroll the aligned columns; Close stays separate
from the scrolling actions. Stop retains results for subsequent read-only use.

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
16-request validation queue and single worker admit capture requests too. Stop
retains file action availability; accepted restart, Close and shutdown invalidate
obsolete generations. UI generation/token checks and cancellation prevent a late
capture from opening a different search result. Queue rejection leaves a visible
retryable error. See the [action verification](../verification/2026-11-12-find-actions.md).

Current scope is native Windows/Linux. Archives and other providers, legacy
content encodings, BOM-less UTF-16, hex/regex content queries and automatic file
execution are not enabled. System file activation requires the explicit Open action. Unsupported content, access errors and reached budgets produce
an explicit incomplete result, counts and Details; no silent complete claim.

The refactor has causal blocked-I/O, sorting, Stop-to-Sort and header-geometry
RED/GREEN coverage. Windows and Linux software-window pointer/key/geometry checks
pass, including the actual mode chooser, populated rows, all five headers and
minimum-size views. See the [verification record](../verification/2026-11-08-find-refactor.md).
Final application deployment/native OS-desktop qualification remains separate;
native desktop/GPU validation must not be inferred from the software tests.
The subsequent wildcard crash and extension parity follow-up is recorded in
[Find crash and extension verification](../verification/2026-11-08-find-crash-extension.md).

The 10 November backend follow-up has causal RED/GREEN coverage for relative
minute/hour intervals, future/unknown times, same-size unequal content, binary and
empty duplicates, hard links/overlapping roots, group-preserving sort, 70-member
atomic publication, Stop/publication boundaries and explicit budgets. Native
Windows fixtures provide the file contents, hard links and modification times;
read denial, short reads, substituted captures, equal-hash/unequal comparison
streams and late representative changes also have explicitly simulated coverage.
Logs and performance fixtures are under `_build/qa/operations-find/`. These tests
do not establish Linux runtime or installed desktop qualification for the new
controls; the current task's separate qualification record governs those claims.

The owned native duplicate performance fixture scanned 2,004 entries in 147 ms
and read 40,960 bytes: 2,000 singleton sizes required no content I/O, while four
identical 4 KiB files required hashing and exact comparison. This is one local
debug-build observation, not a hardware-independent latency guarantee; see
`_build/qa/operations-find/duplicates-performance.log`.
