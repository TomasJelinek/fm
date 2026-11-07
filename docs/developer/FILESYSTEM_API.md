# Read-only filesystem worker API

`midnight-filesystem-api` version `0.1.0` implements protocol **1** for independent
archive filesystem executables. It supports opening one captured byte source,
listing directories and reading members. The application integrates this with
pane navigation, selection, sorting, filtering, tabs and F3 viewing. This is a
separate public workspace from the viewer C ABI: the viewer's `KIND_FILESYSTEM`
constant remains reserved, and released viewer API/SDK `0.1.0` packages are
unchanged. General native/network providers and filesystem mutation are future
contracts, not methods exported by this package.

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
midnight-filesystem-api = { version = "=0.1.0", registry = "midnight" }
midnight-worker-sdk = { version = "=0.1.0", registry = "midnight" }
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
protocol = 1
formats = ["zip", "tar", "gzip", "tar-gzip", "7z"]
```

Linux uses its exact host target and executable basename without `.exe`. The
entry must be a single basename: no separators, colon, `.` or `..`. IDs are
1–128 ASCII letters, digits, dots, hyphens or underscores. Unknown manifest fields,
formats, schema/protocol versions or mismatched targets are refused. Manifests are at most
16 KiB, discovery examines at most 64 packages, and duplicate IDs disable every
package with that ID. The manifest version is nonempty package metadata; the
wire protocol field determines compatibility. Restart after installation changes.

The host validates the installation, ancestors, package, manifest and executable
using the native plugin ownership/ACL policy. Redirections and unsafe writable
ancestors are refused. Installing an executable grants it code trust; containment
limits the untrusted documents it parses. A broadly writable build directory may
be unsuitable as a trusted installation. Do not loosen source-tree permissions to
make discovery accept it.

No package means no archive browsing capability. Native archive files retain
normal OS activation when the catalog has no providers. With providers installed,
the current host selects ZIP/JAR, TAR, GZ/GZIP, TAR.GZ/TGZ or 7z by extension and
the manifest's format list; there is no arbitrary extension registration or
provider-priority negotiation in protocol 1.

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
5. `Close`, pipe closure, worker failure, cancellation or deadline ends the
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
| `Read { entry, offset, length }` | `Content` or `Failed` | Checked `u64` byte offsets, requested length 1–256 KiB |
| `SourceRead` | `SourceData` or failure/closure | Nested worker-to-host read within the current request |
| `Close` | Worker exit | No terminal protocol reply required |

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
the existing viewer broker; it does not create a temporary extracted file.

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
through an external utility or a host path. The current host ends the mount on a
failed exchange. Protocol violations, truncated frames and unexpected replies
also fail closed. Cancellation is host process termination, not an in-band
request that promises the mount survives. Deadlines apply to blocking pipe reads
as well as decoding; timeout/cancellation can therefore surface as a closed-pipe
or interrupted operation diagnostic.

Current application limits are at most two live archive mounts, 512 MiB process
memory, 15 seconds cumulative process CPU and a 20-second deadline per operation.
Shared tab/viewer leases count as the same mount. The source broker allows at
most the source length in aggregate bytes per exchange, with each range inside
the captured length. Source reads are bounded to 256 KiB. Host listing validation
also limits a directory to 100,000 nodes and 8 MiB aggregate raw/display names.

The reference parser adds 64 MiB compressed source, 64 MiB per member, 128 MiB
aggregate decoded data, 100,000 nodes, 8 MiB names and expansion ratio 200. It
eagerly verifies and caches a bounded mount in memory. Solid 7z decoding is serial;
later member reads use that verified cache. These limits can reject valid large
archives. They do not imply unlimited streaming or per-entry parallel decoding.

Supported formats are ZIP store/deflate, TAR including GNU/PAX names, concatenated
GZIP, TAR.GZ, and 7z Copy/LZMA/LZMA2 with the enabled pure Rust BCJ filters. GZIP
exposes a synthetic `content` member, ignoring untrusted FNAME. Links, junctions,
reparse entries, hard links, special files, absolute/traversal names, backslashes,
drive/ADS colons and NUL names are unreadable; there is no disk extraction.
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

An exported plugin directory has passed all 11 archive fixture tests with
`--locked --offline` from outside the host checkout, using packaged APIs only.
Fixtures cover real ZIP/GZIP/TAR/7z, duplicate solid members, invalid names,
Unicode, long names, links, CRC/trailer errors, malformed/truncated input and
resource bounds. Host integration additionally exercises discovery, source
brokering, changed-source failure and absent/duplicate providers. Current
platform qualification and exact commands are recorded in the
[implementation checkpoint](../archive-filesystem-development.md).
