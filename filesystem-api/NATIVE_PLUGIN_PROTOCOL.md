# Native filesystem Browse/Read protocol 1

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
declared in [the C header](include/midnight_filesystem.h). ABI major 1, minor 0,
service kind 2 and explicit structure capacities are required. Function pointers
are nullable in the representation; required null callbacks fail validation.
The host must reject incompatible major/minimum-host-minor values, unsupported
capability bits and invalid bounded ID/version metadata before creating instances.

Callbacks are synchronous: create, call, release_buffer, destroy. All calls,
encoding and resource destruction run on backend workers. The host retains the
loaded library through every instance, active call, callback and reply buffer.
Instance and buffer IDs are nonzero and descriptor-owned. Native node IDs belong
to one mount; cursor and reader IDs belong to one instance. Do not compare IDs
across descriptors, instances or mounts, persist them, or derive native paths
from them. The SDK never reuses IDs within its loaded module.

Cancellation is one borrowed callback/context for one synchronous call. It must
be thread-safe, non-reentrant and mapped until return. The SDK checks before and
after handlers; adapters poll between bounded OS calls and chunks. Cancellation
does not interrupt an arbitrary kernel call. Close operations ignore cancellation.
The caller must adopt a successful late response before dropping it, then close
its resources if the result has become obsolete. A cancel flag cannot revoke a
published buffer or close a reader implicitly.

The SDK reserves reply capacity before invoking the handler. Successful resource
responses use `Reply::owned(response, completion)`; the completion commits only
after encoding and retaining the reply buffer succeeds. Encoding/publication
failure or late observed cancellation rolls back unpublished mounts/readers.
Unpublishable pages invalidate and close their cursor, rather than skip entries
silently. Cursor admission stays busy through publication. Cleanup ignores
cancellation; callback error/panic closes further admission for that instance.
`Reply::stateless` refuses Resolved, Opened and Page responses. Authors must retain
their resources in the owned reply's closure and perform all required rollback.

The host releases every successful buffer exactly once through the same instance
and descriptor. A failed call returns a zero buffer. Destroy refuses active calls
or retained buffers; successful destruction retires remaining native resources.
No author function or large destructor runs while registry locks are held.

## Bounded transport

Every payload begins with bytes `46 53 42 52 01 00 00 00` (`FSBR`, version 1).
Integers are fixed-width little-endian, booleans are one byte 0/1, blobs/text have
u64 byte lengths and vectors have u32 counts. UTF-8 is required only for display
text, IDs and diagnostics. Request operation is the C Call's u32 field. Response
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
their slot before OS I/O; failed publication returns those slots.

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

Capabilities independently advertise Browse, Read, Stat, Volumes, Storage,
NativeAddress and VolumeDetails. Unsupported services return Unsupported, never a host-native
fallback. Operational errors are Failed responses with typed error, optional OS
code and bounded diagnostic. C status errors cover framing, admission, unknown
instance/buffer, incompatibility, shutdown and panic.

### Optional volume information (ABI minor 1)

`CAP_VOLUME_DETAILS` is 64 and its manifest spelling is `volume-details`. The
host checks this capability before operations 15 or 16. ABI major, descriptor
layout, operations 1–14 and response tags 1–11 are unchanged. Existing plugins
remain accepted by new hosts; existing SDK adapters inherit Unsupported defaults.
New native manifests retain minimum-host-minor 0 because old hosts reject the
unknown manifest capability before loading. Hosts that recognize the capability
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

No Watch, write, delete, recursive traversal, authentication, executable launch
or UI callbacks are defined by this service. macOS and aarch64 packages have not
been implemented/qualified. Host composition must route listing and native F3
capture through loaded packages and disable those capabilities when a package is
absent; the independent conformance tests alone do not establish host cutover.
