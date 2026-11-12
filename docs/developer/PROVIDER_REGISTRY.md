# Internal filesystem registry

`midnight-core` now supplies `ProviderId`, `ProviderInstanceId`, `MountRef`,
`NodeToken`, `NodeRef` and `StoredLocation`. `midnight-engine::providers` supplies
the transitional backend registry, Browse/Read contracts and adapters over current
native services and captured archive mounts. These Rust contracts are internal;
independent plugins must continue to depend only on the released public API/SDK.
They are not the missing generic filesystem C ABI.

Production panels still use their existing native/archive routes. This increment
does not replace `Entry`, selection indexes, settings addresses, watches, mutation
capabilities or application service injection. See the
[migration dependency map](FILESYSTEM_MIGRATION.md) before attempting that cutover.
The native adapter is explicitly transitional, not an authorized static fallback
after native plugins become mandatory.

## Identity and restore

- Package IDs are validated ASCII identifiers, independent of captions.
- Host-issued instance/mount/generation counters fail on exhaustion and are never
  persisted. The same opaque node bytes in different mounts cannot be interchanged.
- Node tokens contain at most 256 opaque bytes. They are not paths; native adapters
  retain full paths in bounded backend tables. The provider must never reuse a
  token for another node or kind during that mount's lifetime.
- A node reference identifies a logical browse target, not a pinned inode or
  mutation capability. Content captures retain their own revision validation.
- Stored locations retain provider, nonzero codec version and exact address bytes.
  Their representation imposes no universal path-length limit. Persistence and
  wire callers must apply aggregate resource budgets without truncation. Captions
  are presentation only, and credentials do not belong in stored addresses.
- Missing registrations return `Unavailable`; unsupported codecs return
  `Unsupported`. There is no fallback interpreting provider bytes as native paths.

Native address codec 1 belongs separately to `native.windows` (little-endian raw
UTF-16) and `native.linux` (raw native bytes). It rejects NUL and nonabsolute paths;
Windows also rejects odd-length UTF-16. Unpaired surrogates/non-UTF-8 bytes and long
paths survive encoding without lossy conversion. macOS still needs its own native
implementation and codec; it does not use the Linux adapter implicitly.

## Lifetimes, cancellation and budgets

Registry methods run on backend workers, including final resource destruction.
Provider calls run outside the registry lock. A blocked provider therefore does
not prevent another registration from resolving a location. Current synchronous
contracts are called from the engine's bounded workers; they do not create threads
or promise to interrupt an in-flight native call.

`unregister(id, instance)` explicitly revokes that registration and its requests,
mounts and content. A delayed call for an old instance cannot revoke a replacement.
Dropping the registry also revokes outstanding leases. Merely closing a panel or
dropping a location does not revoke content retained by another consumer.

The mount retains the registration/provider owner; content retains the mount.
Destruction order is content, mount, then provider/library owner. Revoked objects
remain allocated while needed for safe cleanup, but successful late output is
discarded. `Cancellation::linked` joins request and registration lifetimes without
cancelling either parent or unrelated requests.

Initial bounds are 128 registrations, 64 in-flight/open/retained mounts per
registry, 50,000 entries/32 MiB charged payload per complete list, and 100,000
issued nodes per mount. The native adapter separately accounts for 32 MiB of raw
path storage and 100,000 cached tokens. These are admission/cache budgets, not an
OS path limit or an exact process RSS promise. A failure returns an explicit error.
Native stored addresses are admitted before codec allocation, using the cache
budget and that codec's worst-case expansion; native encoding checks its raw
charge first. The production cutover must also align caches across mounts with
the shared application budget, rather than treating each per-mount limit as RSS.
The registry's complete-list adapter is not yet the production streaming panel
projection. Folder-only listings use a separate bounded token-validation ledger,
so irrelevant regular files do not consume the retained-folder quota. See
[inline provider branches](INLINE_TREES.md) for the backend tree model, ownership,
limits and pending production projection.

Listings reject duplicate/self-referencing tokens, contradictory known kinds,
oversized output and cancellation. A failed registry listing issues no partial
new references. Parent navigation is delegated to the provider. The initial
location's `root()` is its starting directory; it may have a provider parent.

## Content and adapters

`MountedLocation::open_content` accepts an issued regular-file node, obtains the
provider's source and uses the existing `CapturedFile` broker to validate its
captured length/version. The returned source also enforces the broker's bounded
256 KiB exact reads, EOF clamping and before/after revision validation. It retains
the mount owner and checks revocation around calls. Viewer consumers never receive
a fabricated native member path. This is LiveValidated content, not a claim of
an immutable snapshot. Unknown-length/sequential sources need an additive API.

The native adapter delegates directory enumeration to an injected `FileSystem`.
`NativeProvider::new` is browse-only: `with_capture` supplies the explicit native
plugin capture seam and unavailable capture fails without a built-in fallback.
`with_locale` uses the shared backend date/time formatter; complete enumeration
metadata avoids an extra stat, while incomplete rows use the injected service.
The adapter is read-only and
provides no shell bridge, mutation, copy destination or removal authority. Native
entry paths must be direct children of the requested directory. Raw names use
the platform codec, while captions use display conversion only.

`captured_archive` adapts an existing validated `ArchiveLocation`; it does not
open a container or discover/load plugins. Tokens retain archive mount/member
identity; parent links come from the captured trail and validated listing records.
It retains the source revision, rejects unknown-size/unreadable/link members for
content, validates exact bounded reads, and installs list cache changes only after
the complete successful callback sequence. Existing archive worker restrictions,
source limits and publication/copy-out guarantees remain unchanged.

`resolve_target` returns a retained backend context: an exact native path or an
archive directory/member with its full captured raw ancestry. A file's archive
location denotes its immediate parent; a directory's location denotes itself.
It never interprets captions, lists again or extracts data to reconstruct a
target. Invalid/cyclic ancestry and resource exhaustion fail explicitly; links
and special archive members do not gain unsupported actions.

`MountedTarget` validates the source registration and any enclosing tree
contexts. Cloning an archive location or mount retains the same leases: list,
read and validate reject revocation before and after provider calls. Resources
are destroyed before their registrations. Cancellation propagates into running
provider calls; a late successful reply cannot publish after cancellation.
These are routing/lifetime checks, not immutable file identity, authorization
to mutate, or proof that a row is still selected in the current panel snapshot.

## Qualification and remaining migration

Tests cover provider isolation, cancellation, late output, exhausted IDs, retained
content and destruction order, malformed responses, unknown providers/codecs,
non-Unicode names, native source replacement detection, and archive reads. See
`providers/*tests.rs`, `midnight-core/tests/vfs_identity.rs` and
`midnight-core/tests/linked_cancellation.rs`.

Before production use, migrate complete navigation → projection → selection → F3
and native capability checks, qualify real worker adapters, publish the independent
filesystem API/SDK, port every native operation through equivalent pinned handles,
and remove static native browsing only after parity. Inline trees and network
discovery remain subsequent consumers of these identities. Passing registry tests
alone does not complete VFS-01, TREE-01 or NET-01.
