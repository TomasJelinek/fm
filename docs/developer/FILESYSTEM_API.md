# Read-only filesystem worker API

`midnight-filesystem-api` version `0.4.0` implements protocols **1, 2 and 3** for independent
archive filesystem executables. It supports opening one captured byte source,
listing directories and reading members. The application integrates this with
pane navigation, selection, sorting, filtering, tabs, F3 viewing and verified F5
copy-out. This is a
separate public workspace from the viewer C ABI: the viewer's `KIND_FILESYSTEM`
constant remains reserved, and released viewer API/SDK `0.1.0` packages are
unchanged. Native libraries and Network Neighborhood use the separate
[filesystem plugin ABI](NATIVE_FILESYSTEM_API.md) and
[network contract](NETWORK_FILESYSTEM.md), not methods exported by this archive
worker package. Filesystem mutation remains outside this read-only worker API.

The authoritative public definitions are [lib.rs](../../filesystem-api/midnight-filesystem-api/src/lib.rs)
and [wire.rs](../../filesystem-api/midnight-filesystem-api/src/wire.rs). The
[wire reference](../../filesystem-api/PROTOCOL.md) specifies every tag and field.
It is packaged in the released 0.4.0 crate and therefore not edited in place: its
statement that cancellation terminates the worker predates the current host, whose
behavior is described under [errors, cancellation and resource bounds](#errors-cancellation-and-resource-bounds).
Workers must still tolerate termination at any time.
The independent [archive plugin](../../plugins/midnight-fs-archive/README.md) is a
working implementation. Its parser owns no native file authority and imports no
application crates.

## Package and installation

A plugin is a separate Cargo project with its own lockfile, parser dependencies
and tests. Its public dependencies are pinned registry packages:

```toml
[dependencies]
midnight-filesystem-api = { version = "=0.4.0", registry = "midnight" }
midnight-worker-sdk = { version = "=0.2.0", registry = "midnight" }
```

Use the distributed offline registry configuration for those dependencies; do not
replace them with paths into the application, private host helpers or `[patch]`
overrides. The worker SDK provides the startup containment handshake; it does not
provide an archive parser. See [worker SDK](WORKER_SDK.md) for that separate
contract. All third-party dependencies and distribution notices must satisfy the
[dependency policy](../DEPENDENCY_POLICY.md).

Install a direct package below `filesystems/<package>/` next to the **actual
application executable**. The current working directory is irrelevant. For
Windows x64, a package manifest can be:

```toml
schema_version = 1
id = "example.archive"
kind = "filesystem-worker"
version = "0.1.0"
entry_executable = "example-archive.exe"
target = "x86_64-pc-windows-msvc"
protocol = 3
stable_entry_ids = true
formats = ["zip", "tar", "gzip", "tar-gzip", "7z", "xz", "tar-xz", "bzip2", "tar-bzip2", "zstd", "tar-zstd"]
```

Linux uses its exact host target and executable basename without `.exe`. The
entry must be a single basename: no separators, colon, `.` or `..`. IDs are
1–128 ASCII letters, digits, dots, hyphens or underscores. Unknown manifest fields,
formats, schema/protocol versions or mismatched targets are refused. Manifests are at most
16 KiB, discovery examines at most 64 packages, and duplicate IDs disable every
package with that ID. The manifest version is nonempty metadata of at most 128 bytes; the
wire protocol field determines compatibility. Restart after installation changes.

An optional `name` provides a short filesystem title: at most 128 UTF-8 bytes,
nonblank and without controls. Its absence preserves older manifests. This is
presentation metadata and does not change provider identity, protocol, formats
or member IDs. The application includes validated package metadata in its
catalog-only `FilesystemPluginsListed` event; listing does not start a worker.
The read-only Plugins overview has no runtime filesystem enable/disable setting.

`stable_entry_ids` is an optional boolean, defaulting to false for older packages.
Only advertise true when identical source bytes, format and implementation version
assign identical opaque IDs to every member across reopenings. Duplicate filenames
must still have distinct IDs. Change the implementation version whenever that
mapping changes. This grants persistent thumbnail-cache capability; false/missing
does not disable browsing. The host combines provider/version, raw container,
captured source revision, format and member identity, revalidates the source on
cache hits, and never uses a display name or transient mount ID as the persistent
key. See the [archive thumbnail contract](../superpowers/specs/2026-11-05-panel-views-thumbnails-design.md#archive-content-integration-10-november-2026).

The host validates the installation, ancestors, package directory, manifest,
executable and every other package object (for example a dependent library the
OS loader could resolve before worker containment) using the native plugin
ownership/ACL policy, at discovery and again before each mount launches the
worker. Redirections, nonregular objects and unsafe writable package objects or
ancestors are refused. Installing an executable grants it code trust; containment
limits the untrusted documents it parses. A broadly writable build directory may
be unsuitable as a trusted installation. Do not loosen source-tree permissions to
make discovery accept it. When several trusted workers advertise one format, the
default is the package with the lowest ID, independent of directory order.

No package means no archive browsing capability. Native archive files retain
normal OS activation when the catalog has no providers. With providers installed,
the current host selects ZIP containers, TAR, GZIP, XZ, BZIP2, Zstandard, their
compressed TAR variants or 7z by extension and the manifest's format list.
Options → Archives assigns custom literal suffixes to each supported decoder,
alongside the built-in extension checkboxes. This policy belongs to the host;
workers receive the typed `Format` and captured bytes through their negotiated protocol,
never a UI rule, filename script or native source pathname. A custom assignment
cannot enable a decoder absent from the trusted manifest. There is no worker-owned
extension registration or provider-priority negotiation in protocol 1. Saved
archive addresses retain their decoder independently of subsequent suffix edits.

## Lifecycle and source authority

1. The host captures a regular native source with identity, length and revision,
   validates the installed executable, and starts a restricted worker.
2. The executable calls `midnight_worker_sdk::enter` on stdin/stdout before reading
   any archive request or bytes. Failed containment ends the worker. The host
   waits for the SDK readiness response under a startup deadline.
3. The host sends `Open { request, format, length }`. The worker requests source
   ranges with `SourceRead`; the host responds with `SourceData` using the same
   request ID. A successful open returns `Opened { root: 1 }`.
4. The host sends serial `List` and `Read` requests. Each has one terminal reply;
   nested source broker exchanges are allowed before that reply. Request IDs
   strictly increase; stale or mismatched responses fail the session.
5. `Close`, pipe closure, worker failure, a protocol violation, a cancelled startup
   or deadline expiry ends the session. Cancelling a listing or member read does not
   (see [cancellation](#errors-cancellation-and-resource-bounds)). The current host
   terminates the worker when its last mount lease is released; destruction never
   waits for a child on the UI thread.

The executable receives pipes and a source length, never a source filename,
arbitrary native handles, command text, credentials or network access. It must
not reconstruct paths from display labels. Native source identity remains
host-owned. The host validates identity, length and change/write timestamps
before and after operations, and before/after each broker read. A detected change
invalidates the operation; consumers must not combine different revisions.
This is live validation, not a filesystem snapshot guarantee.

The [archive worker entry point](../../plugins/midnight-fs-archive/src/main.rs)
shows the complete handshake, source-read loop, dispatch and terminal errors.
Its parser library has an ordinary byte-source interface so real archive fixture
tests run without native file authority.

## Framing, identities and capabilities

Frames have a little-endian `u32` payload length, followed by protocol `u16`, tag
`u8`, request ID `u64` and message fields. Payloads are limited to 1 MiB. The codec
checks sizes before allocation and rejects unknown tags, invalid booleans/enums,
malformed UTF-8 strings and trailing bytes. Raw names are length-prefixed bytes,
separate from lossy UTF-8 display labels.

| Request | Terminal response | Contract |
| --- | --- | --- |
| `Open` | `Opened` or `Failed` | One source and one immutable mount per worker |
| `List { directory, after }` | `Entries` or `Failed` | `after` is the exclusive last node ID; continue while `more` is true |
| `Read { entry, offset, length }` | `ContentReady` (v2/v3), `Content` (v1) or `Failed` | Checked `u64` byte offsets, requested length 1–256 KiB |
| `SourceRead` | `SourceData` or failure/closure | Nested worker-to-host read within the current request |
| `Close` | Worker exit | No terminal protocol reply required |

Protocol 2 adds host-owned staging: StageBegin/Created, bounded StageAppend/Ack,
StageSeal/Sealed, SpoolRead/Data, and ContentReady naming a checked sealed range.
StageProgress/Ack reports strictly increasing actual decoded work while validating
discarded solid siblings. Every exchange retains its active request ID; member
stages cannot escape their request/member authority. Scratch stages can be reused
within the mount after complete verification. The codec helpers require explicit
`Protocol::V2` or `Protocol::V3` (same staging messages, plus cooperative member
cancellation); default helpers retain exact protocol-1 bytes. See the wire reference
for all fields. No native path or file handle crosses this interface.

Root ID is `1`; member IDs start at `2` and are opaque within that mount. Listing
IDs strictly increase, each node's parent matches the requested directory, no node
is that directory itself, only the final page (`more` false) may be empty, and
each batch has at most 128 nodes. Name and display fields each have a 64 KiB bound;
the entire batch must fit the frame limit. A listing page that breaks these rules,
or protocol-1 `Content` longer than the requested length, ends the session, even
when its request was cancelled meanwhile. Duplicate filenames and names that
display identically retain distinct IDs. Do not use a display string, hash of a
name, index in a sorted list or native `PathBuf` as member identity.

A node reports `Directory`, `File`, `Link` or `Other`, optional byte size and a
`readable` flag. These are the capabilities available in this version: directory
listing and readable-file content. There is no mutation capability bitmap,
extraction request, native path conversion or external application launch. The
host disables create, rename, delete, move, paste, editing, native clipboard and
terminal operations in archive locations. F3 uses a provider content lease through
the existing viewer broker, backed by a private host-owned TEMP file after complete
member integrity verification. All viewers share this generic content service.
F5 copy-out uses readable-member streams through the host's ordinary bounded copy
queue; the plugin still receives no extraction or write request. The host validates
raw destination basenames and requires independent SHA-256 readback before native
no-replace publication. Linux preserves raw filename bytes; Windows rejects names
that cannot be represented losslessly as Unicode. Existing targets remain conflicts.
Copying out takes folders and selections of any size: nothing limits how many
members one copy takes or how many bytes their names hold in all. A selection is
resolved in one pass over its folder's listing, keeping only the selected members and
checking Stop every few thousand rows; the copy then takes its roots one at a time,
and each selected member holds one root source until it is copied. It keeps the
identities it has met (a member listed twice, a cycle or a duplicate identity, fails
the copy) and the listing of each folder on its current path. Resolving a selection of
1,000,000 members grows the process by about 355 MiB, about 372 bytes per selected
member (the ignored test `archive_selection_memory_per_member`, Linux, release build,
21 November 2026; 375 bytes in a debug build).

Archive folder sizes are a host backend service over these listing capabilities.
Ctrl+Shift+S captures the exact committed mount/listing and opaque selected IDs;
with no selection it measures all immediate folders. A files-only selection never
falls back to all folders. Space measures the focused ordinary folder while
advancing selection. The iterative scan sums stored logical file lengths, excludes
links and special nodes, validates parent/ID relationships and reports partial
results for missing lengths or errors. It never requests member content, extracts
files or uses viewer TEMP paths. A listing or scan of any size is measured: nothing
limits how many members or folders it holds or the bytes of their names, beyond the
protocol's 64 KiB per name. The scan keeps the identities it has met and the
folders still to visit along its path, and drops each folder's listing once it has
counted it; the results keep one size per folder. Sorting and navigation within the same live
mount preserve cached values and running scans. The status line includes known
selected-folder totals; no unknown-size placeholder is added.

Progress is presented at most four times per second. Snapshot publication is
bounded to one unacknowledged map per pane, with later changes coalesced; the UI
acknowledges stale snapshots too and retires large maps on backend workers.
Invalidated mounts lose their cached totals. Idle observers retain weak authority
so they cannot keep a closed archive's host mount permit alive. Inline expanded
tree selections require separate captured tree authority and are not yet admitted
by this flat archive-listing action.

## Persistent addresses and leases

Runtime IDs are never saved. Host settings store a separately versioned address:
provider ID, native container path, captured source revision bytes and raw member
directory components. The native path uses the existing OS-specific path codec;
member components are never joined onto that path. The address limit is 256
components and 64 KiB aggregate member bytes, with a source revision of 1–256
bytes. Components reject empty, dot, dot-dot, NUL, slash, backslash and colon.
These are application persistence rules, not a public Rust provider trait.

Active saved tabs restore by exact provider ID and source revision, then resolve
components through provider listings. Inactive tabs restore lazily. Missing
providers, changed archives and failed locators retain an unavailable read-only
tab with its address and diagnostic; they never fall back to native member paths.
Parent navigation can leave an unavailable archive and return to its native
container directory. In-session tabs and viewers retain mount leases, so closing
one tab does not invalidate a viewer still reading that captured member.

## Errors, cancellation and resource bounds

`Failed` carries one of `Invalid`, `Unsupported`, `Limit`, `Cancelled`,
`SourceChanged`, `NotFound`, `ReadOnly`, `Corrupt` or `Io`, plus at most 4096 UTF-8
diagnostic bytes. Report unsupported/encrypted codecs explicitly; never retry
through an external utility or a host path. The host ends the mount on a failed exchange, except the explicitly negotiated
protocol-3 cooperative cancellation acknowledgement described below. Protocol violations, truncated frames and unexpected replies
also fail closed. Cancelling one request never ends a mount that other tabs,
viewers and jobs share: a listing stops between pages and discards a conforming
in-flight page; protocol-3 member reads cancel cooperatively through both the ordinary
and the preview read entry points; a protocol-1/2 member read cannot be interrupted,
so a cancelled one finishes its exchange under the idle-progress watchdog and the
host discards its result. Such a legacy read can therefore delay its caller's
cancellation until the member is staged. Only the startup exchange, whose caller
still owns the whole mount, is hard-cancellable. Mount teardown, protocol
violations and the idle watchdog always terminate the worker. Deadlines apply to
blocking pipe reads as well as decoding. A request in flight when the worker ends,
and every later request of that mount, reports why it ended: the failed exchange
with its failure, or idle-deadline expiry as a timeout. A worker that ends by itself
(its pipe closes mid-request) is reported as having ended unexpectedly, for example
because the archive needs more than the worker's memory. Only a request's own
cancellation, including a cancelled startup, is reported as interrupted, and a
`Failed(Cancelled)` that acknowledges no host cancellation is a protocol violation.
The bundled worker reports a failed read of its source with the reply's own code
(`Io` for a source that failed outside the protocol), never as `Corrupt`.

Current application limits are at most two live archive mounts for panels, tabs and
viewers, 1 GiB process memory (`WORKER_MEMORY`, the most the worker SDK grants; 512
MiB for protocol 1) and a 20-second idle-progress watchdog. Find's private
sessions (`Mount::open_for_search`, at most two: one archive per running search) and
archives inside archives (at most three, below) are counted apart, so none of these
uses refuses another's mounts. Protocol 2 adds no aggregate CPU
ceiling; inherited OS resource policy remains in effect. Only successful bounded
byte I/O and monotonic decoded work renew the watchdog. Protocol 1 retains its
legacy limits. Shared tab/viewer leases count as the same mount. Source reads may
revisit captured ranges and are bounded to 256 KiB. Nothing limits how many members
a directory lists or how many bytes their names take: the host requests page after
page for as long as the worker says more follow, checking cancellation between
pages, and each page stays within the frame and batch bounds above. The worker's
memory bounds an archive's index instead: measured at about 105 bytes per TAR
member, 420 per 7z member and 690 per ZIP member at peak (a million members), 1 GiB
holds about nine million TAR, two million 7z or one and a half million ZIP members.
That 1 GiB, the worker SDK's maximum, is the remaining limit. In the application
process a listed folder costs per member about 186 bytes for the host's rows, 639
for a panel listing (its rows, collation keys and index) and 339 for the tree
provider's capture (a million 16-byte names: `archive_folder_memory_per_member` in
`crates/midnight-engine/src/providers/archive_tests.rs`), so a panel showing a folder
of a million members holds about 610 MiB. That test uses stand-in collation keys; a
real Linux en_US key of a 16-character name takes 108 bytes (312 before the keys'
variable-length code of 21 November), about 192 bytes less per member
([transfer and listing performance](../verification/2026-11-21-transfer-and-listing-performance.md)).
The idle supervisor waits without polling; it observes startup cancellation every
10 ms only while that exchange is active. A failed `release_on_worker` retains its
host reference instead of dropping it. Once every host reference of a session whose
cleanup is unconfirmed has been released that way, the host still terminates its
worker and returns staging and the mount permit; the unconfirmed source stays
quarantined. A reference dropped without `release_on_worker` is not observed, so
every owner releases its host reference explicitly; the engine's mount owner holds
a single one.

The current parser imposes no artificial compressed-source, member, aggregate disk
or expansion-ratio byte ceiling; free space on the host's private spool volume
bounds decoded content. The host stops staging with a disk-full (`StorageFull`)
error before that volume's free space falls below max(1 GiB, 2% of capacity); it
queries the volume on each stage's first append and at least every 64 MiB, and
writes by other programs between two queries are not counted. A member whose
`StageBegin` declares a length above the 16 MiB memory stage limit (it goes to a spool
file at once) first claims that many bytes of the volume's free space above the reserve
(`staging_space.rs`): claims of stages still writing count against every later stage of
the mounts that share the process's spool root, and each claim shrinks as its stage
writes and ends with the stage. A member that does not fit is refused before its spool
file exists, with a `StorageFull` error that names the space it needs, the space above
the reserve and what other archive reads still need. Under protocol 3 the host refuses
it through the cooperative cancellation acknowledgement (the session keeps serving its
other users); under protocol 2 the refusal ends the exchange like any staging failure.
Content of undeclared length is checked as it is appended, and so are spills of small
stages. A refusal at the reserve makes the next mount choose its spool root again. The
parser sets no member-count or name-bytes limit. It retains the protocol's 64 KiB
per-name limit, a 16 MiB bound on each buffered TAR long-name/long-link/PAX record,
64 MiB XZ/Zstandard windows, a 256 MiB 7z LZMA/LZMA2 dictionary check (larger
blocks' members are unreadable) and a raw 7z next header that must lie inside the
source and fit an eighth of the worker's memory (128 MiB, the uncompressed header of
about two and a half million members). An index that outgrows the worker's memory
fails the archive with `Limit`; compressed 7z header decoding and ZIP
central-directory parsing are bounded only by the worker memory limit. Source reads use a
seek-preserving 256 KiB window. ZIP and plain TAR browse indexes without
whole-source buffering; selected members stream directly to disk. Solid 7z verifies
the full required block and stages only the selected member. The block's other
readable members stay in worker memory (up to 128 MiB, later members first) until
read: members read within that window need no further block decode, while a larger
block or a read outside the window decodes the block again.
Compressed TAR verifies one complete scratch stream, indexing its headers while
staging it, and reuses checked ranges. The host records a CRC-32 for every 64 KiB
block of a spool file as it appends, and a read checks each block it touches, read
whole, instead of the spool being flushed when it is sealed: lost or failed write-back
is a read error, never content.
A staged stream or member of at most 16 MiB stays in host memory instead of a spool
file while the process's in-memory staging budget has room: 128 MiB, shared by every
archive mount of the process. Creating a spool file waits for the volume's journal
while other programs flush their writes, which takes seconds on a busy disk. Content
declared larger than 16 MiB gets a spool file at once. A stage that outgrows 16 MiB,
or whose next chunk no longer fits the budget, moves its bytes to a new spool file and
continues there; those bytes count against the volume's reserve like any other. Its
memory returns to the budget when it moves, or when the mount and the last read of that
content close. Memory needs no CRC: only spool files can change behind the process.
Disk errors abort staging; final spool cleanup runs on backend workers after any
outstanding native I/O returns. Each mount keeps up to 16 decoded members for
repeated reads. Beyond 512 MiB of their staged content (in memory or in spool files),
least recently read members
leave first, except members in active use: one of the four most recently read
members that was also read within the mount's last 64 member reads stays even
beyond the budget. A sequential reader whose chunks interleave with reads of up to
three other members (a copy beside the two thumbnail renders, a viewer or another
copy) therefore never decodes its member again between two chunks. A member whose
reader moved on leaves once later reads pass it; an idle mount trims nothing until
its next member read. Spools are anonymous files in a per-user private directory: on
Windows `MidnightExplorer-archive-spool` in the user's temporary directory, then
`MidnightExplorer-archive-spool-<pid>` there. On Linux the candidates are
`$XDG_CACHE_HOME` (or `~/.cache`) `/midnight-explorer/archive-spool`, then
`midnight-explorer-<uid>-archive-spool` in the temporary directory, then
`midnight-explorer-archive-spool-<pid>` there, and last
`$XDG_RUNTIME_DIR/midnight-explorer-archive-spool`, which is usually a small tmpfs.
Links in these base directories are resolved first; each spool root itself must be a
directory owned by the user with mode 0700. A candidate that cannot be opened that
way or cannot hold anonymous files yields to the next, so a directory that another
account pre-creates only disables that candidate. A mount uses the first usable
candidate whose volume has at least 64 MiB free above its reserve, or else the first
usable one. The process makes that choice once, creating one test spool, and later
mounts reuse the root. The engine makes it on a background thread right after plugin
discovery when an installed worker stages content (`Catalog::prepare_staging`), so the
first archive opened does not wait for the test spool; a mount that starts during that
check waits for its choice instead of making its own. A spool that cannot be created,
written or sized there, or a refusal at the volume's reserve, makes the next mount
choose again. A `<pid>` root serves only this process: every mount that uses it checks
it again, and it is removed after its last spool closes. A fixed shared name is never
used.

Protocol-2 nodes also carry optional typed UTC seconds/nanoseconds or timezone-free
local civil timestamps. ZIP DOS times never acquire an invented UTC offset.

Supported formats are ZIP store/deflate, TAR including GNU/PAX names, concatenated
GZIP/XZ/BZIP2/Zstandard and their TAR wrappers, and 7z Copy/LZMA/LZMA2 with the
enabled pure Rust BCJ filters. Raw compressed streams
expose a synthetic `content` member, ignoring untrusted FNAME. Links, junctions,
reparse entries, hard links, special files, absolute/traversal names, backslashes,
drive/ADS colons, NUL names and files named by a final `.` are unreadable; unsafe
directory records are listed as non-navigable `Other` nodes, while harmless `.`
components such as `./usr/bin` name the current directory. The worker has no disk
extraction capability. TAR and 7z duplicate regular files remain distinct; the
pinned ZIP reader merges central-directory records that repeat a name (the raw name
or the Info-ZIP Unicode Path replacing it) into a single entry, which is listed but
unreadable, and a ZIP whose records the worker cannot reconcile with the reader's
index is refused. Encrypted or unsupported ZIP members
and 7z blocks are unreadable without hiding the rest of the archive. Passwords,
multi-volume archives, RAR and archive writing are unsupported.

Archives inside archives need no worker change. `ArchiveMount::stage_member` asks the
enclosing session for one ordinary member read (cooperatively cancellable, with
progress at each `StageAck`, and refused before anything is staged when the member's
declared length does not fit the spool volume, as above) and keeps the sealed lease
that the read publishes; the lease stays readable after the member cache evicts it
and after that worker ends.
`Mount::open_nested` then starts another worker whose captured source is that lease
(`CapturedFile::from_source`): it receives the usual `Open` with the member's length
and reads it through `SourceRead`, and each read revalidates the enclosing chain down
to the native file. The nested mount reports its enclosing folder through
`ArchiveMount::parent`, names the native file of the outermost archive as its
container, and inherits persistent member identities only from enclosing providers
with stable member IDs. Nesting stops at `MAX_ARCHIVE_DEPTH` (4) archive levels, and
at most three nested mounts run beside the two mounts that panels, tabs and viewers
open from native files (and Find's own two). Protocol-1 workers cannot stage members,
so archives inside them do not open.
User-facing behavior is in [archives inside archives](../ARCHIVE_FORMATS.md#archives-inside-archives).

## Independent builds and verification

The [plugin build scripts](../../plugins/midnight-fs-archive/scripts/) take a
public SDK configuration and explicit output directory under `_build`. With the
toolchain on PATH, a Windows example is:

```powershell
./plugins/midnight-fs-archive/scripts/build.ps1 `
  -SdkConfig D:/release/worker-sdk/.cargo/registry.toml `
  -BuildRoot <work-dir>/_build/archive-worker `
  -OutputDirectory <work-dir>/_build/install/filesystems/archive `
  -Test -Offline
```

Linux uses `scripts/build.sh SDK_CONFIG /work/_build/archive-worker
/work/_build/install/filesystems/archive`. Set `CARGO_NET_OFFLINE=true` for an
offline Linux build. These scripts test/build the independent Cargo project and
stage its executable, target-specific manifest and third-party notices. Plugin
format dependencies must already be available in the configured Cargo cache or
vendor source; the public SDK bundle is not the complete archive parser bundle.
Build/package staging and trust validation are separate steps.

For development-only API packaging, `filesystem-api/scripts/package.py` creates an
offline directory source from Cargo package artifacts, with explicit `_build`
output and immutable checksums. Distribution uses the separate canonical worker
SDK bundle described in [worker SDK packaging](WORKER_SDK.md). Their archive bytes
and checksums differ: never substitute one bundle for another while retaining the
wrong public package checksums in a lockfile. Released viewer bundles are separate.

The updated plugin has passed 16 archive fixture tests with
`--locked --offline` from outside the host checkout, using packaged APIs only.
Fixtures cover real ZIP/GZIP/XZ/BZIP2/Zstandard/TAR/7z, duplicate solid members, invalid names,
Unicode, long names, links, CRC/trailer errors, malformed/truncated input and
resource bounds. Host integration additionally exercises discovery, source
brokering, changed-source failure and absent/duplicate providers. Current
platform qualification and exact commands are recorded in the
[implementation checkpoint](../archive-filesystem-development.md).

## Protocol 3 cooperative member staging cancellation (MEDIA-01)

Protocol 3 is an explicit installed-worker opt-in, preserving protocol 2 wire
messages and resource bounds while adding a request-local cancellation contract.
The host applies it to ordinary and preview member reads alike. Against legacy
protocol 1/2, either read drains its admitted exchange without a hard token and
suppresses the cancelled result. No extra worker, mount, queue, source authority
or native path is added.

The thumbnail broker passes viewport cancellation OR decoder shutdown/timeout into
a separate cooperative member read. At the next valid StageBegin, StageAppend,
StageProgress or StageSeal exchange, the host may reply Failed(Cancelled) for the
active request instead of performing that staging operation. The worker unwinds
that prepare call and sends exactly one matching terminal Failed(Cancelled), then
accepts the next strictly increasing request. SourceRead is not interrupted this
way: generic decoder readers can retry Interrupted, so cancellation is observed at
the existing bounded staging/progress checkpoints instead.

Among `Failed(Cancelled)` replies, only the exact terminal acknowledgement of a
host-initiated cancellation keeps the session alive. The host retires only that
request's pending/sealed member spool; existing scratch, member cache and
outstanding leases remain valid. A normal terminal result racing cancellation is
validated but cannot publish newly prepared content, to the caller or to the
member cache. Source changes, malformed frames, wrong IDs, unexpected acknowledgement,
I/O errors and the idle watchdog still fail closed. The hard watchdog remains armed
while the cancellation acknowledgement drains and cannot be renewed by chatter.
Real decoder/I/O capacity remains held until native I/O and spool retirement finish.

This is cooperative cancellation, not an OS-call deadline. One already-admitted
bounded source/spool call or decoder work interval can delay the next checkpoint.
CRC/length validation still gates every successful content publication. All existing
memory, chunk, cache, process and decoder-window limits remain unchanged.
