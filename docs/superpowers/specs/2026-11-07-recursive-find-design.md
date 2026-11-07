# Native recursive Find backend

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

Default and hard limits are 10,000 retained results, 1,000,000 examined nodes,
128 retained directory frames, 64 MiB retained result/path accounting, 1 GiB
content per file and 16 GiB content per request. Callers can lower limits for
small searches/tests. Limits are resource budgets, not path-length ceilings.
Four fixed controller slots retain at most four active sessions and one latest
pending request per slot. At most two scan workers run; a blocked native call
retains its permit until it actually returns. Result validation has a separate
bounded request queue and one worker.

Mask matching reuses the existing bounded mask compiler, including exclusions,
`*.*`, Unicode case handling and keeping undecodable native names visible. Regex
Find is not introduced. Content search accepts strict UTF-8, optionally with a
UTF-8 BOM; undecodable/UTF-16 content is reported as an encoding error, never an
ordinary no-match. Decoding keeps a 64 KiB read buffer and at most three pending
UTF-8 bytes. A literal KMP matcher keeps only the bounded query/prefix state across
chunks, preserving split scalars and matches. Case insensitive content folds each
decoded fragment without normalization. Even after a match, decoding continues
to detect malformed trailing content. No file body is retained, logged or sent
to the UI.

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
kind, metadata, provider identity and opaque revision. Batches contain at most 64
results and use bounded output backpressure. The controller compares session and
generation again at publication after waiting for output capacity. Superseded
workers cannot publish new-generation results. Consumers must still reject queued
old-generation events. Progress and the final summary disclose examined files,
directories, bytes, results, skipped entries, errors and reached budgets.
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

The [Find service/UI guide](../../developer/FIND.md) and
[Find window plan](../plans/2026-11-07-find-window.md) describe the implemented native
modeless window, configurable Alt+F7 action, lazy result models and guarded read-only
panel reveal. Final tooltip/compact/native qualification is tracked there. Actual
program execution from a Find result remains unsupported; a future action must
validate at its own backend execution boundary. Archives and other dynamic
providers remain unsupported until they expose equivalent read-only enumeration,
identity, revision and content capabilities.
