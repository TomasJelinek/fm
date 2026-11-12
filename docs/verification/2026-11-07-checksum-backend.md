# Checksum backend and engine adapter

This increment implements backend calculation and verified no-replace manifest
export, with engine command/event integration. It does **not** make Ctrl+F8 or
a modeless checksum window available yet. The user interface remains a separate step.

`midnight-core/src/checksum.rs` defines captured requests, algorithms, stable
job/generation/row identities, results/progress and explicit partial-export
acknowledgements. The engine's `checksum.rs`, `checksum_worker.rs` and
`checksum_manifest.rs` separate scheduling, streaming and export.
`checksum_actor.rs` adapts captured selections, generation state and manifest
lifecycle to engine commands. No viewer ABI, public plugin SDK, shortcuts or
settings are changed.

SHA-256 is the default. MD5/SHA-1 compatibility and SHA-512 share the same ordered
source stream: one reusable 256 KiB buffer, positive short reads until EOF,
transient read-interruption retry, native identity/version checks before and
after chunks and at EOF. Failed/changed/cancelled rows never carry final digests.
These are Calculated bytes, not authentication or an immutable snapshot. Copy
verification remains independent SHA-256 destination readback.

UI callers supply raw paths/captured metadata. Native identities are established
by backend TransferService preparation, or an optional expected identity can come
from prior backend preparation. No UI filesystem work is required. The first
native open establishes the initial identity when no earlier backend token was
supplied; this is not an assertion that a panel listing contained an immutable
filesystem snapshot. Final links/special files are never followed for content.
Recursion is explicit and uses bounded native no-follow traversal. Descendants
on a different native device are skipped before reading; skipped directory
subtrees mark discovery incomplete. The current transfer contract exposes
filesystem device identity, not Linux mount IDs, so same-filesystem bind mounts
are not independently distinguishable. A stronger no-mount-crossing policy needs
that native capability; no such validation is claimed here.

The service has a process-wide maximum of 32 admitted/retained jobs, a shared
64 MiB target/result quota and one active calculation worker. Quota ownership
travels with every row/run, including streamed batches; retaining a result keeps
its quota. Conservative reservation can reject admission before 32 jobs when
memory is full. Per-job defaults: 16,384 result rows, 64 directory frames,
1,000,000 examined nodes and 8 MiB retained data. The event channel holds at most
64 events with batches of at most 64 rows; progress is coalesced to 250 ms.
Limits are resource policies, not path-length limits or process RSS guarantees.
In-flight reads retain their handles/I/O permits until the native call returns.

Manifest export uses UTF-8 without BOM, LF, lowercase hexadecimal, GNU binary
records and the leading escaped-record marker for backslash/newline/carriage
return. Paths are relative to the containing directory, use `/` as the format
separator and must remain underneath that directory on the same native volume.
Unrepresentable native names are rejected without replacement characters.
Known exclusions and incomplete discovery require exact explicit acknowledgement.
Source paths/identity aliases cannot become save targets.

Encoding has a 16 MiB budget and at most two export workers. Save uses the common
verified-copy staging/sync/independent-readback/no-replace publication path, never
direct truncate/write. Existing targets return AlreadyExists; choose another
name. Overwrite/import verification are not implemented here. Export checks
calculated native identities against the destination volume before staging.
The actor holds the engine's shared mutation permit through publication and
cleanup; the synchronous save primitive is backend-only. Errors after publication
retain the published manifest and report any durability limitation explicitly.

The reference behavior was checked in Salamander's checksum `dialogs.cpp`
652-780 and `wrappers.cpp` around 257. No reference code/assets, ANSI buffers,
MAX_PATH assumption or implicit link following were copied.

Dependency evidence: exact released md-5/sha1 0.10.6 sources and both license texts
were inspected. Both enable only `std`, selecting MIT with attribution; no
optional assembly/OID/compression features. Existing sha2 0.10.9 supplies SHA-256
and SHA-512. SHA-1 0.10.7 changes only a disabled optional assembly feature.
Refreshed native Windows/Linux app captures regenerate **58 direct / 488 total /
456 names** in [the inventories](../THIRD_PARTY.md); full new license texts are
staged under `_build/goal/checksums/notices/` and the normal release packager
collects them from the refreshed selected graph.

The local advisory scan used database revision
`965bf9ad083b58e6340d65829d85217b18be9290`, 1,403 advisories, and found no matching
vulnerabilities. Five existing unmaintained warnings remain: bincode 2.0.1,
ttf-parser 0.25.1, zbus_names 4.3.5, zvariant 5.15.1 and zvariant_derive 5.15.1.
The scan used no-fetch/no-yanked; it does not establish current yank status or
absence of unknown vulnerabilities. Exact output is `audit.json` under the
checksum evidence directory.

Final current-source verification passed Windows19/Linux21 standalone backend
tests, three real-engine actor integration tests on each platform, and nineteen
filtered engine unit tests on each platform (five actor ownership/admission
cases plus fourteen component/boundary cases). Focused strict Clippy passed on
both platforms. Logs are `final-windows.log`, `final-linux.log`,
`final-units-windows.log`, `actor-units-linux.log` and `actor-clippy-*.log`.
Cases include cancellation during
channel backpressure, in-flight ownership through shutdown, incomplete discovery,
all four known-answer digests in one stream, Unicode/hardlink aliases and Linux
raw names/link siblings. No application
launch or user preference write is part of this backend verification. All logs,
metadata and generated output are under `_build/goal/checksums/` (shared inventory
captures stay under `_build/qa/dependency-inventory/`). macOS remains unimplemented
and unqualified; no native macOS result is claimed.

Engine integration accepts explicit or immutable selected requests. Selection
expansion runs on a blocking worker, preserves captured sizes and refuses missing
selected rows rather than using focus. Native preparation establishes actual
source identities. Admission reserves the process-wide aggregate lease before
queueing; selected requests reserve the complete bounded 8 MiB expansion.
Captured immutable directory indexes are shared with panel state; that existing
index storage is not a newly copied target buffer or an RSS quota.

New job IDs increase during an engine session; recalculation increases the
existing job's generation. Released jobs cannot be reopened by a stale command.
Current-generation results stay in backend state for saves, and retained row/run
clones continue owning quota. `ReleaseChecksums` cancels work and releases the
backend cache; UI-held clones must also be dropped to return their quota.
Full admission or obsolete generations return visible rejection events.

Manifest save IDs are nonzero, increase across the engine session and are consumed
even when rejected. At most two saves are pending/active, including mutation-gate
waiters. Admission Rejected is distinct from a running export's Finished, so a
duplicate-ID rejection cannot complete the original export. `CancelChecksumManifest` cancels only that export; it preserves the
calculated result. Queued and terminal statuses are reliable while the host
drains events; Saving is coalescible. Generation guards discard stale pending
events, and receivers must also reject tagged older events already delivered.
Normal engine shutdown awaits native checksum/save cleanup before aborting other
async tasks. Tests hold a native boundary deliberately to prove the mutation
permit remains owned after cancellation and shutdown waits for native return.
