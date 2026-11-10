# Native recursive Find backend

Implemented name matching offers the same Wildcards/Masks, Regex and Expression
modes as panel filters, through the existing backend compiler and limits. Results
use a single aligned row per entry. This does not change literal content matching.
The requirements are in
[the panel/viewer follow-up specification](2026-11-07-panel-viewer-followups.md);
the service guide records current implementation and platform qualification.

FIND-01 uses a bounded backend for a separate modeless results session.
The accepted follow-up stage 5 requires name masks, captured starting locations,
optional content text, incremental results, progress and cancellation. The UI is
implemented in the separate Find window plan; this backend evidence does not
by itself close FIND-01 native workflow qualification.

## Reference and approach

Salamander `src/find.cpp:120` defaults to subdirectories enabled, case insensitive,
literal text, without whole-word, hex or regex options. `finddlg1.cpp:1372` defaults
the filename mask to `*.*`; `:1724` binds starting locations and recursive/content
options; `:1841` parses multiple locations. Its results and worker are independent
of panel navigation. This project adopts these defaults without copying source,
assets, ANSI conversions or fixed path buffers.

The relative-age/duplicate follow-up examined `src/filter.cpp:228-255` (one
prepared minute/hour interval), `src/find.cpp:1864` (prepare before traversal),
`:1006-1025` (size/name pruning before digest work), `:819-885` (bounded digest
reads and cancellation), and `src/finddlg1.cpp:460-466,730` (group-aware ordering).
Only behavior informed this implementation. It uses the existing SHA-256 crate
plus exact byte comparison and accepts short reads without treating them as EOF;
the reference's MD5/ANSI/fixed-path implementation was not copied.

The approved approach is a portable bounded worker over a dedicated native
`FindService`/`FindSource` contract. Existing `DirectoryService` streams paths but
cannot pin recursion identity. `TransferSource` already pins identity, but rejects
filesystem roots and obtains copy-oriented data/share access even for names-only
search. A dedicated read-only adapter can support roots, lazily open content, and
distinguish consumed-child errors from directory-enumeration failure. Native
Windows and Linux implementations stay in separate files and reuse existing
audited native opening primitives where their contracts fit.

## Inputs and limits

Each request identifies a modeless session and monotonically increasing generation.
It captures 1–16 absolute native directory roots, masks, recursion, explicit mask
case handling and optional literal UTF-8 text with explicit case handling. Empty
content queries and NUL are invalid. Starting roots may cross filesystems; recursive
children do not cross a mount/volume or follow symlinks/reparse points. Names are
kept as raw native paths, and display text is never used as identity.
The 10 November extension adds optional whole-word content matching, a positive
modified-within duration up to 366 days, and content-duplicate mode. Defaults keep
substring search with no age restriction or duplicate grouping. The backend
freezes the reference clock at admission; the inclusive age interval excludes
future and unknown timestamps and does not move while queued or scanning.

Default and hard limits are 10,000 retained results, 1,000,000 examined nodes,
128 retained directory frames, 64 MiB retained result/path accounting, 1 GiB
content per file and 16 GiB content per request. Duplicate mode additionally limits
retained candidate identities to 100,000. Candidate vector/hash capacity and
comparison/group scratch share the retained-memory budget, and all hash/compare
read bytes share the total-content budget. Callers can lower limits for
small searches/tests. Limits are resource budgets, not path-length ceilings.
Four fixed controller slots retain at most four active sessions and one latest
pending request per slot. At most two scan workers run; a blocked native call
retains its permit until it actually returns. Result validation has a separate
bounded request queue and one worker.

Mask matching reuses the existing bounded mask compiler, including exclusions,
`*.*`, Unicode case handling and keeping undecodable native names visible. Regex
and typed metadata Expression modes reuse the corresponding panel compilers.
Content search accepts strict UTF-8 with an optional BOM, and BOM-marked UTF-16
little/big endian. Unsupported/malformed encodings produce errors rather than
ordinary no-matches. Decoding keeps a 64 KiB read buffer plus bounded reusable
decoder storage and incomplete scalar state. A prepared literal KMP matcher
reuses bounded query/prefix state across files, preserving split scalars and
matches. Case-insensitive content uses Unicode folding without normalization.
Whole-word mode retains both boundaries across reads and case-fold expansions.
Even after a match, decoding continues to detect malformed trailing content.
No file body is retained, logged or sent
to the UI. BOM-less UTF-16, legacy encodings, hex queries and regex content are
outside the current contract.

Duplicate mode applies all ordinary predicates before grouping regular files by
size. It deduplicates native identities across roots, excluding hard-link aliases,
and never reads singleton size buckets. Equal-size candidates receive streamed
SHA-256 hashes; equal hashes then require full byte equality with independently
reopened/revalidated sources. Empty and arbitrary binary files are supported.
Two reusable 64 KiB buffers bound content memory. Every representative is checked
for each comparison and every member is checked again before publication. A hash
error excludes that file; a comparison/revision error excludes the digest bucket
before publishing any of its groups. This cannot establish a point-in-time
snapshot against writers that evade the provider's metadata revision checks.

## Traversal, output and integrity

Depth-first traversal retains only its bounded stack of pinned directory handles.
Each source child call consumes one entry and returns an ordinary pinned file or
directory, an explicit skipped entry, end of stream, or a fatal directory error.
Links and special entries are skipped. Native mount crossings are skipped.
Overlapping roots may produce separate path results; within each root identity
ancestry prevents loops. Errors are sampled to at most 32 diagnostics while all
counts remain available. Cancellation is checked around native calls and output
backpressure; already-running OS calls cannot be forcibly interrupted.

Results have stable IDs within their generation and share immutable raw path,
kind, metadata, provider identity and opaque revision. Ordinary batches contain at
most 64 results and use bounded output backpressure. Each duplicate group is one
atomic batch of at least two and at most 10,000 results, with a stable group ID
and complete discovered member count. Remaining result capacity must fit the
whole group; cancellation or rejected output cannot publish only part of it.
The controller compares session and
generation again at publication after waiting for output capacity. Superseded
workers cannot publish new-generation results. Consumers must still reject queued
old-generation events. Progress and the final summary disclose examined files,
directories, bytes, results, skipped entries, errors and reached budgets. Duplicate
phases additionally report candidate/group counts and their actual read bytes.
The sampled raw current-location slot shows directories during traversal and
files during hashing/comparison. Group-aware sorting preserves adjacent members
and stable IDs under every selectable column/direction.
New session IDs monotonically increase for the engine lifetime; closed IDs cannot
be reused, so late callbacks cannot alias a newly opened session's result IDs.
Captured native metadata includes date/time strings from the existing per-OS
backend formatter. This happens once at source capture, outside UI callbacks and
content chunk processing; raw revalidation snapshots do not repeat formatting.

Completed means the declared native scope finished. Access/encoding errors and
resource omissions produce Incomplete. Explicit cancellation produces Cancelled;
superseded generations may stop without further output. Unsupported providers are
explicit failures. This is a live search, not a filesystem snapshot.

Result validation takes the retained session/generation/result ID, reopens the
native object without following links, compares identity and the captured revision,
and revalidates immediately before returning a target. Unknown, stale, changed or
missing results fail. No executable command or destructive operation is accepted
by this milestone. A future Open/Reveal action must call this backend validation
again at execution and preserve the platform's stated namespace-race limits;
an earlier UI path callback is not authority to activate a replacement file.

## Verification and remaining integration

Tests cover roots, recursion off/on, masks, Unicode/raw names, BOM, chunk-spanning
content, case folding, malformed encodings, access failures, resource budgets,
result substitution, cancellation under full output queues and superseding a
running generation. Real native Windows/Linux temporary fixtures verify provider
boundaries. All generated output stays under `_build`; dependencies are unchanged.
The 10 November backend extension adds actual Windows modification-time,
hard-link, binary/empty duplicate and group-budget fixtures. Explicitly simulated
failure cases cover denied/short reads, changed reopened captures, a representative
changing after an earlier successful comparison, equal-hash/unequal byte streams
and cancellation after a single content read. A 70-member group proves atomic
publication and retained results after cancellation. These source/backend checks
do not imply installed desktop or Linux-runtime qualification of new controls.

The [Find service/UI guide](../../developer/FIND.md) and
[Find window plan](../plans/2026-11-07-find-window.md) describe the implemented native
modeless window, configurable Alt+F7 action, lazy result models and guarded read-only
panel reveal. Final tooltip/compact/native qualification is tracked there. Actual
program execution from a Find result remains unsupported; a future action must
validate at its own backend execution boundary. Archives and other dynamic
providers remain unsupported until they expose equivalent read-only enumeration,
identity, revision and content capabilities.
