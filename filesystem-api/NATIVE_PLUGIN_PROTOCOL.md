# Native filesystem protocol 1 (ABI 1.3)

This is an additive in-process filesystem service. The existing archive worker
protocol 1 and viewer ABI are unchanged. Native packages are trusted code; loading
them is not sandboxing. The host must use its existing package trust, canonical
path and restricted library-loading checks before calling the entry point.

The public Rust model/codec is `midnight-filesystem-plugin-api`; Rust authors can
use `midnight-filesystem-plugin-sdk`. Both use only the standard library. Their
independent workspace is this directory; native packages are separate Cargo
workspaces depending only on these public crates and their OS bindings.

## Entry and lifetime

`midnight_filesystem_entry_v1(Api *out, uint32_t capacity)` fills the descriptor
declared in [the C header](include/midnight_filesystem.h). The current ABI is
major 1, minor 3, service kind 2. Explicit structure capacities are required.
Older minor descriptors remain valid when their capabilities and minimum host
minor are compatible. Function pointers are nullable in the representation;
required null callbacks fail validation.
The host must reject incompatible major/minimum-host-minor values, unsupported
capability bits and invalid bounded ID/version metadata before creating instances.

Callbacks are synchronous: create, call, release_buffer, destroy. All calls,
encoding and resource destruction run on backend workers. The host retains the
loaded library through every instance, active call, callback and reply buffer.
Instance and buffer IDs are nonzero and descriptor-owned. Native node IDs belong
to one mount; cursor, reader and prepared-Rename IDs belong to one instance.
Prepared Rename resources also retain their originating mount. Do not compare
IDs across descriptors, instances or mounts, persist them, or derive native
paths from them. The SDK never reuses IDs within its loaded module.

Cancellation is one borrowed callback/context for one synchronous call. It must
be thread-safe, non-reentrant and mapped until return. The SDK checks before
handlers and, for non-effectful requests, before reply encoding; adapters poll
between bounded OS calls and chunks. Cancellation does not interrupt an arbitrary
kernel call. Close operations and GetRenameState ignore cancellation. Rename has
an initial cancellation check, but a late flag never rewrites its actual result.
Its adapter must finish required recovery after the commit gate; see ABI 1.3 below.
The caller must adopt a successful late response before dropping it, then close
its resources if the result has become obsolete. A cancel flag cannot revoke a
published buffer, close a reader or undo a completed namespace change.

The SDK reserves reply capacity before invoking the handler. Newly created
resources use `Reply::owned(response, completion)`: completion receives `true`
after encoding and retaining the reply buffer succeeds, or `false` on failed
publication. Encoding/publication failure or late observed cancellation rolls
back unpublished mounts/readers and prepared Rename pins. Unpublishable pages
invalidate and close their cursor, rather than skip entries silently. Cursor
admission stays busy through publication. Cleanup ignores cancellation.
`Reply::stateless` refuses Resolved, Opened, Page and RenamePrepared responses.
Authors retain new resource ownership in the closure and perform required rollback.
A Rename attempt instead retains its recorded outcome after failed delivery; its
owned completion releases execution admission without undoing namespace effects.

Completion error/panic closes further admission for that instance, with one narrow
exception: `completion(true)` may return StaleReference only after fully rolling
back resources that lost a race with mount/generation retirement. The SDK releases
the encoded buffer and returns C UNKNOWN_HANDLE without poisoning other mounts.
A rollback failure, including any failure of `completion(false)`, still poisons
admission. This exception does not authorize abandoning cleanup or undoing an
executed Rename when its reply is lost.

The host releases every successful buffer exactly once through the same instance
and descriptor. A failed call returns a zero buffer. Destroy refuses active calls
or retained buffers; successful destruction retires remaining native resources.
No author function or large destructor runs while registry locks are held.

## Bounded transport

Every payload begins with bytes `46 53 42 52 01 00 00 00` (`FSBR`, version 1).
Integers are fixed-width little-endian, booleans are one byte 0/1, blobs/text have
u64 byte lengths and vectors have u32 counts. Display text, IDs, diagnostics and
explicitly UTF-8-tagged native text require valid UTF-8. Request operation is the
C Call's u32 field. Response
has a u16 tag. The public encoder/decoder and model are the normative field-order
schema; no Rust layout, allocator, String or Vec crosses the C boundary.
Decoders reject unknown tags, malformed lengths, invalid native encodings and
trailing bytes before accepting a response. Hosts validate operation/response
pairing and requested counts separately; decoding a valid frame alone does not
make it the response to a particular call.

Default budgets: request 32 MiB, reply 1 MiB, retained reply allocations 16 MiB,
64 retained/reserved buffers and 8 active calls per instance. Config can lower
these within the public ceilings. A smaller reply budget can make otherwise
valid operations fail explicitly. No path is truncated or capped at MAX_PATH;
transport/allocation limits remain explicit resource limits.

The native author helper additionally bounds mounts to 64, cursors to 32, readers
to 64, cached node records to one million per mount and conservative aggregate
native cache charges to 256 MiB per instance. In-flight native opens consume
their slot before OS I/O; failed publication returns those slots. Prepared
Rename pins have additional slot, charge and receipt admission described below.

## Operations

| Op | Request fields | Success |
| --- | --- | --- |
| 1 Resolve | address | mount, Node |
| 2 Stat | mount, node | Node |
| 3 ListBegin | mount, directory, max_entries | Page, ordinal 0 |
| 4 ListNext | cursor, expected_page, max_entries | Page |
| 5 CloseList | cursor | Unit |
| 6 Parent | mount, node | optional Node |
| 7 OpenRead | mount, node | reader, length, revision bytes |
| 8 ValidateRead | reader | Unit |
| 9 ReadAt | reader, offset, length | bytes, eof |
| 10 CloseRead | reader | Unit |
| 11 CloseMount | mount | Unit; Busy while dependent operations exist |
| 12 Volumes | none | volume IDs/captions/root addresses |
| 13 StorageKind | address | Unknown/LocalSsd/Rotational/Network |
| 14 SpaceInfo | address | available, total bytes |
| 15 VolumeLabel | address | optional raw native text; requires VolumeDetails |
| 16 VolumeInformation | address | independent measured volume fields; requires VolumeDetails |
| 17 Activate | mount, node | Activated; requires Activate |
| 18 Refresh | mount, node | Unit; requires Refresh |
| 19 ResolveText | UTF-8 text | Resolved; requires ResolveText |
| 20 PrepareRename | mount, node | RenamePrepared; requires PreparedRename |
| 21 Rename | prepared, attempt, raw name, policy | RenameState; requires PreparedRename |
| 22 GetRenameState | prepared | RenameState; requires PreparedRename |
| 23 CloseRename | prepared | Unit; requires PreparedRename |

Capabilities independently advertise Browse, Read, Stat, Volumes, Storage,
NativeAddress, VolumeDetails, Activate, Refresh, ResolveText and PreparedRename.
Hosts check optional capabilities before dispatch. Unsupported services return
Unsupported, never a host-native fallback. Failed responses carry a typed error,
optional OS code and bounded diagnostic. Rename also carries failures inside
Unchanged and RecoveryRequired outcomes. C status errors cover framing, admission,
unknown instance/buffer, incompatibility, shutdown and panic.

### Optional volume information (ABI minor 1)

`CAP_VOLUME_DETAILS` is 64 and its manifest spelling is `volume-details`. The
host checks this capability before operations 15 or 16. ABI major, descriptor
layout, operations 1–14 and response tags 1–11 are unchanged. Existing plugins
remain accepted by new hosts; existing SDK adapters inherit Unsupported defaults.
Packages using only this extension may retain minimum-host-minor 0 because old
hosts reject the unknown manifest capability before loading. Other extensions
can require a higher minimum. Hosts that recognize the capability
must implement these payloads. An older plugin is never sent an optional query
without advertising support. Missing support is explicit, with no native fallback.

The new reply tags are 12 (`VolumeLabel`) and 13 (`VolumeInformation`); Failed
remains 11. Both requests contain exactly the existing encoded address. Native
text is an optional-present boolean followed, when present, by name encoding and
a length-prefixed raw byte sequence. UTF-8, UTF-16LE and Unix bytes are supported;
opaque, NUL-containing, malformed UTF-8 and odd-length UTF-16 values are rejected.
Unpaired UTF-16 units and invalid Unix UTF-8 are preserved. Each native textual
field is at most 64 KiB. Returned roots remain native addresses bounded by the
existing 1 MiB reply transport budget, without an additional text/path cutoff.

VolumeInformation encodes, in order: optional address root; optional native label,
filesystem and source; optional u64 physical total, filesystem free and caller
available; storage-kind byte; optional read-only boolean; u32 property count and
property bytes; u32 diagnostic count and UTF-8 diagnostic strings. Optional scalar
fields use a present boolean followed by their value. Properties use the public
`VolumeProperty` values 1–14, appear at most once, and number at most 14. At most
16 diagnostics of at most 4096 bytes describe partial failures. Present free or
available cannot exceed a present total. Unknown and absent values are preserved;
availability is never substituted for filesystem free or physical total.

Windows resolves the existing literal path before volume metadata, suppresses
media error dialogs on the calling thread, and uses GetDiskSpaceInformationW for
quota-independent actual total. Legacy SpaceInfo continues to honor caller quotas.
Linux queries capacity and flags from a held O_PATH object; mount ID and device
identify its exact mountinfo record, and bounded udev metadata supplies optional
raw labels. Partial failures retain independent fields. Metadata queries and
cleanup run on bounded backend workers; cancellation is checked between OS calls.
One synchronous OS call cannot be forcibly interrupted by this ABI.

SSD hints require positive local physical evidence: Windows validates the single
disk extent, nonremovable ATA/SATA/NVMe descriptor and seek-penalty query; Linux
validates local mount/device identity, physical PCIe NVMe or ATA topology and
rotation metadata. Virtual, stacked, removable, unsupported and ambiguous devices
remain Unknown. Recognized network mounts remain Network. No read-only property
is inferred solely from a filesystem name.

Listing starts an OS iterator and returns at most 128 entries per call. It does
not materialize the directory before the first batch. Each Next supplies the
exact next ordinal; concurrent Next returns Busy and stale ordinals cannot
advance or close the cursor. CloseList is required even after done. Enumeration
is live, not a snapshot; external edits can cause missing entries, partial
failure or SourceChanged. Old native tokens reject an observed kind change.
Explicit navigation into a newly resolved directory link remains possible.

ReadAt requests 1–256 KiB at an arbitrary u64 offset. Sources are regular files
with known length, retained OS handles and before/after revision validation.
The expected byte count is min(requested, captured length minus offset); shorter
reads fail rather than invent EOF. Offset equal to length returns empty/eof.
Revision bytes are opaque (1–4096), not persistent object identity or a hash.
CloseRead keeps any concurrently active reader alive until its call returns.

## Native text and safety limits

Address encodings are opaque, NUL-free UTF-16LE code units (including unpaired
surrogates) or NUL-free Unix bytes. Name encoding is independently tagged;
display captions are never native identity. Missing metadata remains missing;
size exists only for regular files. Linux reports Unix mode, not fabricated
Windows attributes. Timestamp seconds are signed Unix seconds with nanoseconds
0–999999999. Unknown storage/compression is not a positive SSD/uncompressed hint.

Windows OS operations use explicit verbatim disk/UNC paths to preserve literal
trailing dots/spaces and DOS-looking names beneath an ordinary root. Ambiguous
dot-component or unsupported device namespaces return Unsupported rather than
changing the addressed object. Supplied provenance addresses remain lossless.
New Rename destination components have the stricter namespace policy below.
Linux walks components through descriptors, allowing paths beyond PATH_MAX.
Content qualification uses O_PATH/no-follow before reopening the exact retained
regular inode through the internal proc descriptor namespace. External procfs,
sysfs and similar content whose advertised size is not a known file length is
Unsupported; this does not forbid internal `/proc/self/fd` handle reopening.

Reads are LiveValidated, not immutable snapshots. Identity, size, modified and
change stamps detect observable changes, but filesystems can coalesce metadata
updates in one clock tick; malicious writers can defeat stamp-only validation.
These handles/revisions grant no mutation authority and cannot replace the
application's stronger verified-copy/remove protocols.

Browse/Read capabilities grant no mutation authority. The separate ABI 1.3
PreparedRename extension defines only prepared same-parent Rename; this protocol
does not define generic writes, Watch, delete, recursive traversal, authentication,
executable launch or UI callbacks. Host composition must route listing and native
F3 capture through loaded packages and disable those capabilities when a package
is absent; independent conformance tests alone do not establish host cutover.

## Optional explicit activation, ABI minor 2

Independent virtual providers may advertise `CAP_ACTIVATE = 128`. Their
descriptor and manifest require host minor at least 2. Native providers do not advertise
it and return Unsupported; all existing operation tags and C layouts remain
unchanged. Operation 17 carries `(mount: u64, node: u64)` for an already-issued
node, both nonzero. Hosts invoke it only for an explicit user navigation action,
never restore, stat, listing, icons, previews or hover.

Response tag 14 carries a one-byte target kind and an Address: kind 1 is
WithinProvider (provider-owned address, never a native-path fallback); kind 2
is Native (absolute Windows UTF-16LE or Unix-byte path). Opaque/relative/native
NUL addresses fail codec validation. The host also verifies that native encoding
matches its platform, retains original provider/back context until success, and
uses its own native provider. It does not launch a shell or interpret a URL as a
path. A failed handoff leaves the original virtual location visible.

Additional failure codes 11–15 are Unavailable, Offline,
AuthenticationRequired, MountRequired and TimedOut. The original 1–10 meanings
remain unchanged. AuthenticationRequired requires an observed authentication
challenge; PermissionDenied must not be guessed to mean a missing password.
These messages carry no passwords or credentials. Network-provider policy and
the later host broker determine permitted explicit connection actions; the ABI
does not grant a virtual node mutation or content-read authority.

`CAP_REFRESH = 256` admits operation 18, the same `(mount,node)` payload, returning
Unit after invalidating the selected directory's TTL cache. It does not activate
the location, mount a share or authenticate; later listings use new data. A late
old listing cannot repopulate an explicitly invalidated cache generation.

`CAP_RESOLVE_TEXT = 512` admits operation 19, one UTF-8 length-prefixed string of
explicit user input (nonempty, no NUL). It returns the existing Resolved response.
The plugin parses its own user-facing syntax without opening the endpoint. The
host never constructs a provider's opaque storage codec or uses a display caption
as an address. This allows manual server entry when discovery is unavailable.
These capabilities also require host minor at least 2; no old operation is repurposed.

## Optional prepared Rename, ABI minor 3

`CAP_PREPARED_RENAME = 1024` adds operations 20–23 and response tags 15–16.
The C descriptor layout, entry symbol, wire prefix and older operation/response
tags are unchanged. An advertising descriptor must set ABI minor at least 3 and
`minimum_host_minor = 3`; the current API validator rejects a larger host minimum.
The SDK entry helper sets this minimum automatically. Package metadata must also
require host minor 3. Adapters that do not implement the extension inherit
Unsupported from `NativeAdapter::prepare_rename` and must not advertise support.

### Preparation and mutation authority

PrepareRename selects an already-issued `(mount,node)` namespace locator. Browse
does not acquire Rename pins or attest the object's listing-era identity. The
first authoritative source and parent pins are captured and binding-validated
during Prepare, before confirmation. A same-kind replacement since listing may
therefore be the current object returned by Prepare. The host presents that
prepared snapshot for confirmation; listing metadata is not an identity check.

The SDK checks the pinned basename and compatible kind before publishing the
prepared resource. DirectoryLink/Symlink hints do not grant authority over a link's
referent. The public `native::NativeRename` trait retains the source/parent pin;
`prepared()` returns its cached immutable snapshot without native I/O. Execution
must revalidate the retained source, parent and relevant target bindings, rather
than reopen a pathname to choose a replacement source. A failed binding check
does not silently prepare a different object.

Rename accepts one raw destination basename in the retained parent. It is not a
cross-parent Move or a generic write service. The initial policy is NoReplace.
RecoverableCurrentEntry consent applies only to the last reported conflict for
that prepared handle and the identical destination encoding/bytes. Both the
prepared source's `recoverable_replace` and conflict's `can_replace` must be true;
source and target must be regular files with distinct identities. Directories,
links, a second name for the source identity and stale/foreign conflict tokens
do not qualify. The SDK assigns conflict tokens; tokens do not replace native
target pins or the adapter's revalidation. A changed target requires fresh conflict
evidence and consent. No unconditional overwrite policy exists.

### Public model, wire representation and bounds

Operation 20 carries two nonzero u64 values, mount then node. Operation 21 carries
nonzero prepared and attempt u64 values, NativeText, then a policy byte: 0 for
NoReplace, or 1 followed by the nonzero conflict u64 for RecoverableCurrentEntry.
Operations 22 and 23 carry only a nonzero prepared u64. The SDK pairs Prepare
with RenamePrepared/Failed, Rename and query with RenameState/Failed, and close
with Unit/Failed.

Response tag 15 (RenamePrepared) contains handle, source RenameObject, parent
identity blob and recoverable-replace boolean. RenameObject contains identity,
revision, raw name, kind and Metadata. Tag 16 (RenameState) contains prepared
handle, attempt and outcome. Names carry a NameEncoding byte and raw-byte blob.
The [public model](midnight-filesystem-plugin-api/src/rename.rs) and
[encoder](midnight-filesystem-plugin-api/src/wire/encode.rs) define exact field
order; the [decoder](midnight-filesystem-plugin-api/src/wire/decode.rs) rejects
unknown nested tags, invalid booleans, malformed lengths and trailing bytes.

| Outcome byte | Meaning and carried data |
| --- | --- |
| 0 Ready | No attempt yet; state attempt is 0. |
| 1 Executing | An admitted attempt is in progress. |
| 2 Conflict | Token, target RenameObject and can-replace flag; fresh consent required for replacement. |
| 3 Unchanged | Terminal failure with no residual namespace change; temporary staging may have occurred. |
| 4 Renamed | Actual name, same-entry flag, recovery records and durability-confirmed flag. |
| 5 RecoveryRequired | Source location, optional destination name, recovery records and failure. |

Every non-Ready state has a nonzero attempt. Recovery source-location bytes are
0 OriginalName, 1 Destination, 2 Recovery and 3 Unknown. Each recovery record
contains role, exact Address and identity: roles are 0 Source, 1 DisplacedTarget
and 2 EmptyStage. An empty recovery identity means identity could not be confirmed.
These locators report residual entries; they grant no later write authority.
Renamed may retain recovery records and may report durability unconfirmed.
RecoveryRequired must not be presented as an unchanged or fully completed rename.

Each raw name is at most 64 KiB. Source/target and parent identities are nonempty
and at most 256 bytes; object revisions are nonempty and at most 4096 bytes.
At most four recovery records share 256 KiB of address bytes, with at most 256
identity bytes per record. Failure diagnostics are at most 4096 UTF-8 bytes.
`MAX_RENAME_RECEIPT_BYTES` is 384 KiB, an upper bound for a valid receipt including
the longest names and recovery locators. Transport budgets still apply.

Names support UTF-8 (tag 1), Windows UTF-16LE (2) and Unix bytes (3), preserving
unpaired UTF-16 units and non-UTF-8 Unix bytes without normalization. Opaque
names, empty names, NUL, slash and exact `.`/`..` are rejected; Windows names also
reject backslash and odd byte lengths. Backslash remains a valid Unix name byte.
Observed source/conflict names may preserve extended Windows spellings. Proposed
Windows destinations additionally reject control characters, `<>:"/\|?*`,
trailing dot/space and reserved device stems, including COM/LPT digit forms.
These are structural/namespace checks, not filesystem access; native adapters
must additionally enforce their own commit policy and platform name limits.
RenameObject excludes Other; size metadata is present only for a regular file.

### SDK ledger, admission and lifecycle

The public codec validates structure; the SDK's `native::NativeFilesystem<A>`
adds the retained ledger and admission policy described here. Independent
implementations must supply the corresponding native identity and recovery
behavior; the codec alone cannot establish it.

A published prepared handle starts at Ready, attempt 0. Its first Rename uses
attempt 1. Repeating the current attempt replays the recorded state only when
the exact raw name and policy match; changed input is Invalid, and an older
attempt is StaleReference. Only Conflict allows the next consecutive attempt.
When another attempt is allowed, skipped numbers are Invalid. After a terminal
outcome, newer attempts are StaleReference. Unchanged, Renamed and RecoveryRequired
are terminal for execution, although the current attempt remains replayable.

The SDK records Executing before native execution and records the resulting
outcome before reply cloning, encoding or publication. GetRenameState can observe
Executing without executing or canceling anything. Concurrent Rename and
CloseRename return Busy while that handle is executing or awaiting publication
completion. A lost effectful reply leaves the handle and recorded result intact:
query it, or replay the identical current attempt on the same live handle.
Do not reprepare automatically or infer that failed delivery rolled back effects.
Malformed adapter receipts become terminal RecoveryRequired with unknown source
location, rather than a fabricated Unchanged result. A provider panic poisons
instance admission; retaining an internal unknown outcome does not promise it
can subsequently be queried.

The native helper admits at most 32 opening/live Rename resources. Each reserves
`RENAME_RESOURCE_BYTES = 1 MiB` within its existing 256 MiB native allocation
budget. The charge covers native pins, captured paths, conflict state and the
ledger/receipts. Adapter-private capacities must fit that charge; the SDK cannot
inspect allocations hidden behind the trait object. Slots/charges are held
through native disposal. Before execute, the SDK requires a configured reply
budget of at least 384 KiB and admits input, receipt capacity and conflict ID
before native work. A failure of this SDK admission occurs before native execute.

On first PrepareRename or query/close admission, the SDK reserves one shared
control lane for GetRenameState and CloseRename: one active call, one retained
or reserved buffer and one maximum reply allocation. This share comes from
Config's existing total ceilings. Admission requires at least two calls, two
buffers and `retained_buffer_bytes >= 2 * reply_bytes`; existing ordinary
pressure can delay/refuse that reservation. Legacy-only instances keep their
full capacity. Once reserved, ordinary requests cannot consume the control share.
Query and close share it, so release a query buffer before requesting close.
The reservation persists for the instance; it is not a guarantee against control
lane saturation, allocation failure, poisoning or shutdown.

GetRenameState and CloseRename ignore cancellation. Rename may be refused by
the initial SDK cancellation check before an attempt starts; after admission,
an observed pre-execute cancellation can record terminal Unchanged(Cancelled).
After native execution begins, adapters must reserve exact recovery receipts
before writes and finish required recovery beyond the commit gate despite
cancellation. Late cancellation does not replace the actual Rename result.

Every call into native execute invalidates the originating Browse epoch, even
when the returned outcome is Conflict or Unchanged: transient staging/restoration
may have occurred. Old Browse nodes and cursors then fail as stale. Existing read
handles retain their separate LiveValidated contract. Retained prepared handles
continue to support query, identical replay, close and a next attempt after
Conflict, subject to native pin revalidation. Refresh by resolving a new Browse
mount; do not select a new source by reopening the old name.

CloseRename removes the idle handle and disposes pins; it does not undo an
outcome or erase residual recovery entries. CloseMount stays Busy while prepared
resources retain it. Query/replay recovery is limited to a live, known handle
in a healthy instance, not a durable journal: close, destruction, poisoning or
process failure do not promise retrieval or automatic rollback.

This section specifies the public API and implemented SDK policy. Capability
availability, platform adapter qualification and host routing require separate
integration; these declarations do not establish Windows/Linux provider support
or host cutover.
