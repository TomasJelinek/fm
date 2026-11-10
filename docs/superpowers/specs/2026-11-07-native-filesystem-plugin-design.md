# Native filesystem Browse/Read ABI design

2026-11-07; follows clean checkpoint 8d0741f. Root approved this synchronous C ABI
contract. Existing archive/viewer contracts remain compatible.

## Choice and scope

Use an independent `midnight_filesystem_entry_v1` C entry point and std-only
public API/author SDK. The host calls it on bounded backend workers. Native
packages are trusted in-process libraries; there is no sandbox claim. Archive
worker protocol 1 and `midnight_plugin_entry_v1` viewer ABI/SDK stay unchanged.

Alternatives considered: archive-style workers require new native path grants
and containment semantics unrelated to captured archive parsing; reusing the
viewer asynchronous table requires generalizing its viewer-only runtime and
service broker. Neither is necessary for blocking filesystem operations already
owned by host workers. A separate synchronous table is smaller and keeps existing
clients compatible. Root selected this approach.

Deliver independent Windows/Linux packages and host-connected listing and
selected-file F3. Pure registry/mock tests alone do not close this slice. The
host owns composition, UI, selected-entry/view revisions, discovery/trust and
viewer capture injection. This implementer owns API/SDK/native packages. Existing
mutation/transfer/desktop services remain explicitly transitional; Browse/Read
does not grant mutation authority or immutable object identity.

## Public projects and ownership

- Add `filesystem-api/midnight-filesystem-plugin-api` and
  `filesystem-api/midnight-filesystem-plugin-sdk` to that independent workspace.
  Existing `midnight-filesystem-api` 0.2.0 archive crate stays unchanged.
- API: `abi.rs`, `model.rs`, `wire/{encode,decode}.rs`, public C header and tests.
  `#![forbid(unsafe_code)]`; C-compatible declarations do not dereference pointers.
- SDK: safe `Filesystem` trait, `CallContext`, owned `Request`/`Response`, bounded
  instance/buffer registries and `export_filesystem!`; audited `ffi.rs` is the
  only narrow unsafe allowance. No application/core/engine/UI dependencies.
- Separate `plugins/midnight-filesystem-native-windows` and
  `plugins/midnight-filesystem-native-linux` Cargo workspaces, each depending only
  on public filesystem API/SDK plus its already-used OS binding crate. Per-OS
  implementation and native tests stay separate.
- No root Cargo or host/core/engine/app/UI modifications by this implementer.
  Root adds host dependency lines and trusted loader/composition integration.

## Descriptor and ABI

ABI major 1, minor 0; payload schema 1 is independent of archive protocol version.
All callback table fields are Option<unsafe extern "C" fn> so an all-zero
descriptor is valid. All types are `repr(C)` fixed integers, pointers and function pointers; no Rust
Vec/String/trait object/Result, allocator or exception crosses the ABI. Supported
targets initially Windows x86-64 and Linux x86-64/aarch64; actual aarch64 testing
must be reported separately. Calling convention is `extern "C"`.

```text
Slice       { data: const u8*, len: u64 }
Buffer      { id: u64, bytes: Slice }
Header      { size:u32, major:u16, minor:u16, kind:u32,
              minimum_host_minor:u16, reserved:u16 }
Cancellation{ context:u64, cancelled: Option<unsafe extern C fn(u64)->u32> }
Call        { size:u32, operation:u32, payload:Slice, cancel:Cancellation }
Config      { size:u32, reserved:u32, request_bytes:u64, reply_bytes:u64,
              retained_buffer_bytes:u64, max_buffers:u32, max_calls:u32 }
Api         { header:Header, capabilities:u64, id:Slice, version:Slice,
              create:fn(Config*,instance:u64*)->Status,
              call:fn(instance:u64,Call*,Buffer*)->Status,
              release_buffer:fn(instance:u64,buffer:u64)->Status,
              destroy:fn(instance:u64)->Status }
Entry       fn(Api*, caller_capacity:u32)->Status
```

Header kind=2 (filesystem), reserved=0. Descriptor metadata is immutable UTF-8,
id <=128 bytes, version <=64 bytes, lifetime pinned to the loaded library.
Capacity/header validation precedes any tail access or write. A larger future
tail is ignored; unknown required ABI major/minimum minor is incompatible.
The SDK publishes C sizeof/offsetof assertions and Rust layout tests.

Status: OK=0, Invalid=1, Incompatible=2, Busy=3, Unsupported=4,
ResourceLimit=5, UnknownHandle=6, ShuttingDown=7, Panicked=8.
`call` returns OK with a typed Response (including filesystem Failed responses).
Admission/allocation/ABI failure returns non-OK and an empty output Buffer.
No asynchronous event channel: the call's return identifies its response, so
request IDs/order caches are unnecessary and cannot reject reordered workers.

Each successful nonempty reply has a nonzero instance-owned buffer ID. Only
`release_buffer` on the same instance/descriptor releases it. Wrong-owner or
double release cannot free another buffer. Buffers remain valid until release;
the host copies/decodes before releasing and retains the DLL/instance lease.
No pointer is retained from Call, Config or cancellation after that call returns.
The SDK catches unwinding at every FFI entry and fails closed after a handler
panic. A bad native pointer from trusted code is not made safe by validation.

Calls may overlap on one instance. Registry locks never cover OS I/O or author
callbacks. Concurrent use of one listing cursor returns Busy, not reordered
pages. Reads use positioned I/O, not a shared seek cursor. Destroy refuses while
calls/buffers are owned; no early resource release on host async cancellation.

## Portable typed model and strict wire

Std-only binary encoding, little endian, no new codec dependency. Every payload
begins `FSBR`, u16 schema=1, u16 reserved=0. Operation chooses the request type;
response has a checked u16 variant. Integers have fixed widths; bool is 0/1;
blobs/UTF-8 strings carry u64 byte lengths checked against remaining bytes and
Config budget before allocation. Unknown tags, enums, trailing bytes, invalid
UTF-8 captions, integer overflow and malformed optional values fail closed.

Opaque handle values are nonzero u64 and never reused during the loaded SDK
module's lifetime. SDK-generated instance/buffer IDs use one checked module-wide
allocator. Native packages also use nonreused IDs with owner/mount/type records.
Every operation validates the expected owner, mount and handle type before I/O.
The host additionally scopes handles by loaded descriptor, registration instance
and mount generation. A plugin number alone is never a globally meaningful ID.

`Address { encoding, bytes }`: encoding Opaque=0, WindowsUtf16Le=1, UnixBytes=2.
Opaque bytes are provider-owned. Native encodings are NUL-free absolute paths;
Windows UTF-16 can contain unpaired surrogates, Unix bytes need not be UTF-8.
No MAX_PATH or fixed path buffer and no display-to-path conversion. Variable
address storage is subject to explicit aggregate allocation budgets, reported
as ResourceLimit rather than truncation or a semantic application path ceiling.
OS limitations remain visible. Native supplied addresses are provenance for the
transitional native adapter, not a write grant or object-identity proof.

`Node { id, address, name_encoding, raw_name, display, kind, metadata }`:
name encoding Opaque=0, Utf8=1, WindowsUtf16Le=2, UnixBytes=3; display is separate
UTF-8. Kinds Directory/File/DirectoryLink/Symlink/Other. Metadata has optional
regular-file size, modified `{unix_seconds:i64,nanos:u32}`, native attributes,
POSIX mode, and compression Unknown/Uncompressed/Compressed. Missing stays
missing. Locale formatting stays with host locale services; it does not reopen
the file. Host never guesses separators, decodes Opaque bytes or joins captions.

Node identity is a logical path/kind in this mount, not a pinned inode. If kind
changes, old token access fails SourceChanged and a fresh listing allocates a
new token. File replacement before OpenRead is still live path capture; only
the resulting content lease is revision validated. Do not claim a snapshot of
the entire listing or filesystem.

### Exact payload field order and public helpers

Public enum `Operation` uses the numbers in the table below; public enums
`Request` and `Response` own decoded data. API helpers are
`encode_request(&Request, maximum:usize) -> Result<Vec<u8>,WireError>`,
`decode_request(operation:u32,&[u8],maximum:usize) -> Result<Request,WireError>`,
`encode_response(&Response,maximum:usize)` and
`decode_response(&[u8],maximum:usize)` returning the analogous Results. No FFI
pointer traversal is performed by the API crate.

After the eight-byte prefix, request fields follow table order. Handle/mount/
node/cursor/ordinal/length-size/offset values are u64, except read length and page
entry limit u32. Composite field order:

```text
Address: encoding:u8, bytes:blob
Metadata: present:u16 (only bits0..3), [size:u64],
          [modified_seconds:i64,modified_nanos:u32],
          [native_attributes:u32], [unix_mode:u32], compression:u8
Node: id:u64, address:Address, name_encoding:u8, raw_name:blob,
      display:utf8, kind:u8, metadata:Metadata
Volume: id:utf8, display:utf8, root:Address
```

Metadata optional bits0=size,1=modified,2=native attributes,3=Unix mode, with
present values encoded in that order. Compression0=Unknown,1=Uncompressed,
2=Compressed. Kind1=Directory,2=File,3=DirectoryLink,4=Symlink,5=Other.
All blobs/UTF-8 use u64 lengths; vector counts use u32. Nanos<1,000,000,000.
Every response has its u16 variant immediately after the prefix:

```text
1 Unit: no fields
2 Resolved: mount:u64,node:Node
3 Node: Node
4 Page: cursor:u64,page:u64,done:bool,count:u32,Node[count]
5 Parent: present:bool,[Node]
6 Opened: reader:u64,length:u64,revision:blob
7 Content: bytes:blob,eof:bool
8 Volumes: count:u32,Volume[count]
9 StorageKind: kind:u8 (0 unknown,1 SSD,2 rotational,3 network)
10 SpaceInfo: available:u64,total:u64
11 Failed: code:u16,has_os_code:bool,[os_code:i32],message:utf8
```

Failure codes1=Invalid,2=Unsupported,3=NotFound,4=PermissionDenied,
5=SourceChanged,6=Cancelled,7=ResourceLimit,8=Io,9=StaleReference,10=Busy.
Counts and lengths are checked against remaining payload/budget before allocating.
The optional-native-error field preserves a native error for diagnostics without
making an OS error number the cross-platform error contract.

SDK safe author trait:
`Filesystem::create(Config)->Result<Self,Failure>` and
`Filesystem::handle(&self,Request,&CallContext)->Result<Reply,Failure>`.
`export_filesystem!(Type,id_literal,version_literal,capability_bits)` implements
the fixed entry/table. `CallContext::is_cancelled()` and `config()` expose only
borrowed cancellation and validated limits. The SDK owns wire allocations and
FFI panic/owner/buffer guards; the author owns bounded filesystem handles/state.
Resource replies use `Reply::owned` with a publication completion: success commits
only after encoding/buffer retention, while failed encoding/publication or late
observed cancellation rolls back undisclosed resources. Stateless replies cannot
contain Resolved/Opened/Page. An unpublishable page invalidates its cursor;
serialization remains busy through publication. Cleanup ignores cancellation and
failure/panic closes further instance admission. This SDK-only owner type does
not alter the wire or C ABI. The concrete contract is also documented in
[the public protocol](../../../filesystem-api/NATIVE_PLUGIN_PROTOCOL.md).

## Operations (exact v1 operation numbers)

| Op | Request | Successful Response |
| --- | --- | --- |
| 1 Resolve | Address | Resolved { mount, node:Node } |
| 2 Stat | mount,node | Node (same logical kind or SourceChanged) |
| 3 ListBegin | mount,directory,max_entries | Page { cursor,page:0,entries,done } |
| 4 ListNext | cursor,expected_page,max_entries | Page with exact expected ordinal |
| 5 CloseList | cursor | Unit |
| 6 Parent | mount,node | optional Node; root has None |
| 7 OpenRead | mount,node | Opened { reader,length,revision:blob } |
| 8 ValidateRead | reader | Unit, after validating its captured revision |
| 9 ReadAt | reader,offset:u64,length:u32 | Content { bytes,eof } |
| 10 CloseRead | reader | Unit |
| 11 CloseMount | mount | Unit, or Busy while listings/readers retained |
| 12 Volumes | no fields | Volume list { id,display,root:Address } |
| 13 StorageKind | Address | Unknown/LocalSsd/Rotational/Network |
| 14 SpaceInfo | Address | available:u64,total:u64 |

Capabilities: Browse=1, Read=2, Stat=4, Volumes=8, Storage=16,
NativeAddress=32. Browse+Read+NativeAddress required for this native package slice;
Stat/Volumes/Storage are declared optional service families, Unsupported when
absent. Our native packages implement all, but StorageKind can conservatively
return Unknown. Unsupported does not mean call host LocalFS. Native entry_metadata
can Resolve an uncached raw file address then Stat; no host-native reopen is used.

Initial transport defaults: request budget32MiB, reply budget1MiB, retained reply
buffers16MiB, max64 buffers, max8 active calls per instance; configurable downward.
Listings <=128 entries/page and fit reply budget; reads1..256KiB, diagnostics
<=4096 UTF-8 bytes and revisions1..4096 bytes. Native node/cache memory and OS
handles have separate explicit budgets; proposed initial native cache256MiB,
1,000,000 node records,32 cursors,64 readers. These are resource limits, not exact
RSS claims. Budget exhaustion returns an error, never a truncated successful
listing. Volumes has a bounded complete reply; exceeding it is ResourceLimit.

Listing starts an OS iterator and produces the first bounded batch, without
collecting/sorting the whole directory. Root owns collation/order on backend
snapshots. A cursor is tied to one mount and listing lifetime; page ordinals do
not wrap. Names cannot retarget IDs or alias a parent. Native listing is live,
not an atomic snapshot: races may skip vanished entries; errors are explicit.
Cancellation/failure invalidates that cursor, so no partially advanced retry is
presented as a complete result. CloseList/CloseRead/CloseMount cleanup remains
available even when the cancellation callback is true.

## Read and cancellation guarantees

OpenRead requires an actual regular non-link/non-reparse object, opens a retained
native handle and captures identity, length, modification and change stamps.
Windows includes volume/file ID + write/change time; Linux dev/inode + mtime/ctime
including nanoseconds. No special-file blocking, automatic extraction, whole-file
materialization or native-file length ceiling. Empty files are valid.

ValidateRead checks the captured object/path revision; ReadAt validates before
and after positioned reads and returns SourceChanged on mismatch. It returns
exactly min(requested,length-offset) bytes; zero progress/short non-EOF is failure,
offset overflow or offset>length is invalid, offset==length returns empty EOF.
Independent readers can interleave. The opaque revision stays constant for a
live lease; this is LiveValidated, not an immutable snapshot against external
writes that a filesystem fails to report.

Per-call cancellation is a borrowed callback, checked before/after I/O and
between entries/read chunks. It must be thread-safe, nonblocking and never call
back into this plugin. An OS call can remain blocked; the host retains its worker
permit, instance, library and buffers until it actually returns. No TerminateThread,
detached cleanup, forced DLL unload, private plugin pools or native fallback.

Closing a pane releases its owner while a viewer can keep a content lease/mount.
Package revocation rejects new work, cancels requests and makes source validate/
read unavailable; retained close/release cleanup still runs. Root's host wrapper
checks revocation around calls and discards stale replies. Neither removal nor
re-registration may redirect an old source to replacement code/handles.

## Manifest and host integration

```toml
schema_version = 1
id = "native.windows" # native.linux on Linux
kind = "filesystem-native"
version = "0.1.0"
entry_library = "midnight_filesystem_native_windows.dll"
target = "x86_64-pc-windows-msvc"
abi_major = 1
minimum_host_minor = 0
protocol = 1
capabilities = ["browse", "read", "stat", "volumes", "storage", "native-address"]
address_encoding = "windows-utf16le" # unix-bytes on Linux
```

Install in `filesystems/<id>/`. Root discovery distinguishes filesystem-native
from filesystem-worker and reuses existing package/ancestor trust checks, fixed
validated library paths, restricted dependency search and manifest/descriptor
identity checks. No CWD/PATH/environment plugin discovery. Optional package absent
means native browsing/F3 unavailable, with no static native fallback. Bootstrap
preferences/plugin trust/fonts/locale remain host services.

Root adds a host session wrapper implementing current FileSystem through these
methods, plus a provider ContentSource for the injected viewer capture seam.
Every read source retains the original loaded instance/read handle and revision;
host CapturedFile::from_source is used, never capture(native_path) fallback.
Host adapter decodes only target-matching declared native provenance. Existing
archive/viewer routes and their protocols remain intact.

## Dependencies, source reference and evidence gates

API/SDK std-only. Native Windows may use already pinned windows-sys=0.61.2 with
Foundation/Storage_FileSystem (and LibraryLoader only in standalone loader tests);
Linux libc=0.2.191. Before manifest edits inspect those pinned source licenses and
feature definitions and update inventory/notices for new package attribution.
No new third-party library, private app dependency or test-only paid runtime.

Salamander `src/plugins/shared/spl_fs.h` was inspected directly: separate OpenFS/
CloseFS/ReleaseObject, provider-owned ChangePath/ListCurrentPath/Parent semantics,
and context-specific GetSupportedServices inform lifecycle/capabilities. Do not
copy its ANSI/MAX_PATH buffers, shortened-path fallback, UI callbacks or source.
Its native disk panels are separate from plugin FS; it is behavior inspiration,
not evidence that native I/O is already universally plugin based.

RED/GREEN gates: malformed codec/layout/capacity; wrong-owner buffers/handles;
destroy with active call/buffer; panic/cancellation and full quota; first page
before entire directory; raw Windows surrogate/Unix invalid bytes; >260 native
paths and Linux handle-relative >4096 fixture where feasible; symlink/FIFO refusal;
independent reads, source edit/replacement and exact EOF; actual dynamic library
load/descriptor/call on both OSes. Native projects build/test without host workspace.
Root end-to-end gate: installed native package lists real fixtures, selection/F3
reads exact chosen bytes through ContentSource, package absent/revoked disables
both routes and host-native spy proves no fallback. Existing archive/viewer tests
stay passing. All generated/test output under `_build/goal/native-plugin`.
