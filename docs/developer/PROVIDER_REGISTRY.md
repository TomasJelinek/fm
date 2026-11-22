# Internal filesystem registry

`midnight-core` supplies `ProviderId`, `ProviderInstanceId`, `MountRef`,
`NodeToken`, `NodeRef` and `StoredLocation` (`vfs_identity.rs`). `midnight-engine::providers`
supplies the backend registry, Browse/Read contracts and adapters over the native
services and captured archive mounts. These Rust contracts are internal;
independent plugins must continue to depend only on the released public API/SDK.
They are not the missing generic filesystem C ABI.

Flat panels list through their own native and archive routes; inline trees use this
registry. `Entry`, selection indexes, settings addresses, watches, mutation capabilities
and application service injection do not go through it (known limit; see the
[migration dependency map](FILESYSTEM_MIGRATION.md)). The native adapter is not a static
fallback for a missing native plugin.

## Identity and restore

- Package IDs are validated ASCII identifiers, independent of captions.
- Host-issued instance/mount/generation counters fail on exhaustion and are never
  persisted. The same opaque node bytes in different mounts cannot be interchanged.
- Node tokens contain at most 256 opaque bytes (`NodeToken::MAX_BYTES`). They are not paths; native adapters
  retain full paths in bounded backend tables. The provider must never reuse a
  token for another node or kind during that mount's lifetime. A token stays
  valid for the mount's lifetime unless the mount's single lister opts in to
  [reclamation](#lifetimes-cancellation-and-budgets): a released token fails with
  `StaleReference`, and a node seen again later may receive a new token. A
  provider that kept its record emits the same token again; the native adapter
  never mints a released value again.
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
paths survive encoding without lossy conversion. Known limit: macOS has no native
address codec of its own, and it does not use the Linux adapter implicitly.

## Lifetimes, cancellation and budgets

Registry methods run on backend workers, including final resource destruction.
Provider calls run outside the registry lock. A blocked provider therefore does
not prevent another registration from resolving a location. The synchronous
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

The bounds are 128 registrations, 64 in-flight/open/retained mounts per registry
(`MAX_PROVIDERS`, `MAX_MOUNTS` in `providers/registry.rs`), 50,000 entries/32 MiB
charged payload per complete list, and 100,000 issued nodes per mount
(`MAX_LIST_ENTRIES`, `MAX_LIST_BYTES`, `MAX_ISSUED_NODES` in `providers/lease.rs`). The
native adapter separately accounts for 32 MiB of raw path storage and 100,000 cached
tokens (`MAX_PATH_STORAGE`, `MAX_NODES` in `providers/native_nodes.rs`). Folder-only listings and presence probes
let the native adapter skip regular files before metadata, name encoding and token
binding, so those files never count; only `list_folders_observed` (below) reads the
metadata of a skipped file whose enumeration lacks it. These are admission/cache
budgets, not an OS path limit or an exact process RSS promise. A failure returns an
explicit error.
Native stored addresses are admitted before codec allocation, using the cache
budget and that codec's worst-case expansion; native encoding checks its raw
charge first. Known limits: per-mount caches are not aligned with a shared application
budget, and the registry's complete-list adapter is not the panels' streaming listing.
Folder-only listings (`list_folders`) use a separate bounded
token-validation ledger, so irrelevant regular files do not consume the
retained-folder quota; a generic provider's default `list_folders` still emits its
complete listing, so every returned token is validated. `has_folders` answers a
disclosure probe at the first folder without issuing references; the tokens it
inspects before that folder are charged to the same 32 MiB token ledger and
one-million-callback ceiling (`MAX_INSPECTED_ENTRIES`), so a generic provider's long file prefix fails as
resource-limited instead of growing the probe's duplicate set. Entries a provider
emits after the stop cannot change the answer. `list_folders_observed` returns the
same rows plus the provider's full watch observation of the directory from that
enumeration: the native adapter adds every entry, files included, with the
metadata a watch probe reads, still without encoding or binding files; other
providers return none. A skipped file costs one metadata read where enumeration
lacks it (the Linux platform filesystem; the installed provider's plugins return
every entry's metadata with the enumeration). After 4,096 such reads
(`OBSERVED_METADATA_READS`) the adapter stops observing and returns the rows without an
observation, so the first watch probe of a branch with more such files requests one
maintenance pass. See [inline provider branches](INLINE_TREES.md) for the backend tree
model, ownership, limits and projection.

Without reclamation neither table evicts: every node a listing or `parent` issues,
and every native record a listing or probe binds, counts until the mount closes.
Reclamation is opt-in per mount and belongs to its single lister. The inline-tree
session is the only one: it resolves its own mount, lists it only through its
branch work and owns which references are live (see
[inline provider branches](INLINE_TREES.md#budgets) for when it reclaims).
`MountedLocation::retain_issued` keeps the references the lister still holds plus
the root, drops every other issued reference, then asks the provider to forget the
rest (`MountedFilesystem::retain_nodes`). The native adapter also forgets records
that failed, stopped or presence-only enumerations bound but never issued, and
reports its record count (`node_count`) so those count toward reclamation. Both
tables return their spare capacity once they shrink below a quarter of it.

A per-lease reclaim gate covers the calls in flight without waiting. Listings,
presence probes and parent lookups hold it shared from their first check through
the provider call and the issue step; reclamation takes it only if it is free and
otherwise skips (`Ok(None)`), so it never releases a reference that a running call
may bind, emit or issue. The gate does not cover a call that has returned: `keep`
must also hold the references of results the lister has not applied yet, and the
tree meets this by not reclaiming while any of its branch work or results is
outstanding. The lease forgets before the provider, so every reference it still
issues stays known to the provider. Providers that keep the default
`retain_nodes`, such as captured archives, keep every record, and a mount whose
owner never reclaims keeps its history up to the caps above. Under the tree, both
tables hold the live model plus at most one reclamation threshold and the listings
since the last settled completion (one listing, or one unpublished recursive
batch), regardless of how long the mount lives.

Known limit: plugin-backed native mounts cannot honour `retain_nodes`, because the
public ABI has no node-release operation. The SDK's native helper keeps up to one
million node records per mount within a 256 MiB cache budget that all mounts of one
plugin instance share (`native/state.rs` in the filesystem plugin SDK), so a churning
tree on such a mount would also exhaust the instance's other mounts.

Listings reject duplicate/self-referencing tokens, contradictory known kinds,
oversized output and cancellation. A failed registry listing issues no partial
new references. A released reference fails with `StaleReference`; listing its
node again issues a reference anew. Parent navigation is delegated to the
provider. The initial location's `root()` is its starting directory; it may have a
provider parent.

## Content and adapters

`MountedLocation::open_content` accepts an issued regular-file node, obtains the
provider's source and uses the `CapturedFile` broker to validate its
captured length/version. The returned source also enforces the broker's bounded
256 KiB exact reads, EOF clamping and before/after revision validation. It retains
the mount owner and checks revocation around calls. Viewer consumers never receive
a fabricated native member path. This is LiveValidated content, not a claim of
an immutable snapshot. Unknown-length/sequential sources need an additive API.
`open_content` is compiled for tests only until a production consumer exists, and
production `NativeProvider::audited` installs no capture, so native content through
this route would be `Unsupported`; F3 viewers, thumbnails and copy use their own
capture routes.
Leased content does not give up a tainted reference yet: `LeasedContent` and
`NativeContent` retain it, and a production consumer first needs a quarantined
give-up for content sources in the plugin host
([ARCH-06](../IMPLEMENTATION_BACKLOG.md#deliberate-later-capabilities-and-current-limits)).

The native adapter delegates directory enumeration to an injected `FileSystem`.
`NativeProvider::new` and production `NativeProvider::audited` are browse-only; the
test seam `with_capture` supplies an explicit capture, and unavailable capture fails
without a built-in fallback.
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
the complete successful callback sequence. The archive worker's restrictions, source
limits and publication and copy-out guarantees apply.

`resolve_target` returns a retained backend context: an exact native path or an
archive directory/member with its full captured raw ancestry. A file's archive
location denotes its immediate parent; a directory's location denotes itself.
It never interprets captions, lists again or extracts data to reconstruct a
target. Invalid/cyclic ancestry and resource exhaustion fail explicitly; links
and special archive members do not gain unsupported actions.

Resolution requires issued references. `MountedTarget` then validates the
registration lifetime and mount identity of the target and of every enclosing tree
context, not current issuance: a resolved target, such as a queued copy source or
a watch target, keeps its value after its lister releases the node references.
Cloning an archive location or mount retains the same leases: list, read, validate
and persistent member identities reject revocation before and after provider
calls, and the container's native locator is withheld once revoked. The leased
mount forwards cooperative reads, the locator and member identities to its source,
so archive previews entered from a tree keep their storage classification and
persistent cache keys. Resources are destroyed before their registrations.
A release that cannot run because its retirement is already quarantined, or an owner
whose resource release failed, gives its references up instead
(`MountedFilesystem::release_quarantined`): a shared reference only gives up its
count, and the last owner passes the archive references it holds on to their own
quarantined release, frees plain memory and keeps its registration and mount permit
retained, so no registration is released before its resources. A target whose archive
source release failed hands its target context down the same way, so the last
reference to a tainted archive session still reaches its host
([engine lifetime](ENGINE_LIFETIME.md#archive-navigation-and-queued-size-work)).
Cancellation and revocation propagate into running listings, which the archive
host ends between pages without stopping the shared worker. Member reads of a
shared archive mount go through `read_cooperative`.
Cancelling a listing or member read never ends the worker that other tabs, viewers
and jobs use: protocol 3 cancels cooperatively, and protocol-1/2 reads finish their
exchange while the host discards the result. A late successful reply cannot publish
after cancellation.
These are routing/lifetime checks, not immutable file identity, authorization
to mutate, or proof that a row is still selected in the current panel snapshot.

## Tests and remaining migration

Tests cover provider isolation, cancellation, late output, exhausted IDs, retained
content and destruction order, malformed responses, unknown providers/codecs,
non-Unicode names, native source replacement detection, archive reads, and
reclamation (released and kept references, the reclaim gate, no release while
branch work or results are outstanding, never-reused native tokens, returned
capacity, and resolved, watch, paste and drop targets after release). See
`providers/*tests.rs`, `tree_reclaim_tests.rs`, `midnight-core/tests/vfs_identity.rs`
and `midnight-core/tests/linked_cancellation.rs`.

Not done yet (VFS-01, TREE-01 and NET-01 in the
[backlog](../IMPLEMENTATION_BACKLOG.md)): flat-panel navigation, projection, selection,
F3 and native capability checks through these identities; every native operation through
equivalent pinned handles; and network discovery as a consumer.
