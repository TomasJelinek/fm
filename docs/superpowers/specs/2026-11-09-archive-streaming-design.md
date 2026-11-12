# Archive source streaming and private content spooling

Status: resumed at the user's explicit request, 9 November 2026. The implementation
must support generic archive member content on private TEMP disk and index-only
ZIP browsing. The 11 November MEDIA-01 extension adds negotiated cooperative
thumbnail read cancellation, with scoped implementation and installed-worker gates
passed; final combined application acceptance remains separate. No JPEG-specific
decoder changes are part of this work.

The resumed requirements supersede the earlier proposed per-file, total expanded
data and aggregate disk byte quotas below: encoded sources and decoded members
have no artificial size ceiling. Available disk space and platform offset ranges
are authoritative. Bounded RAM, metadata counts, I/O chunks, process isolation,
cancellation and integrity before publication remain required. Numerical byte
quota proposals in historical sections are not implementation requirements.

## Resumed implementation decisions

The current executable contract is filesystem API 0.4, explicit protocol 3.
It preserves protocol-2 staging records and adds request-local cancellation at
validated member staging/progress checkpoints. Legacy protocol-1/2 behavior and
default protocol-1 helper bytes remain unchanged. See the
[current cancellation contract](../../developer/FILESYSTEM_API.md) and
[MEDIA-01 evidence](../../verification/2026-11-10-archive-thumbnails.md). SourceRead is valid during
any operation. Host stages support bounded append/ack, seal, scratch reads and a
terminal ContentReady range; IDs never expose a path or native handle. Sealed
member stages belong to their creating request and selected member. Scratch
stages can supply multiple checked member ranges after complete stream verification.

ZIP and plain TAR read their indexes through captured-source Read+Seek, with
bounded reader lookahead. Selected ZIP/TAR members stream directly to disk. A 7z
request drains the complete required block, retaining only selected content on
disk; monotonic actual decoded-byte progress keeps the idle watchdog alive while
discarding siblings. Compressed TAR and standalone streams use verified scratch
files. The host caches 16 content leases by identity and validates source revision
before/after reads; eviction never destroys an active read's Arc lease.

Worker SDK 0.2 adds an explicit resource-policy startup while retaining the
original bounded startup bytes. Archive workers impose no aggregate CPU deadline;
memory isolation, cancellation and an idle-progress watchdog remain. Existing
ancestor OS resource policies are preserved. No decoder, image extension or JPEG
special case appears in the spool/source interface. Image codecs retain their own
decoding and pixel-allocation behavior; this change removes archive buffers.

Native private files use Linux O_TMPFILE with close-on-exec or Windows private
DACL/delete-on-close handles with pinned ancestors. There is no permanent member
pathname. Native offset ranges and real disk errors are authoritative; unsupported
TEMP filesystem primitives fail explicitly. Final file cleanup stays on backend
workers and waits for outstanding native I/O. These are ephemeral files, not a
crash-recovery or persistent caching product.

## Salamander source inspection, 9 November 2026

The requested local source tree was inspected directly. Relevant references are:

- `D:/experiments/salamander-main/src/plugins/zip/list.cpp:28` opens a read-cached
  ZIP source; `:97` iterates central headers from the central-directory offset.
  This supports separating index browsing from member decompression.
- `src/plugins/zip/extract.cpp:578` connects refill/flush buffers and a sliding
  decompression window. A complete decoded-file RAM buffer is unnecessary.
- `src/fileswn5.cpp:849` acquires a disk-cache name; `:874` extracts a selected
  archive file; `:889` marks it prepared only after successful extraction;
  `:900` releases a failed preparation. `:948` ties the prepared file lifetime to
  the viewer lock. Its provider branch checks `FS_SERVICE_VIEWFILE` before dispatch.
- `src/cache.cpp:140`, `:234`, `:264`, and `:1216` separate preparation, viewer
  ownership, release and readiness. `src/cache.h:36` distinguishes immediate
  deletion from retained-cache lifetime.

These are behavioral precedents, not imported implementation. Salamander's code
is GPL-2.0-or-later; no source or assets were copied. This project retains its
path-free restricted-worker broker, opaque raw member identities, Unicode support,
source-revision checks, and current dependency licenses. Salamander's pathname
normalization/MAX_PATH handling and 100 MB cache quota are not adopted.

## Intent and current boundary

Opening a large archive or previewing one member must not require retaining the
whole container and every decoded member in RAM. Preserve duplicate/raw member
identity, source revision checks, sandbox isolation, bounded resources and the
current rule that no member bytes are returned before the relevant archive
checksums succeed. A tiny preview can still require complete decompression of a
member or solid block. Bounded memory does not imply bounded work independent of
that block's size.

The native filesystem cutover and this work are independent. The archive worker
uses `filesystem-api/midnight-filesystem-api` and the executable worker protocol
in `filesystem-api/PROTOCOL.md`. The native filesystem plugin API/SDK and C ABI
are a different contract; do not add spool methods to their native ABI. Native
plugins already supply a captured, revision-checked `CapturedFile` to
`crates/midnight-filesystem-host/src/session.rs`. Keep that capture and do not
reopen its navigation locator as an alternate source.

Today `main.rs` downloads the complete archive through `SourceRead` into a Vec;
the host and worker both reject sources over 64 MiB. ZIP first access retains a
complete verified member Vec. 7z retains verified sibling Vecs from a solid block.
TAR retains its complete source Vec; compressed TAR first retains a complete
decoded stream. `ArchiveMount::read` has a small result bound, not a small hidden
working-set guarantee. A viewer retains the mount through `capture_archive.rs`.

## Approaches

1. Direct worker access to native source/temp paths would simplify library I/O,
   but changes the sandbox authority and bypasses provider identity. Reject it.
2. Re-decode from the start for every range avoids a retained output buffer but
   makes image decoder seeks and repeated previews potentially quadratic. It
   still must reach EOF to preserve CRC-before-return. Reject as the default.
3. Keep all I/O brokered. Use bounded random source reads and host-owned content
   staging, retaining small content under an aggregate RAM budget and spilling
   larger content to private disk. Recommend this approach. It fits the existing
   sandbox and lets all viewers/copy consumers share the same verified lease.

No new archive codec is needed. Keep all pinned versions and feature sets until
their independent validation shows a concrete need for a dependency change.

## End-to-end ownership and flow

1. Backend captures native content through the installed provider. A mount owns
   its `CapturedFile`, source revision and archive worker permit.
2. Worker receives only length and resource policy. A bounded `Read + Seek`
   adapter sends `SourceRead` messages during Open and later requests; the host
   checks arithmetic, cancellation, source identity and exact reply lengths.
3. ZIP/7z index source metadata through this reader. Plain TAR indexes headers
   with seek and records checked u64 source ranges. None retains full source.
4. Opening a member creates a lazy content lease bound to mount ID, member ID,
   source revision and declared length. It does not eagerly decode every item
   in the viewer's navigation order.
5. First content read either reads a plain TAR range or sends the worker a member
   read. ZIP decoding appends bounded output to one private staging object. 7z
   drains the entire required block, including unsafe siblings, but retains only
   the requested member. All siblings still receive their length/CRC checks.
6. Only after decoder EOF, length checks and applicable member/block CRC checks
   does the worker return `ContentReady`. The host revalidates source identity,
   seals the staged length and atomically publishes its lease. Failure discards
   the staging object, including a valid-looking prefix.
7. The host reads the requested range from the verified lease. Image workers see
   the same image source broker as today. They receive no spool path or handle.
8. Retained content leases keep their backing storage and source identity alive.
   The host keeps a bounded, evictable cache of otherwise unleased members.
   Last lease release schedules backend cleanup; no UI destructor deletes large
   data or waits for a worker.

For compressed TAR, Open streams the entire outer codec to one private scratch
object and checks its final checksum/trailer and all concatenated frames. After
verification it becomes a sealed, mount-owned seekable TAR source; the worker
indexes it through spool reads. Member ranges share that one object and do not
create copies. This preserves current whole-stream verification before listing;
it improves RAM use, not time to first listing. Standalone gzip/xz/bzip2/zstd uses
the same decode path but publishes its single content object instead of TAR
indexing. Plain TAR has header checksums, not a payload CRC; do not claim payload
authenticity or invent an archive checksum for it.

## Public worker protocol 2

Keep protocol 1 bytes and limits unchanged for old installed packages. Record
`Package.protocol` during discovery; accept 1 or 2 explicitly, launch the matching
codec, and retain the 64 MiB limit only on the legacy route. Export a new public
API package version through the existing immutable registry packaging process.
Do not replace published 0.2.0 bytes. Protocol 2 uses the same framed shape with
version 2; unknown versions still fail closed. Packaging manifests advertise 2.

Retain Open/List/Read/Content/SourceRead/SourceData/Failed/Close semantics. Add:

| Message | Direction | Fields and meaning |
| --- | --- | --- |
| StageBegin | worker to host | request, purpose (`Member(entry)` or `Scratch`), optional exact length, maximum length |
| StageCreated | host to worker | request, opaque stage ID |
| StageAppend | worker to host | request, stage ID, expected offset, bytes |
| StageAck | host to worker | request, stage ID, next offset |
| StageSeal | worker to host | request, stage ID, exact verified length; decoder has finished its integrity checks |
| StageSealed | host to worker | request, stage ID, exact length |
| SpoolRead | worker to host | request, sealed stage ID, offset, length |
| SpoolData | host to worker | request, stage ID, bytes |
| ContentReady | worker to host | terminal response to Read: request, member ID, sealed stage ID, checked offset and exact member length |

Each nested message carries the active outer request ID. Stage IDs are generated
by the host and scoped to one worker mount. No basename, user path, URI or file
descriptor is accepted. StageAppend is sequential, nonempty and at most 256 KiB;
offset must equal bytes already written. All reads are at most 256 KiB. Unknown
IDs, cross-request unfinished stages, append-after-seal, double seal, changed
lengths or unexpected messages terminate the mount. A stage is either private
scratch owned by the mount or content for the exact currently requested member;
the worker cannot publish content under an arbitrary sibling ID. One operation
may own one unfinished staging object. No worker-side write method exists other
than this restricted append capability.

Seal makes scratch readable to its originating parser, but does not expose member
content to viewers. Only a valid terminal response followed by host source
revalidation publishes content. Failed Open/Read rolls back all stages created
by that operation. Compressed TAR's verified scratch survives successful Open.
The host registry bounds object count and declared/actual byte totals separately.

The old broker condition `broker_bytes <= source.length()` is incompatible with
random rereads and small archives: replace it in protocol 2 with explicit
operation/read-count budgets. All additions and multiplications are checked.
Use a 256 KiB read-ahead cache per worker source, not per archive member. Limit
Open metadata reads separately from sequential decompression reads; resets must
not allow infinite work because mount CPU time and session totals remain capped.

## Host interfaces and private storage

Add a focused host content module rather than placing disk policy in the UI or
native filesystem plugin. Proposed core interface:

```rust
pub trait ArchiveContent: Send + Sync + std::fmt::Debug {
    fn length(&self) -> u64;
    fn validate(&self, cancel: &Cancellation) -> Result<(), FsError>;
    fn read_at(&self, offset: u64, length: u32, cancel: &Cancellation)
        -> Result<Vec<u8>, FsError>;
}
// ArchiveMount addition; lease acquisition is cheap and decoding remains lazy.
fn open_content(&self, entry: u64, cancel: &Cancellation)
    -> Result<Arc<dyn ArchiveContent>, FsError>;
```

Keep existing `ArchiveMount::read` as a compatibility convenience while migrating
viewer and copy consumers to an explicitly retained lease. Content cache keys use
mount identity/member ID, never display paths or member basenames. Cache entries
hold Weak leases plus a charged bounded hot-cache reference. Never evict an active
lease, and never let an evicted cache entry invalidate a reader's retained data.
Avoid Arc cycles: content holds an Arc to mount/source validation state; mount
cache retains backing objects without content objects that own the mount.

Extend `midnight-worker-platform` with a new private anonymous spool primitive;
reuse audited ancestor pinning/permission checks from `store_windows.rs` and
`store_linux.rs`, not the thumbnail cache's whole-Vec read/write API. Windows
creates an exclusive ordinary file with delete-on-close and private DACL under a
pinned private directory; Linux uses a verified private directory, exclusive
no-follow creation and immediate unlink while retaining the descriptor. Test
exact crash cleanup semantics on each platform. No archive-controlled path is
ever extracted. The platform primitive exposes backend-only append/read_at/length
and final close; host policy owns quotas and publication. Data are ephemeral,
not a durable cross-process cache. A crash must not require scanning arbitrary
TEMP paths; if a platform cannot unlink immediately, retain a narrowly scoped
owned-file recovery mechanism with process ownership checks.

Quota proposals for first qualification: 1 MiB per small in-memory member,
16 MiB total retained/staged RAM, 8 GiB aggregate retained/staged spool disk,
8 GiB maximum individual staged content/expanded stream, 64 MiB decoder
dictionary/window, 100,000 indexed nodes and 8 MiB raw/display names. Keep ratio
200 as a work/output restriction, separate from memory. Logical total indexed
member bytes must no longer share the old 128 MiB retained-RAM limit. Offset and
source sizes are u64 and seekable sources are not copied just to enforce a size
ceiling. Aggregate disk reservations include in-progress writes and every live
lease; reserve before writing and release only after the file really closes.
Unknown expanded lengths reserve incrementally, aborting before quota overrun.
Free-space checks are advisory; ENOSPC/write/flush failure remains authoritative.
Never fall back to an unbounded Vec when disk is unavailable.

Two existing 512 MiB archive worker limits remain a hard 1 GiB aggregate ceiling
for archive processes; distinguish that ceiling from the much smaller normal
working set. The host's staging/broker/index budgets are additive. Existing image
worker/decoded-pixel admission is an additional budget; do not report only one
process when measuring peak viewing memory. If the product requires one lower
cross-kind ceiling, admission must reserve the existing per-process bounds in a
shared service before spawning. Changing process limits requires measurements.

The present 15 s cumulative archive CPU and 20 s operation wall deadlines may
reject valid large solid blocks. Keep explicit finite CPU/work limits, but qualify
and revise them with actual large fixtures rather than claiming arbitrary sizes
are supported after removing the 64 MiB input check.

## Existing pinned library feasibility and risks

| Library | Usable streaming operation | Additional restriction |
| --- | --- | --- |
| zip 8.6.0 | `ZipArchive<R: Read + Seek>` and `by_index` reader | Read selected member to EOF for CRC, then publish. Keep Stored/Deflated feature scope. |
| sevenz-rust2 0.23.0 | `Archive::read` with seek; `BlockDecoder::for_each_entries` supplies a reader per member | Preserve current compact stream-ordinal mapping. Drain every substream of the block; discard unrequested output. |
| tar 0.4.46 | `entries_with_seek`, `raw_file_position` | Retain u64 ranges into source/spool; bound seeks including final padding; keep sparse/special/link entries unreadable. |
| flate2 1.1.10 / bzip2 0.6.1 | MultiGzDecoder / MultiBzDecoder over `Read` | Drain concatenated streams and terminal integrity checks. |
| lzma-rust2 0.21.0 | XzReader with memory limit | Keep dictionary budget separate from entry length. |
| ruzstd 0.9.1 | StreamingDecoder over `Read`, FrameDecoder window limit | Rework slice-based remaining-input handling to buffered input; preserve skippable/concatenated frames and explicit checksum comparison. |

Lifting the source cap exposes metadata allocations hidden by the old bound.
`ZipArchive::new` allocates its declared directory before the caller checks
`zip.len()`; its Config has no metadata budget. `sevenz::Archive::read` allocates
the declared next header and buffers the expanded encoded header before application
node/name checks. Its public BlockDecoder does not expose an application memory
budget; LZMA properties must be checked before decoder construction. These are
explicit qualification gates. Keep the process limit as containment, and report
resource termination clearly. Do not describe a post-parse check as protection
against pre-parse allocation. For graceful rejection, add verified bounded-header
preflight where adequate (ZIP/7z top-level sizes) and obtain narrowly scoped
upstream/pinned-source limit support for encoded 7z header and decoder allocations.
Do not write a second general-purpose archive parser in the host. Any patched
dependency version/source must receive the license, checksum, notices and isolated
package review required by `docs/DEPENDENCY_POLICY.md` before shipment.

The raster JPEG issue is separate. `image 0.25.10`'s JpegDecoder first calls
`read_to_end`; the already pinned zune-jpeg 0.5.15 accepts `ZByteReaderTrait`, and
zune-core 0.5.3 provides buffered std I/O support. A plugin-local adapter using
those exact transitive dependencies directly can remove this encoded copy.
Preserve image-rs output-color selection, permissive JPEG setting, EXIF orientation,
ICC processing and pixel limits; qualify progressive JPEG and broker I/O failures.
Full decoded RGBA and progressive coefficient memory remain. Both crates offer
MIT/Apache-2.0/Zlib branches; inspect their actual pinned license files and update
direct-dependency inventories/notices when promoting them to direct dependencies.
The public [pinned JPEG API](https://docs.rs/zune-jpeg/0.5.15/zune_jpeg/struct.JpegDecoder.html)
documents its generic reader and decode-into interfaces. PSD remains unsupported;
adding a PSD codec, layer loading or embedded-preview selection is a separate task.

Salamander provides useful lifecycle precedent, not reusable sandbox design:
`D:/experiments/salamander-main/src/cache.h` defines GetName, NamePrepared,
AssignName and ReleaseName; `src/zip.cpp` connects prepared temp files and viewer
locks. Its viewer receives a temp filename. Reuse the idea of private preparation,
publication and reader lifetime, while preserving Midnight's path-free workers
and Unicode/raw identity contracts. Do not copy its pathname or MAX_PATH limits.

## Implementation sequence and file ownership

1. Protocol/API tests and versioned packaging: worker API `src/lib.rs`,
   `src/wire.rs`, `tests/protocol.rs`, PROTOCOL.md; worker-package export tooling,
   archive build scripts/manifests and host discovery tests. Prove v1 remains
   byte-compatible and v2 rejects malformed/spoofed staging messages.
2. Platform private spool primitive and tests: new `spool.rs`, `spool_windows.rs`,
   `spool_linux.rs` under `crates/midnight-worker-platform/src`, plus separate
   Windows/Linux ownership, reparse/symlink, close and process-crash tests.
   This can proceed independently from the protocol work.
3. Host staging/lease policy: new `content.rs`, `spool_budget.rs`, `broker.rs` and
   focused tests under `midnight-filesystem-host`; keep process supervision in
   session.rs. Add the core ArchiveContent contract and lazy consumer leases in
   `midnight-viewer-engine/src/capture_archive.rs` and
   `midnight-engine/src/archive_transfer.rs`. Depend on steps 1 and 2.
4. Worker broker source and ZIP/plain TAR conversion: plugin-local `broker.rs`,
   `content.rs` and `source.rs`, main.rs/lib.rs/formats.rs/tar.rs/tree.rs. Keep the
   plugin independently buildable against public exports only. Replace library
   test Vec ownership with Cursor-based fixture sources through the same trait.
5. 7z and compressed-stream conversion: sevenzip.rs/streams.rs and separate
   fixture tests. Preserve solid-block checksum atomicity and unsafe-sibling
   draining. Metadata/dictionary qualification is part of this step, not deferred
   until after source limits are raised in production.
6. Host sandbox acceptance, large-source/RAM/disk measurement, docs and release
   packaging. Update IMAGE_VIEWER_API.md, ARCHIVE_FORMATS.md and archive developer
   docs with measured behavior and any remaining per-format limits.

The JPEG adapter can be implemented/tested independently inside
`plugins/midnight-viewer-image` without waiting for any archive protocol step.
Independent native filesystem cutover and tree-controller work should not edit
the files above concurrently with these steps.

## Required observable tests

- Real ZIP/TAR/7z sources above 64 MiB and offset fixtures above 4 GiB; mount/index
  reads do not scan or retain all seekable payload bytes. Include ZIP64, long raw
  names, duplicates and interleaved empty/no-stream 7z nodes.
- First one-byte read of a compressed member whose tail CRC is corrupt returns
  no content; a corrupt later sibling in the same solid block also publishes
  nothing. A corrupt unrelated non-solid block does not prevent an intact member.
- Repeated far-apart reads use one verified backing object; retained leases
  survive cache eviction and panel navigation, and source replacement invalidates
  them before new bytes/pixels are published. A cached spool is not authority to
  bypass live source validation.
- Cancellation during source I/O, append, final CRC, seal and cleanup; worker
  crash and host process crash. Cancellation cannot free a worker/quota permit
  before its OS work/file close actually completes. Connect viewer-close
  cancellation to the archive request; the current default Cancellation in
  Member::read_at is insufficient for long spooling work.
- Low disk, short/failed writes, flush failure, RAM threshold crossing and quota
  races across two mounts/readers. Assert aggregate charged bytes and no partial
  publication or resource leak; a failure cannot silently switch to RAM.
- Malicious stage IDs, malformed frames, wrong active request, overflowed ranges,
  empty append, extra output, undersized output and attempts to read unsealed
  data fail closed. Sandbox probe still cannot open arbitrary files or write a
  supplied path, and the worker still starts before receiving untrusted data.
- Oversized compressed 7z headers/dictionaries and ZIP directory metadata are
  rejected or contained under the documented process budget, never allowed to
  consume host memory proportional to attacker-declared source length.
- Concatenated gzip/bzip2/xz/zstd, truncated trailer, bad last-frame checksum,
  skippable zstd frames and TAR final padding maintain current rejection rules.
- Qualify total peak process memory, source bytes read, spool bytes, first-preview
  latency and repeat-preview latency across host + archive + image worker for a
  large JPEG, solid archive and compressed TAR. Small images stay in the shared
  RAM budget; large content grows disk usage without proportional RAM growth.

Test fixtures, measurement output and build artifacts stay under `_build/`;
toolchains and caches stay under `.tools/`. Run independent plugin/API tests from
their exported package configurations plus host integration on Windows and Linux.
Do not mark ARCH-01 complete while any format still loads the full archive/member
into application-owned Vecs or while lifecycle/aggregate quota tests are missing.
