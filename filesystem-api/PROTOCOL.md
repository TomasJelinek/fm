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
Raw names are byte blobs and are never normalized or parsed by the host as paths.

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

Formats: ZIP=1, TAR=2, GZIP=3, TAR.GZ=4, 7z=5. Kinds: directory=1, file=2,
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
