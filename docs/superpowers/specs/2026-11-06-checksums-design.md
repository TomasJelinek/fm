# Checksums for selected files: Ctrl+F8

**8 November extension:** the current implementation adds SFV/CRC32 export and
verification from SFV/MD5/SHA-1/SHA-256/SHA-512 files, including GNU/BSD records
and hash-only sidecars. Native Open/Save dialogs and a Verify file page share the
existing modeless window. The older exclusions of CRC32/import below are historical
milestone scope; the [current API](../../developer/CHECKSUMS.md) supersedes them.
See the [implementation plan](../plans/2026-11-08-checksum-files.md).

Date: 6 November 2026. **Implementation update, 7 November:** the native backend
and actor are implemented and verified; modeless UI integration and qualification
are in progress. See [the current API contract](../../developer/CHECKSUMS.md)
and [backend evidence](../../verification/2026-11-07-checksum-backend.md).
The remaining sections preserve the design baseline and proposed follow-ups;
they are not a claim that every proposal is already released. The workflow follows
Salamander's Checksum plugin. The user's corrected shortcut is
**Ctrl+F8**. Ctrl+F7 remains filename case conversion; Ctrl+F2 remains the metadata
editor. Those existing operations are not reassigned.

This design follows [security requirements](../../../SECURITY.md),
[code style](../../../CODE_STYLE.md), the
[dependency policy](../../DEPENDENCY_POLICY.md), and the
[main architecture](2026-11-04-dual-pane-file-manager-design.md). Numerical bounds
and defaults below are design proposals to validate during implementation, not
claims of existing performance or interfaces.

## Intent and first milestone

Press Ctrl+F8 in a file panel to calculate checksums for **all selected regular
files**. With no selection, use the focused regular file. A modeless results
window lets the user keep browsing while work runs, inspect progress and errors,
copy individual digests or rows, and save an interoperable checksum manifest.
SHA-256 is the default; MD5, SHA-1 and SHA-512 are selectable. Selected algorithms
consume the same ordered read stream rather than rereading each file.

The first milestone includes local regular files, including native paths to
network shares, bounded streaming, cancellation, per-file results, copy and
explicit manifest save. Folder recursion is an explicit option, off by default.
Checksum-manifest verification is a follow-up using the same calculation service;
it is specified as a compatibility direction here, not included as a completed
or silently enabled feature. Archive members and future remote provider handles
wait for the corresponding content/provider contract.

This feature calculates a digest of bytes read. It does not authenticate a file,
prove an immutable snapshot, encrypt/sign data, or replace verified copying.
The implemented copy engine continues to require SHA-256 and a full independent
destination readback before publication. Choosing MD5 for a checksum report cannot
weaken that copy contract.

## Observed Salamander behavior

The reference was read from
`D:/experiments/salamander-main/src/plugins/checksum/`. These are observations of
that checkout, not a requirement to copy its source, UI assets or implementation.

| Reference source | Observed behavior | Decision for Midnight Explorer |
| --- | --- | --- |
| `checksum.cpp:34`, algorithm table at `:40` | Registers CRC32, MD5, SHA-1, SHA-256 and SHA-512; all calculation flags initially enabled; MD5 is the initial save format | Offer MD5/SHA-1/SHA-256/SHA-512; start with SHA-256 only; CRC32/SFV is outside the initial request |
| `dialogs.cpp:2075`, especially `:2099` | Captures selected entries, falling back to the focused entry when none are selected | Preserve selection-first and focus fallback, using native identities and accepted visible-panel state |
| `dialogs.cpp:475`, `:560`; `help/hh/checksum/using_calcchecksum.htm:22` | Selected directories expand into files; help describes recursive inclusion | Require explicit recursion; skipped directories remain visible in the summary |
| `dialogs.cpp:652`, `:738`, `:771`; buffer at `:15` | Initializes chosen hashers, reads 256 KiB chunks and feeds each chunk to every enabled algorithm | One ordered stream per file, reusable bounded buffer, all requested digests updated together |
| `dialogs.cpp:28`, `:305`, `:313` | Timer-driven progress with a 100 ms UI period | Coalesce checksum progress to at most four updates per second; terminal events are prompt |
| `dialogs.cpp:1017`; calculation help `:34` | Result columns and context actions copy an individual algorithm's hash | Keyboard and context-menu copy actions, plus copy rows with algorithm labels |
| `dialogs.cpp:957`, `:1001`; `wrappers.cpp:257` | Saves algorithm-specific text manifests; parser recognizes digest-first and tagged forms | Explicit algorithm-specific UTF-8 export, with specified escaping and safe future import |
| `help/hh/checksum/using_verify.htm:16` | Separate modeless verification workflow and statuses | Keep verification a distinct follow-up action and result state |
| `dialogs.cpp:412`, `:430` | Closing requests worker termination and destruction waits for the worker | Responsive cancellation/closing state; backend retains ownership until native calls return |

The reference also uses `MAX_PATH`/narrow buffers (`dialogs.cpp:694`, `:961`) and
follows some link targets during directory expansion (`:512`). Those implementation
choices are not carried over. Midnight Explorer preserves native path identities,
does not impose `MAX_PATH`, and skips links/special files in this milestone.
No Salamander code or assets are imported.

## Approaches and proposed choice

| Approach | Benefit | Cost / constraint |
| --- | --- | --- |
| Built-in backend checksum service with shared Slint presentation | Reuses native identity, cancellation, storage scheduling and existing SHA-256 dependency; works consistently on Windows/Linux | Adds an internal application service and results presenter |
| New general-purpose calculation plugin contract | Could permit independently shipped algorithms later | Needs a new batch-operation ABI, ownership/budget rules and conformance tests; today's viewer ABI does not expose these |
| External checksum commands | Can interoperate with tools already installed on some systems | Adds executable discovery, platform availability, subprocess limits and output/path parsing; makes one-pass multi-algorithm reads and shared storage accounting harder |

**Recommendation:** implement the first approach. This is an engineering proposal,
not a claim that the user approved a new plugin contract. Keep algorithm dispatch
small and internal. The existing viewer API returns Text/Binary/Document pages
for a viewer session; it must not be repurposed for checksum jobs or presented as
a public checksum SDK. A future calculation-plugin proposal would be a separate
versioned API design.

## Shortcut and settings behavior

Proposed semantic action: `files.checksums`, label **Calculate checksums**, default
**Ctrl+F8**, scoped to file-panel actions. This identifier is planned; it is not
currently exported or dispatched. The current shortcut catalog assigns Ctrl+F7
to `files.case`, Ctrl+F2 to `files.attributes`, and no default to Ctrl+F8.

Add the new action, menu item, preferences entry and key hint only with a working
implementation. Existing shortcuts continue to work unchanged in this design-only
change. No placeholder binding should consume Ctrl+F8 and then do nothing.

On the implementation's settings migration, preserve every explicit override,
including custom Ctrl+F8 bindings and disabled actions. Apply the new default only
where Ctrl+F8 is free in the overlapping file-panel scope. If another explicit
binding owns it, keep that binding and expose Calculate checksums through the
menu/preferences without a default key. Do not invalidate the entire settings
file, silently reset shortcuts, or steal the chord. An explicit later conflicting
assignment still uses normal conflict validation. Migration must be idempotent
and tested separately from new-install defaults.

Persist the selected algorithm set and window/column preferences using the
existing bounded versioned settings mechanism. Default to SHA-256 when no valid
saved choice exists; require at least one algorithm. Recursion is off for each new
invocation unless a later product decision explicitly changes that policy.

## Capturing targets

The invocation captures the active panel's accepted directory identity, visible
selection revision, selected native entry IDs/paths and algorithm set. It does not
read file contents or walk directories on the UI thread. Backend preparation
materializes and validates the immutable job input before queue admission. Later
navigation, sorting, refresh, selection changes or a second invocation cannot
retarget an existing job. A failed/stale preparation reports failure rather than
substituting whatever now occupies the row.

Selection rules:

- A nonempty selection is authoritative. Include every selected regular file;
  record other selected entry kinds as skipped with a reason. Do not add the
  focused file merely because selected items were folders or unsupported types.
- With an empty selection, use the focused regular file. A parent row, empty
  filtered view or unsupported focus produces a clear no-eligible-files state.
  Hidden entries never become operation targets.
- Symlinks, junctions, reparse files, devices, FIFOs and sockets are skipped.
  Do not follow a final link or probe a special file's contents to discover its
  type. An already navigated directory may be the panel location, but captured
  entries still undergo backend identity/open checks.
- Folder recursion is a visible **Include subfolders** option. Enabling it and
  recalculating creates a fresh run over the same captured root selection.
  Traverse ordinary directories without following links or crossing mounts.
  Track directory identities to avoid cycles; report access and resource failures.
  Depth 64 is an operation budget, not a path-length limit. A limit hit marks the
  run incomplete, never successfully truncated.
- Overlapping selected directory roots must not produce duplicate rows for the
  same captured root-relative path. Distinct hardlink names remain distinct rows;
  sharing an inode does not merge the user's named targets or justify reusing an
  old digest without revision validation.

Paths remain native `PathBuf`/OS representations through capture, opening, results
and export preparation. Display labels are separate, may be escaped for clarity,
and never reconstruct filesystem authority. Preserve Windows UTF-16, raw Linux
filename bytes, combining sequences and long paths; do not normalize names or
truncate them to fit display columns. File lengths, offsets, byte counters and
totals use checked `u64`; arithmetic failure is an error.

## Algorithms and dependencies

| Algorithm | Digest | Default | Intended use |
| --- | --- | --- | --- |
| SHA-256 | 32 bytes / 64 hex characters | On | General checksum calculation and interchange |
| SHA-512 | 64 bytes / 128 hex characters | Off | Compatibility with SHA-512 manifests and explicit requests |
| SHA-1 | 20 bytes / 40 hex characters | Off | Legacy interoperability only |
| MD5 | 16 bytes / 32 hex characters | Off | Legacy interoperability only |

The UI identifies MD5 and SHA-1 as legacy algorithms unsuitable for security
decisions. Their upstream implementations explicitly document that limitation.
Computing SHA-256 also does not establish who supplied a file or whether a supplied
manifest is trusted. See the primary [MD5 documentation](https://docs.rs/crate/md-5/0.10.6)
and [SHA-1 documentation](https://docs.rs/sha1/0.10.6/sha1/).

Reuse the already pinned `sha2 = 0.10.9` for SHA-256/SHA-512. The current
`midnight-platform-common` transfer implementation uses its incremental `Digest`
interface. Its pinned manifest declares `MIT OR Apache-2.0`; both licence texts
are present in the local Cargo source. The crate supplies incremental SHA-256
and SHA-512 through the same interface. See
[sha2 0.10.9 source manifest](https://docs.rs/crate/sha2/0.10.9/source/Cargo.toml)
and [incremental API](https://docs.rs/sha2/0.10.9/sha2/).

Proposed additional implementation candidates are RustCrypto **`md-5` 0.10.6**
(Rust module `md5`, not the unrelated crate named `md5`) and **`sha1` 0.10.6**.
These versions share the `digest` 0.10 family with the existing SHA-2 package.
They are compatibility candidates, not claims to be the newest releases and not
newly selected lockfile entries. Both declare `MIT OR Apache-2.0`; choose a free
compatible branch and retain notices. References:
[MD5 licence declaration](https://docs.rs/crate/md-5/0.10.6),
[MD5 feature set](https://docs.rs/crate/md-5/0.10.6/features),
[SHA-1 pinned source manifest](https://raw.githubusercontent.com/RustCrypto/hashes/sha1-v0.10.6/sha1/Cargo.toml),
and [SHA-1 MIT text](https://raw.githubusercontent.com/RustCrypto/hashes/sha1-v0.10.6/sha1/LICENSE-MIT).

At implementation time, recheck the precise releases, maintenance/advisory status,
resolved transitive graph, bundled source and enabled features. Prefer the small
portable `std` feature set; do not enable optional assembly crates, OID/compression
APIs, SIMD feature overrides or parallel hash pools without a demonstrated need.
Use supported runtime CPU detection rather than assuming every x64 CPU has the
same instructions. The public manifests identify optional assembly dependencies;
they are not required for the proposed feature set.

Every dependency must permit development, use and distribution without mandatory
fees/subscriptions/royalties, under [project policy](../../DEPENDENCY_POLICY.md).
Before adding either candidate, inspect its distributed licence texts, update
the [inventory](../../THIRD_PARTY.md) and all affected distribution notices, and
run the actual target builds/advisory checks. This document adds no dependency,
selects no paid licence and changes no project licence. Candidate versions may
change during that review; algorithm and file-format behavior must remain stable.

## Backend streaming and source validity

Add an internal backend contract for preparing captured targets and reading a
regular file through a native validated handle. Use separate Windows/Linux
implementations and platform-neutral digest/state orchestration. Share audited
identity/open primitives with other services when their guarantees match, without
coupling calculation to a viewer instance or destination-copy object. Any names
chosen for new Rust traits/types are internal implementation details, not new
public SDK exports.

For each file:

1. Resolve the captured target on a backend worker, refuse unexpected links or
   type changes, obtain native identity/length/change metadata and retain one
   read-only regular-file handle. Revalidate against the captured target before
   beginning. If preparation cannot establish the required identity, fail that
   item rather than opening a replacement by display name.
2. Initialize only the selected incremental hash states. Read sequentially into
   one reusable buffer of at most 256 KiB. Update every chosen algorithm with
   the same bytes in the same order, then advance the checked offset once.
   Never allocate according to whole-file size or launch one task per block.
3. Handle partial reads correctly: positive short reads continue from the actual
   byte count; interruptions follow the native retry contract; EOF and error are
   distinct. Empty files finalize the standard empty-stream digest.
4. Check cancellation between read/hash steps. Revalidate handle identity,
   length and available change/write/version metadata before reading and after
   each chunk, including after any scheduling pause. Also perform a final
   revalidation and byte-count check at EOF. Avoid publishing a digest while
   validation remains pending.
5. Only after successful EOF and final validation, publish a **Calculated** row
   with the chosen raw digests and observed byte count. Errors, detected changes
   or cancellation discard unfinished digest states; never expose their partial
   hashes as completed results.

The guarantee is **LiveValidated**, not an immutable filesystem snapshot. Native
identity prevents common pathname replacement mistakes, and version/metadata
checks detect supported changes, but timestamps and identity alone cannot prove
that no writer changed and restored data between checks. A result describes the
bytes read through that validated handle. Report a detected change as **Source
changed — recalculate**, with no usable calculated digest. Do not label calculation
as Verified. Even future comparison with a manifest needs a separate **Matches**
status and cannot turn a live read into a snapshot or authenticate its source.

A source renamed/replaced while pinned must be rechecked against the captured
target contract; it must not silently become the new object at that pathname.
The initial implementation should report the item as changed when it cannot
establish that the captured name/identity relationship still holds. Current
read-only operations do not promise a universal race-free namespace snapshot.

## Scheduling, queue and resource budgets

Filesystem work, digest updates, traversal, sorting, manifest encoding/writes,
clipboard preparation and large result cleanup all run on backend services.
UI callbacks submit small commands and present bounded result pages. Acquiring
CPU/I/O permits precedes dispatch to blocking workers; `spawn_blocking` alone is
not an application concurrency bound. There is no private thread pool per window,
algorithm, file or plugin.

Hashing has **only a source**. The
[future copy concurrency design](2026-11-04-copy-concurrency-design.md) requires
two positively classified SSD endpoints for parallel copies, but checksum jobs
have no destination and must not invent a destination-SSD requirement. Their
source reads still share the actual device/endpoint budgets with scans, copies
and readback; separate queues must not multiply storage capacity.

Proposed conservative limits:

| Resource | Initial design bound |
| --- | --- |
| Queued/preparing/running/retained checksum jobs | 32 process-wide; admission is nonblocking and reports a full queue |
| Preparation / recursive discovery | Two backend preparation tasks; at most 64 directory handles per depth-bounded traversal, 128 total |
| Actual source I/O on HDD | One stream per proven device; ambiguous aliases use the conservative shared group |
| Network/unknown storage | One stream per established endpoint/group; unresolved aliases share one conservative group |
| Positively identified local SSD | At most two source streams per physical device, at most four process-wide, enabled only after shared-scheduler measurement |
| Active checksum CPU work | At most two hash workers process-wide; one file's algorithms update sequentially |
| Active source buffers | One 256 KiB buffer per admitted active file; at most four / 1 MiB total; no unbounded read-ahead |
| Open source handles including paused files | Eight process-wide; paused ownership still consumes quota |
| Ready file window | 256 candidates per active job, also subject to byte accounting |
| Captured targets and retained results | 8 MiB per job and 64 MiB process-wide, charging native paths, vectors and strings |
| Presentation | Pages of at most 512 rows; bounded 64-event delivery queue with reserved terminal capacity |
| Progress | Coalesced at most 4 Hz per visible job, with a bounded aggregate dispatch batch |

Limits are implementation proposals to measure, not universal process-RSS caps.
If a complete top-level selection cannot be captured within its budget, refuse
admission as a whole; do not silently run a selected subset. During recursive
discovery, a resource/traversal failure stops that discovery and marks the job
incomplete, retaining finished rows and reporting unprocessed work. A later
spill-to-disk result store would need a separate bounded private-storage design;
this milestone does not grow memory indefinitely to handle arbitrarily many rows.

Begin with one active checksum stream if integration with the shared storage
arbiter is not yet ready. Do not enable SSD concurrency by adding an independent
semaphore alongside existing copy/scan limits. Physical aliases, two partitions
of one drive and virtual/RAID topology must not multiply permits. Positive SSD
detection is an optimization permission, not proof of immutable topology; errors
or ambiguous hints fall back to serial work.

Use bounded file-step scheduling, FIFO job admission with fair turns for ready
jobs, and lower priority than interactive navigation at the next permit boundary.
Do not interrupt a running OS call or alternate tiny chunks on HDD merely for
round-robin fairness. The measured scheduling quantum can retain sequential
locality while preventing large files from indefinitely monopolizing SSD/CPU
capacity. A paused file keeps its offset, hash states, handle and quota; a worker
does not block waiting for another permit or user decision.

## Results window, progress and cancellation

Ctrl+F8 opens a modeless **Checksums** window and submits a run with the captured
targets and saved algorithm choice (SHA-256 for first use). The user can continue
using either panel. The window shows source location, selection count, algorithms,
per-file status/bytes and digest columns. Stable row IDs preserve selection while
results arrive; sorting is backend work and never changes the job's targets.
Long names/digests can be horizontally scrolled or copied without truncation.

Algorithm and recursion controls describe the next run once calculation starts.
**Recalculate** creates a fresh generation from the same captured root selection
after current work is cancelled or complete; it does not add a new algorithm to
half-processed files or mix old and new digests. To hash a new panel selection,
invoke the command again. Changes detected while reopening for a new run are
explicitly reported, not silently substituted. The current run's controls display
the exact frozen options used to produce its rows.

States are **Preparing**, **Queued**, **Calculating**, **Cancelling** and a terminal
summary. Per-file terminals are **Calculated**, **Source changed**, **Read error**,
**Skipped** and **Cancelled**. The overall summary distinguishes fully completed,
completed with errors/skips, cancelled and incomplete/resource-limited runs.
An absent digest cell never looks like a successful empty digest. Failures on one
file normally allow other independent files to continue; fatal resource exhaustion
or cancellation stops the affected run with an explicit partial summary.

Show per-file bytes and aggregate processed bytes/count. While recursive totals
are still being discovered, display an indeterminate/partial total instead of a
misleading fixed percentage. Count source bytes once regardless of algorithm count.
Throttle/coalesce progress at 250 ms; send state changes and terminal results
without waiting for the next progress tick. Progress updates can be replaced by
newer snapshots; file terminals cannot be dropped. Session/job/generation checks
reject late events from an old or closed window.

**Cancel** stops further admission and requests cooperative cancellation of active
work. Keep already calculated rows; unfinished rows do not receive a final digest.
Closing a running window requests cancellation and can hide its presentation,
while the backend continues cleanup. No UI thread waits for a join or network
read. A blocked native read retains its handle, buffer and CPU/I/O ownership until
it returns; a timeout is not permission to free state or start replacement work
outside the budget. Application exit follows the existing responsive cancellation
and terminal-cleanup policy. No detached checksum workers survive application
shutdown.

## Copy and save

Copy actions operate on completed values: **Copy SHA-256** (or another visible
algorithm) copies only its canonical lowercase hexadecimal digest; **Copy rows**
includes filename, algorithm, digest and status with unambiguous escaped text.
No incomplete/failed value is copied as a checksum. Backend clipboard preparation
and the native clipboard service own large formatting and transfer work. A copied
display label is never later used to reopen a file.

**Save manifest** is an explicit write after calculation. Choose one algorithm
and a destination; emit only Calculated rows for that algorithm. If any rows are
excluded, show the exact included/excluded counts before saving a partial manifest.
Never silently export a partial run as if it covered every selected file. Save
does not reread/recompute source files; it records the captured run's results and
does not imply those files remain unchanged at export time.

Proposed interchange output is UTF-8 without BOM, LF line endings, lowercase hex,
and GNU-style untagged records: digest, one space, `*` binary marker, relative
filename. Hash exact file bytes on every OS, with no text-mode newline conversion.
Use algorithm-specific extensions `.md5`, `.sha1`, `.sha256` and `.sha512` and
show the chosen algorithm explicitly. Implement the documented GNU escaping for
backslash, newline and carriage return, including its leading escaped-line marker;
do not invent an ambiguous whitespace splitter. See the primary
[GNU checksum output-format specification](https://www.gnu.org/software/coreutils/manual/html_node/cksum-output-modes.html).

Manifest paths are relative to the manifest's containing directory, so moving a
manifest together with its covered tree preserves meaning. The save dialog
proposes the captured source root as destination and previews the resulting
relative paths. Initially refuse an export that would require absolute paths,
parent traversal or a different native volume; choose a common containing
directory instead. Convert native Windows separators to `/` only as a defined
format encoding; preserve literal Linux backslashes using the escaping rule.

Filenames must round-trip exactly through the selected format and target OS.
If a Linux name contains invalid UTF-8 or a Windows name cannot be represented
losslessly as Unicode scalar text, the initial UTF-8 export refuses that row/file
format and explains why. It must never use replacement characters, silently omit
the name or generate a manifest that refers to a different file. Hash-only copy
remains possible. A future native encoded-name manifest is a separate versioned
format, not a lossy fallback disguised as GNU text.

Manifest writing uses the backend's safe create/stage/sync/publish approach,
with explicit overwrite conflict handling, cancellation and exact native target
identity. Refuse a destination that aliases a hashed source, including a hardlink;
never overwrite the files just calculated. Do not implement Save by opening a
user-supplied path with truncate on the UI thread. Atomicity and overwrite
guarantees must match the platform's available publication contract; unsupported
guarantees produce a concrete error.

## Future manifest verification

A later **Verify checksums** action can consume `.md5`, `.sha1`, `.sha256` and
`.sha512` records plus explicitly recognized tagged `ALGORITHM (name) = digest`
forms. Salamander's SFV/CRC32 support is reference behavior, not an implicit
requirement to add CRC32 now. Accepting an extension does not replace validating
algorithm names, exact digest lengths/hex and every record's structure.

Parse manifests as untrusted bounded data on a backend worker. Reject ambiguous,
duplicate/conflicting, oversized or malformed records; use the selected manifest
directory as the explicit path scope. Refuse absolute paths, traversal, link
escapes, device files and hidden shell/URI interpretation. Unsupported raw-name
encodings must fail losslessly rather than being normalized. An imported filename
is data, never a command. Missing, unreadable, changed, mismatch and matching files
need distinct result states; neither an empty manifest nor all-skipped input counts
as successful verification. Specify import byte/record budgets and safe path
resolution in that implementation's extension before shipping it.

Verification compares newly calculated bytes with the supplied manifest. It is
not copy verification, source authentication or a digital signature. MD5/SHA-1
matches remain legacy compatibility results. The existing copy engine's mandatory
SHA-256 destination readback and mutation publication rules remain unchanged.

## Acceptance and validation plan

The implementation must validate observable behavior on Windows x64 and Linux x64:

| Area | Required evidence |
| --- | --- |
| Digests | Published known-answer vectors for all four algorithms; empty, binary, Unicode-content and chunk-boundary files; compare trusted independent tools |
| One-pass reading | Instrumented reader proves N source bytes are read once when 1–4 algorithms are selected; EOF handling and short reads preserve exact order |
| Identity and races | Replacement, same-size edits, truncation/growth, rename, final-link substitution and change during pause produce defined errors; no partial digest is Calculated |
| Targets | All selected regular files, no-selection focus fallback, unsupported-only selection, hidden rows, navigation/refresh during preparation and overlapping recursive roots |
| Scheduling | Measured 1/2/4-stream behavior on SSD, HDD, network and unknown storage; aliases share permits with copy/readback/scans; UI navigation remains responsive |
| Cancellation | Cancel during preparation, queueing, read, hashing, backpressure and save; slow reads retain permits until return; no late event can revive a closed generation |
| Bounds | Large files above 4 GiB with bounded memory; huge selections and recursion hit explicit budgets; no thread/task per file and no unbounded result/UI queue |
| Paths | Czech/CJK/Arabic/emoji, combining names, Windows long paths, raw Linux names, newlines, tabs, literal backslashes and leading punctuation; exact export round-trip or clear refusal |
| Manifests | GNU-compatible binary records and escapes; completed-only partial-export disclosure; save conflict/source alias/disk-full/cancellation; no source-file overwrite |
| Shortcuts | Ctrl+F8 new default only when free; custom/disabled bindings survive migration; Ctrl+F7 and Ctrl+F2 unchanged; visible hints match resolved bindings |
| UI | Modeless multi-file results, keyboard copy, screen-reader labels, stable selection while updates arrive, 4 Hz progress and honest partial/error summaries |
| Dependencies | Exact locks/features/licences/advisories reviewed, inventories/notices updated; no mandatory-fee dependency and no copied Salamander source/assets |

Do not report this feature as available until its command, backend, modeless UI,
shortcut migration, save path and tests are integrated. Developer scratch,
benchmarks, fixtures and generated reports stay under `_build/`; toolchain caches
remain under `.tools/`.

## Implementation boundaries and later decisions

The user specified the checksum workflow, MD5/SHA support and Ctrl+F8. SHA-256-only
default, optional SHA-512, explicit recursion, initial resource values and UTF-8
manifest policy are recommendations recorded here. Implementers should preserve
the observable ownership/integrity rules while measuring the scheduling quantum
and storage hint quality. Read-only permission failures should remain errors rather
than trigger automatic elevation or credential prompts.

Possible later work includes manifest verification, safe provider-backed content,
bounded disk-backed result history, additional hashes and a separate public
calculation-plugin API. None is implied to exist by this design, and none requires
changes to the current viewer ABI for the first checksum milestone.
