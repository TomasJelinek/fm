# Filesystem provider migration

This document records which production consumers reach user files through an
installed filesystem provider, which still use the built-in platform services, and the
rules for migrating the rest. The delivery boundaries are VFS-01, TREE-01 and NET-01
in the [backlog](../IMPLEMENTATION_BACKLOG.md); [SECURITY.md](../../SECURITY.md) remains
the operation contract. The internal registry contracts are described in
[the provider registry](PROVIDER_REGISTRY.md).

## Current composition

`midnight-explorer/src/main.rs` creates the engine with
`Engine::with_shared_native_filesystem` over
`midnight_native_filesystem_host::InstalledProvider::shared_composition`, and installs
the other services explicitly:

- **Installed native provider** (the separate Windows/Linux package; see
  [the native API](NATIVE_FILESYSTEM_API.md)): panel roots, listing, metadata,
  capacity and storage hints (Browse/Stat/Storage, with the optional ABI 1.1 volume
  details and conservative SSD hints), folder sizes, F3 content, archive-container
  capture (`native_content::archive`), both comparator inputs including manually chosen
  paths, image, video and custom-icon thumbnail sources
  (`native_content::thumbnail_factory`) and command-icon artwork imported through the
  path picker (`command_artwork::installed`). These routes capture on backend workers,
  require the package and never fall back to built-in file access. Thumbnail
  cancellation reaches provider capture and is checked before cached frames are
  returned; the bounded native reader cache is shared with F3
  ([media capture evidence](../verification/2026-11-08-native-media-capture.md)).
- **Built-in platform services** (`midnight_platform::LocalFileSystem`): Copy and
  Move, Find, checksums and manifest saves (through the transfer service), rename,
  creation, Trash, permanent deletion and attribute edits (`MutationService`),
  watches, desktop actions (activation, Properties, the shell menu, the terminal),
  custom commands, the editor, drag and drop, the native clipboard, settings, the
  native Open/Save pickers and the navigation fallback. Directory comparison reads
  through the same built-in Find service.
- **Network filesystem plugin**: Network Neighborhood discovery and activation
  ([network contract](NETWORK_FILESYSTEM.md)).

Engine and UI code reach these services only through `midnight-core` traits; display
paths never authorize a provider call.

## Archive containers

Archive navigation, saved archive tabs, selection, F3 member content, thumbnails of
archive members (for providers with stable member IDs), verified copy-out and
archives inside archives (session only) exist. Archive writes and generic remote
content do not.

- New mounts and saved-tab restoration capture the container through the explicit
  `ArchiveCapture` callback. `Mount::open_captured` keeps the original native locator
  separate from its pathless content lease; only the lease authorizes reads, and the
  worker never receives a native path. Production navigation has no direct opener and
  no missing-provider fallback
  ([archive capture record](../verification/2026-11-09-archive-native-capture.md)).
- `InstalledProvider::capture_matching` accepts stored v1 metadata that matches in
  everything but the change time (VIEW-05, see
  [content reads](CONTENT_READS.md#change-time-drift-view-05)) under the current or a
  supported legacy platform prefix. A successful restoration stores the current token;
  changed, malformed, foreign or unknown tokens fail closed.
- Archives inside archives open from staged member leases of their enclosing mount
  (`Mount::open_nested`).
- `ArchiveAddress` stays backed by the native container path, so it is not a generic
  stored location yet.

The worker protocol, limits and cancellation are described in the
[filesystem API](FILESYSTEM_API.md) and the public
[`filesystem-api/PROTOCOL.md`](../../filesystem-api/PROTOCOL.md). The viewer C ABI
reserves `KIND_FILESYSTEM`, but the viewer SDK exposes only the viewer request family;
the native Browse/Read API, SDK, header and per-OS packages live separately under
`filesystem-api/` and `plugins/midnight-filesystem-native-*`.

## Remaining consumers and their migration boundary

Paths are relative to `crates/`.

| Consumer | Route today | Required migration boundary |
| --- | --- | --- |
| Sort, prefix search, filtering, filename colors | Captured entries and backend-prepared keys (`core/entry.rs`; engine `search.rs`, `filters.rs`, `filename_colors.rs`); no file access | Preserve immutable source/projection revisions, raw names, metadata availability, locale keys and cancellation |
| Selection and action capture | Native paths (`core/selection.rs`, `DirectoryIndex.by_path`; engine `selection.rs`, `file_targets.rs`); archive selection by member ID; tree occurrences | Provider-aware node index and captured targets; keep row IDs separate from node identity and the independent selection revision |
| Watches and refresh | Built-in `WatchService` (`engine/watch.rs`) | Mount-scoped subscription ownership, overflow/rescan, revocation and an explicit unsupported/poll policy |
| Location aliases and type/drive icons | Built-in desktop services (`locations()`, `load_icon`) | Provider-scoped location tokens and icon cache domains; no provider names passed to native file APIs |
| Copy, Move, recovery | Built-in `TransferService` (`engine/copy_*`, `move_*`; OS `transfer*`) | Provider-scoped prepared source/destination capabilities with equivalent revalidation, readback, publication and recovery; Read alone grants no source removal |
| Rename, create, Trash, permanent delete, attribute edits | Built-in `MutationService` (`engine/operations.rs`, `operation_worker.rs`, `operation_metadata.rs`) | Preparation before confirmation, pinned target/parent identity, no-replace and distinct Trash/permanent semantics; never reuse read-only size traversal for mutation |
| Recursive Find, directory comparison | Built-in `FindService` (`core/find.rs`, `engine/find*`, `engine/comparison`) | Optional provider enumeration/content capability, explicit unsupported limits and bounded traversal; result actions validate the original provider and version |
| Checksums and manifest save | Built-in `TransferService` (`engine/checksum_worker.rs`, `checksum_manifest.rs`) | Scoped readable captures and separately authorized destination publication; a manifest never inherits write authority from a readable source |
| Activation, Properties, shell menu, editor, terminal, custom commands | Built-in desktop, shell-menu, editor and user-command services | A named, version-bound native-location bridge per host action; no automatic extraction, guessed working directory, shell fallback or execution on listing or hover |
| Clipboard and drag and drop | Built-in clipboard and drop services (`engine/clipboard.rs`, `drag_drop*.rs`) | Explicit native export/import capabilities; a generic node or label never becomes an OS file-list entry or a Move removal grant |
| Tabs, history and bookmarks | Native `PathBuf` plus archive addresses (`core/preferences.rs`) | Versioned stored provider locations, unavailable restore and parent/return context; runtime registry IDs never enter settings |

Host bootstrap is separate authority: preferences files and atomic saves, plugin
discovery and loading, fonts, locale and window integration, diagnostic snapshots and
explicitly configured program launches are not panel Browse operations. They do not
justify a hidden native fallback for provider-owned user files, and Options and plugin
discovery keep working when the native filesystem package is missing.

## Rules for the remaining migration

1. **Identity and authority differ.** Scope opaque nodes by live provider instance,
   mount and generation. A provider package ID survives restart; a runtime handle
   does not. A node ID is neither a transfer object identity nor consent to mutate.
   Include an identity domain before comparing hardlinks, ancestors or cycle keys.
2. **Never infer native authority from the absence of archive state.** The UI's
   `Pane::native_capabilities()` requires no archive and no network location (and no
   request for either), and `Entry::native_path()` returns a path only when neither
   `virtual_node` nor `network_node` is set. Identity and duplicate checks call
   `native_path()` rather than test for an archive node. Empty `PathBuf`s, labels and
   URI-looking strings are never compatibility keys or native targets.
3. **Selection survives projection without retargeting.** Capture the provider
   location, navigation/view and independent selection revisions. Reused row numbers,
   filtering, refresh, sorting and later tree occurrences cannot substitute targets. A
   nonempty selection whose nodes disappeared fails and never activates the focus
   fallback. Navigation/context-only rows are never operation targets.
4. **Browse defines completion.** Page and continuation revisions, partial versus
   final results, missing metadata, root/parent semantics and live versus snapshot
   listings are explicit. No host separator, case folding or lexical `..` is assumed of
   an opaque provider address. Pages, retained bytes, queued work and IDs are bounded.
5. **Read needs a retained versioned lease.** `ContentSource` requires a known length
   and exact-length `read_at` replies; unknown-length, sequential or replay sources need
   an explicit extension, never full-file buffering or a fake zero length.
   `LiveValidated` is not an immutable snapshot.
6. **Removal and revocation are distinct.** Unregistering prevents new requests; it is
   explicit whether existing leases finish or are revoked. Closing a pane never
   destroys a legitimate viewer or copy lease, a revoked lease never reads from a newly
   registered instance with recycled IDs, and workers, permits and library lifetimes
   last until work actually ends, with late output and heavy destructors retired on
   backend workers.
7. **Native safety survives extraction.** Lossless UTF-16/Unix bytes, long paths,
   explicit link navigation, no-follow destructive recursion, native locale metadata,
   watcher overflow and the verified Copy/Move contracts stay. A generic `writable`
   flag never replaces prepared handles, independent destination readback or recovery
   proofs.
8. **Persist addresses, not sessions.** Provider codecs are versioned and bounded;
   malformed known codecs are refused, well-formed unavailable providers are kept
   without silently navigating elsewhere, and legacy native bytes and archive parent
   anchors are preserved.

Fakes cannot prove OS no-follow behaviour, raw names, Trash, change times, publication
or recovery; the native and archive regression tests stay required when each adapter is
connected.

## Reference

Salamander's separate disk, archive and plugin-filesystem panels, its plugin
filesystem interface (open/close/disconnect, path conversion, contextual supported
services) and its Network plugin, which presents discovery nodes without pretending to
support file reads and writes, are a behavioural reference only. Its pointer
identities, ANSI/MAX_PATH buffers and synchronous UI filesystem work are not carried
over, and no reference code is copied.
