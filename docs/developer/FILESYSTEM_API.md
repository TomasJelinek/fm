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

The host validates the installation, ancestors, package, manifest and executable
using the native plugin ownership/ACL policy. Redirections and unsafe writable
ancestors are refused. Installing an executable grants it code trust; containment
limits the untrusted documents it parses. A broadly writable build directory may
be unsuitable as a trusted installation. Do not loosen source-tree permissions to
make discovery accept it.

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
5. `Close`, pipe closure, worker failure, hard cancellation or deadline ends the
   session. The current host terminates the worker when its last mount lease is
   released; destruction never waits for a child on the UI thread.

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
| `Read { entry, offset, length }` | `ContentReady` (v2), `Content` (v1) or `Failed` | Checked `u64` byte offsets, requested length 1–256 KiB |
| `SourceRead` | `SourceData` or failure/closure | Nested worker-to-host read within the current request |
| `Close` | Worker exit | No terminal protocol reply required |

Protocol 2 adds host-owned staging: StageBegin/Created, bounded StageAppend/Ack,
StageSeal/Sealed, SpoolRead/Data, and ContentReady naming a checked sealed range.
StageProgress/Ack reports strictly increasing actual decoded work while validating
discarded solid siblings. Every exchange retains its active request ID; member
stages cannot escape their request/member authority. Scratch stages can be reused
within the mount after complete verification. The codec helpers require explicit
Protocol::V2; default helpers retain exact protocol-1 bytes. See the wire reference
for all fields. No native path or file handle crosses this interface.

Root ID is `1`; member IDs start at `2` and are opaque within that mount. Listing
IDs strictly increase, each node's parent matches the requested directory, and
each batch has at most 128 nodes. Name and display fields each have a 64 KiB bound;
the entire batch must fit the frame limit. Duplicate filenames and names that
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

Archive folder sizes are a host backend service over these listing capabilities.
Ctrl+Shift+S captures the exact committed mount/listing and opaque selected IDs;
with no selection it measures all immediate folders. A files-only selection never
falls back to all folders. Space measures the focused ordinary folder while
advancing selection. The iterative scan sums stored logical file lengths, excludes
links and special nodes, validates parent/ID relationships and reports partial
results for missing lengths or errors. It never requests member content, extracts
files or uses viewer TEMP paths. Sorting and navigation within the same live
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
also fail closed. Ordinary cancellation remains host process termination; only
the separate protocol-3 cooperative read can preserve the mount. Deadlines apply to blocking pipe reads
as well as decoding; timeout/cancellation can therefore surface as a closed-pipe
or interrupted operation diagnostic.

Current application limits are at most two live archive mounts, 512 MiB process
memory and a 20-second idle-progress watchdog. Protocol 2 adds no aggregate CPU
ceiling; inherited OS resource policy remains in effect. Only successful bounded
byte I/O and monotonic decoded work renew the watchdog. Protocol 1 retains its
legacy limits. Shared tab/viewer leases count as the same mount. Source reads may
revisit captured ranges and are bounded to 256 KiB. Host listing validation
also limits a directory to 100,000 nodes and 8 MiB aggregate raw/display names.

The current parser imposes no artificial compressed-source, member, aggregate disk
or expansion-ratio byte ceiling. It retains 100,000-node and 8 MiB name limits,
bounded decoder windows and a 64 MiB raw 7z next-header guard. Compressed 7z header
allocation remains inside the worker memory boundary. ZIP and plain TAR browse
indexes without whole-source buffering; selected members stream directly to disk.
Solid 7z verifies the full required block and stores only the selected member.
Compressed TAR verifies one complete scratch stream and reuses checked ranges.
Disk errors abort staging; final spool cleanup runs on backend workers after any
outstanding native I/O returns. The host retains at most 16 cached member leases.

Protocol-2 nodes also carry optional typed UTC seconds/nanoseconds or timezone-free
local civil timestamps. ZIP DOS times never acquire an invented UTC offset.

Supported formats are ZIP store/deflate, TAR including GNU/PAX names, concatenated
GZIP/XZ/BZIP2/Zstandard and their TAR wrappers, and 7z Copy/LZMA/LZMA2 with the
enabled pure Rust BCJ filters. Raw compressed streams
expose a synthetic `content` member, ignoring untrusted FNAME. Links, junctions,
reparse entries, hard links, special files, absolute/traversal names, backslashes,
drive/ADS colons and NUL names are unreadable; the worker has no disk extraction capability.
Duplicate regular files remain distinct. Encryption, passwords, multi-volume
archives, RAR, nested mounts and archive writing are unsupported.

## Independent builds and verification

The [plugin build scripts](../../plugins/midnight-fs-archive/scripts/) take a
public SDK configuration and explicit output directory under `_build`. With the
toolchain on PATH, a Windows example is:

```powershell
./plugins/midnight-fs-archive/scripts/build.ps1 `
  -SdkConfig D:/release/worker-sdk/.cargo/registry.toml `
  -BuildRoot D:/work/_build/archive-worker `
  -OutputDirectory D:/work/_build/install/filesystems/archive `
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
Ordinary hard cancellation retains session-ending semantics under every protocol.
A cooperative preview read against legacy protocol 1/2 drains under an independent
hard token and suppresses its canceled result. No extra worker, mount, queue,
source authority or native path is added.

The thumbnail broker passes viewport cancellation OR decoder shutdown/timeout into
a separate cooperative member read. At the next valid StageBegin, StageAppend,
StageProgress or StageSeal exchange, the host may reply Failed(Cancelled) for the
active request instead of performing that staging operation. The worker unwinds
that prepare call and sends exactly one matching terminal Failed(Cancelled), then
accepts the next strictly increasing request. SourceRead is not interrupted this
way: generic decoder readers can retry Interrupted, so cancellation is observed at
the existing bounded staging/progress checkpoints instead.

Only a host-initiated cancellation with the exact terminal acknowledgement keeps
the session alive. The host retires only that request's pending/sealed member spool;
existing scratch, member cache and outstanding leases remain valid. A normal
terminal result racing cancellation is validated but cannot publish newly prepared
content. Source changes, malformed frames, wrong IDs, unexpected acknowledgement,
I/O errors and the idle watchdog still fail closed. The hard watchdog remains armed
while the cancellation acknowledgement drains and cannot be renewed by chatter.
Real decoder/I/O capacity remains held until native I/O and spool retirement finish.

This is cooperative cancellation, not an OS-call deadline. One already-admitted
bounded source/spool call or decoder work interval can delay the next checkpoint.
CRC/length validation still gates every successful content publication. All existing
memory, chunk, cache, process and decoder-window limits remain unchanged.
