# Read-only filesystem worker protocol 1

This independent public API augments the existing viewer C ABI. It does not change
released viewer API or SDK packages. Archive parser isolation uses an executable
worker: the host starts a trusted installed package through the public worker SDK
startup contract, then exchanges the messages below over its inherited pipes.
No native source paths, shell commands, arbitrary file handles or network services
cross this boundary. The source broker grants access to one captured byte source.

Each frame starts with a little-endian `u32` payload byte length (at most 1 MiB).
The payload starts with little-endian `u16` version 1, a `u8` message tag and
little-endian `u64` request ID. Numeric fields are little endian. Booleans are
exactly 0 or 1. Blobs and UTF-8 strings have `u32` byte lengths; lengths are checked
against both field limits and remaining frame bytes before allocation. Unknown
versions, tags, enum values, trailing bytes and malformed UTF-8 strings fail closed.
Raw names are byte blobs and are never normalized or parsed as source paths.
Explicit host copy-out validates one raw basename before converting it to a native
destination component; display labels never supply filesystem identity.

| Tag | Message | Fields after request ID |
| --- | --- | --- |
| 1 | Open | format `u8`, source length `u64` |
| 2 | Opened | root node `u64` |
| 3 | List | directory ID `u64`, exclusive prior ID `u64` |
| 4 | Entries | count `u32`, nodes, more `bool` |
| 5 | Read | member ID `u64`, offset `u64`, length `u32` |
| 6 | Content | raw blob, EOF `bool` |
| 7 | SourceRead | source offset `u64`, length `u32` |
| 8 | SourceData | raw blob |
| 9 | Failed | error code `u8`, UTF-8 diagnostic |
| 10 | Close | no additional fields |

A node contains `u64` ID and parent ID, raw name blob, UTF-8 display name, kind
`u8`, size-present `bool`, optional size `u64`, and readable `bool`. Names and
display strings each have a 64 KiB resource bound. Listing batches have at most
128 nodes and must fit the aggregate frame budget. Pagination uses monotonically
increasing opaque member IDs, preserving duplicate and case-distinct names.
Root node is 1; member IDs begin at 2. IDs are valid only within the captured mount.

Formats: ZIP=1, TAR=2, GZIP=3, TAR.GZ=4, 7z=5, XZ=6, TAR.XZ=7,
BZIP2=8, TAR.BZ2=9, ZSTD=10, TAR.ZST=11. New formats are selected only when
advertised by the installed worker manifest; older workers retain their advertised
format set and reject unfamiliar enum values. Kinds: directory=1, file=2,
link=3, other=4. Errors: invalid=1, unsupported=2, limit=3, cancelled=4,
source-changed=5, not-found=6, read-only=7, corrupt=8, I/O=9.

Read and source-read lengths are 1 through 256 KiB. Reply blobs cannot exceed the
requested amount. Offset arithmetic must be checked. Diagnostics are at most 4096
UTF-8 bytes. A request has one terminal response; SourceRead/SourceData are nested
broker exchanges using the original request ID. Host request IDs strictly increase
within a worker session. The host rejects stale or mismatched replies.

The first implementation serializes operations per immutable mount. Cancellation
or deadline expiry terminates that worker and invalidates its mount; it does not
pretend that killing a request preserves the session. Source revision validation
is host-owned and runs before and after operations. The format plugin has no
mutation, extraction or launch operation. Additional provider capabilities require
an explicit versioned protocol change.

The helpers are trusted installed executables; installing a plugin grants code
trust. OS containment limits untrusted archive parsing. Frame validation is not a
replacement for the worker SDK's OS restrictions and process resource limits.
# Protocol 2 streaming extension (API 0.3)

Protocol selection is explicit from the trusted installed manifest. The default
read/write helpers retain protocol 1 bytes; `*_with_protocol(..., Protocol::V2)`
is required for version 2. Version 1 node records omit `modified` and decode it
as unavailable. Version 2 appends a timestamp tag after each node's readable flag:
0 unavailable, 1 UTC (signed i64 Unix seconds, u32 nanoseconds), 2 local civil
(i32 year, u8 month/day/hour/minute/second). Numeric fields are little endian.
Civil dates are calendar validated; they carry no inferred timezone.

Version 2 adds message tags after the existing common version/tag/request header:

| Tag | Message | Fields |
| --- | --- | --- |
| 11 | StageBegin | purpose u8 (0 scratch, 1 member followed by entry u64), expected-length bool and optional u64, max-length u64 |
| 12 | StageCreated | stage u64 |
| 13 | StageAppend | stage u64, offset u64, bounded bytes |
| 14 | StageAck | stage u64, next-offset u64 |
| 15 | StageSeal | stage u64, length u64 |
| 16 | StageSealed | stage u64, length u64 |
| 17 | SpoolRead | stage u64, offset u64, length u32 |
| 18 | SpoolData | stage u64, bounded bytes |
| 19 | ContentReady | entry u64, stage u64, offset u64, length u64 |
| 20 | StageProgress | stage u64, completed decoded bytes u64 |
| 21 | StageProgressAck | stage u64, completed decoded bytes u64 |

All request/stage IDs are nonzero, member IDs start at 2, ranges cannot overflow
u64, read/append payloads remain bounded to 256 KiB, append is nonempty. SourceRead
may occur during any active request and may revisit ranges. The host validates
source identity before/after reads and before publishing content. It grants at
most one unfinished stage, validates session ownership and exact append offsets,
and seals only the complete expected length. Scratch stages remain private to
the mount. Member stages belong to the requested member. ContentReady is a
terminal reply naming a checked range of a sealed stage, after all enclosing
format integrity checks have passed. It never names a filesystem path. Before
sealing, no staged bytes may be read. Any protocol, I/O, integrity or cancellation
failure discards unfinished stages and ends the session.

StageProgress reports strictly increasing actual decoded work for the active
unsealed stage, including discarded solid-block siblings required for integrity.
The host validates stage/request ownership and monotonic nonzero progress before
acknowledging it. Only successful nonempty source/spool I/O, appends and accepted
decoded progress renew the idle watchdog; protocol chatter does not. Progress
does not publish bytes or replace final integrity checks.

`max_length` is the declared member length, or u64::MAX for an unknown decoded
stream. It is a consistency guard, not an artificial content-size quota. Disk
exhaustion and platform offset failures are reported by host I/O. The host retains
bounded metadata/frame/process resources independently of total content length.

# Protocol 3 cooperative member cancellation (API 0.4)

Version 3 explicitly opts into request-local member staging cancellation. It uses
version 3 in the frame header, with all protocol-2 message tags, field layouts,
validation, policy-ready containment and resource bounds unchanged. Legacy default
helpers still emit version 1; version 1/2 session-ending cancellation is unchanged.

During a Read request the host may answer a valid StageBegin (member purpose),
StageAppend, StageProgress or StageSeal with Failed(Cancelled), using that request
ID. This reply replaces the operation's normal acknowledgement and means no staging
work was accepted for that message. The worker must unwind preparation, send one
terminal Failed(Cancelled) with the same request ID, and accept the next increasing
request. It must not publish partial content or continue nested exchanges after
cancellation. The host preserves the session only after this exact acknowledgement.

SourceRead/SourceData and SpoolRead/SpoolData retain their ordinary semantics. In
particular, the host does not inject Interrupted source-reader errors, which some
decoders retry. Existing bounded StageAppend and StageProgress calls are cooperative
checkpoints, including discarded solid-block siblings. An already-running OS call
or decoder interval can delay cancellation; this is not a hard latency guarantee.

Before cancellation the host validates the checkpoint's request, direction, stage,
member, offsets, lengths and monotonic progress without writing. Malformed input,
unsolicited/wrong cancellation acknowledgement, source mutation, I/O failure and
watchdog expiry still terminate the session. The watchdog stays armed while the
terminal acknowledgement drains; protocol chatter cannot extend it.

After acknowledged cancellation only request-owned pending/sealed member storage
is dropped. Existing scratch, cache entries and retained read leases remain valid.
A completed ContentReady racing cancellation must pass full source/stage/range
validation but cannot publish bytes or insert a new cache entry. All native cleanup
and actual worker/I/O permit release completes on backend workers.
