# Filesystem migration inventory

Read-only M0 source review, 2026-11-07. This records the production composition
before the internal M1 registry is connected. Concurrent new `vfs_*` and
`engine/providers/` modules do not by themselves change that composition. No
Cargo, application launch, filesystem mutation, network discovery or performance
measurement was run for this inventory.

The migration sequence is in
[`_build/goal/vfs/integration-plan.md`](../../_build/goal/vfs/integration-plan.md).
The durable delivery boundaries are [VFS-01, TREE-01 and NET-01](../IMPLEMENTATION_BACKLOG.md).
[SECURITY.md](../../SECURITY.md) remains the operation contract.

## Current authority and dependency map

[`midnight-explorer/src/main.rs`](../../crates/midnight-explorer/src/main.rs)
constructs one `LocalFileSystem`. The
[`midnight-platform` facade](../../crates/midnight-platform/src/lib.rs) selects
built-in `WindowsFileSystem` or `LinuxFileSystem`; their listing, metadata, storage,
watch, mutation, transfer and host-action implementations live in separate OS
crates. `Engine.filesystem` is an `Arc<dyn FileSystem>` aggregate of directory,
volume and storage traits. The same concrete native object is also injected into
independent services. Replacing only `Engine.filesystem` cannot remove native
user-file access.

Paths in the following table are relative to `crates/`. Entries describe current
production consumers, rather than every test fixture that calls `std::fs`.

| Consumer | Current route and authoritative data | Required migration boundary |
| --- | --- | --- |
| Roots, listing, metadata and capacity | `core/provider.rs`; `engine/lib.rs::list_directory`, `volumes.rs`; native `read_directory`, `entry_metadata`, `list_volumes`, `storage_kind`, `space_info` take `Path` | Provider roots and node-scoped Browse/Stat/Storage, with explicit unavailable and unsupported results |
| Sort, prefix search, filtering, filename colors | `core/entry.rs`, sorting; engine `search.rs`, `filters.rs`, `filename_colors.rs` consume captured entries and backend-prepared keys | Preserve immutable source/projection revisions, raw names, metadata availability, locale keys and cancellation; these policies need no authority to reopen a native path |
| Selection and action capture | `core/selection.rs`, `DirectoryIndex.by_path`; engine `selection.rs`, `file_targets.rs`, `selection_sizes.rs`; archive selection is a separate ID-based path | Provider-aware node index/origin and captured targets; keep row IDs separate from node identity and independent selection revision |
| Sizes and scheduling | `engine/folder_sizes.rs`, `folder_sizes/batched.rs`, `entry_sizes.rs`; `SharedSizes` keys are native `PathBuf`; native directory/metadata calls | Node/revision keys, bounded recursive enumeration and provider concurrency groups; preserve partial estimates and conservative unknown/network scheduling |
| Watches and refresh | `core/watch.rs`, `engine/watch.rs`; native path subscription plus debounced hints/polling through `FileSystem` | Mount-scoped subscription ownership, overflow/rescan, revocation and explicit unsupported/poll policy |
| Location aliases and type/drive icons | `core/desktop.rs`, `engine/desktop.rs`; native `locations()` and `load_icon`, extension/type keys and optional raw drive root | Provider-scoped location tokens and icon cache domains; retain safe host type-icon fallbacks without passing provider names to native file APIs |
| F3 native content | app `viewers.rs` creates its own viewer service; `viewer-engine/capture.rs` calls `plugin-host::CapturedFile::capture(path)` | Inject provider content capture into the viewer path; listing migration alone does not affect this native opener |
| F3 archive content | `viewer-engine/capture_archive.rs` wraps `ArchiveMount` in `ContentSource`, then `CapturedFile::from_source` | Preserve this working lease route; generalize ownership/version validation rather than reconstructing a member path |
| File comparison | app `comparator.rs`; `compare-engine/worker.rs` directly captures both native paths; text/image workers use those captures | Provider-scoped independent content leases. This differs from the engine's two-pane metadata/selection comparison |
| Thumbnails and command icons | app `thumbnails.rs` supplies `thumbnail-host::service::native_capture`; host service supports a capture closure, but current panel target construction requires native capability | Route selected provider sources and versioned cache keys through the existing bounded capture seam; keep explicit configured native icon paths distinct from panel-node authority |
| Archive mount and listing | `engine/archives.rs` discovers installed packages; `filesystem-host/session.rs::Mount::open` captures a native container, validates it and supervises a restricted worker | Preserve archive worker protocol 1. Generic/nested mount sources need parent content leases and aggregate nesting/decompression budgets |
| Archive copy-out | `engine/archive_transfer.rs` re-lists captured IDs and checks mount, parent, raw name, kind, length and readability; adapts members to `TransferSource` | Preserve this completed verified-copy source route and native destination guarantees; no arbitrary display-to-path conversion |
| Copy, Move, recovery | separately injected `TransferService`; `engine/copy_*`, `move_*`; `core/transfer*.rs`; OS `transfer*` retain native handles/proofs/stages | Provider-scoped prepared source/destination capabilities with equivalent revalidation, readback, publication and recovery; Read alone grants no source removal |
| Rename, create, Trash, permanent delete and metadata editing | separately injected `MutationService`; `engine/operations.rs`, `operation_worker.rs`, `operation_metadata.rs`; OS mutation modules | Preserve preparation before confirmation, pinned target/parent identity, no-replace and distinct Trash/permanent semantics; do not reuse read-only size traversal for mutation |
| Recursive Find | separately injected `FindService`; `core/find.rs` roots/results use native paths and revision stamps; `engine/find*` revalidate results | Optional provider enumeration/content capability, explicit unsupported limits and bounded traversal; result actions must validate the original provider/version |
| Checksums and manifest save | `engine/checksum_worker.rs` opens captured targets through `TransferService`; `checksum_manifest.rs` uses prepared native destination/staging | Scoped readable captures and independently authorized destination publication; manifest output cannot inherit write authority from a readable source |
| Enter/association, Properties, shell menu, editor and terminal | `DesktopServices`, `ShellMenuService`, `EditorService`, native user-command preparation/runtime, plus native metadata checks | A named, version-bound native-location bridge for each host action; no automatic extraction, guessed cwd, shell fallback or execution on listing/hover |
| Clipboard and native drag/drop | `engine/clipboard.rs`, `drag_drop*.rs`, OS clipboard/drop services; captured native paths and Copy/Cut intent | Export/import native capabilities explicitly. A generic node/label cannot become an OS file-list entry or a Move removal grant |
| Tabs, history and bookmarks | `core/preferences.rs::PaneTabPreferences.paths` plus separate `archives`; bookmarks contain native `PathBuf`; archive addresses contain native container, source version and raw member components | Versioned stored provider locations, unavailable restore and parent/return context. Runtime registry IDs must never enter settings |

Host bootstrap remains a separate authority: preferences files and atomic saves,
trusted installed plugin discovery/loading, fonts/locale/window integration,
diagnostic snapshots and explicit configured program launch are not panel Browse
operations. Their existence does not justify a hidden host-native fallback for
provider-owned user files. Conversely, native migration should not break Options
or plugin discovery when the native filesystem package is missing.

## Archive support is implemented; generic native plugins are not

The public [`filesystem-api/PROTOCOL.md`](../../filesystem-api/PROTOCOL.md) defines
the working read-only archive worker protocol: Open with an advertised format and
captured length, List, Read, brokered SourceRead and Close. Names are raw byte
blobs, IDs belong to the mount, pagination preserves duplicate/case-distinct names,
and the worker receives no native path. Current host limits include a 64 MiB
container, two active mounts and a 20-second request deadline. Cancellation/deadline
terminates that worker and invalidates the mount. Do not promise that an individual
cancelled archive request leaves its mount usable.

Archive navigation, saved/unavailable archive tabs, selection, F3 member content and
verified copy-out exist. Nested archives, archive writes, panel provider thumbnails
and general remote content are separate gaps; these do not erase the delivered
archive functionality. The existing `ArchiveAddress` remains native-container
backed, so it is not yet a universal stored location.

The viewer C ABI reserves `KIND_FILESYSTEM`, but its messages/SDK currently expose
the viewer request family and `export_viewer!`. That constant is not an independent
native filesystem API. Neither the archive worker's `Open.format` nor an `Archive`
field on `Entry` provides generic Resolve/Stat/Watch/native actions/mutations.
Existing viewers and archive workers must remain compatible while a new public
filesystem service family, SDK, headers and independent conformance package are
introduced.

## Invariants and contracts still required

1. **Identity and authority differ.** Scope opaque nodes by live provider instance,
   mount and generation. A provider package ID survives restart; a runtime handle
   does not. A node ID is neither a transfer object identity nor consent to mutate.
   Include an identity domain before comparing hardlinks, ancestors or cycle keys.
2. **Never infer native authority from absence of archive state.** Today
   `Pane::native_capabilities()` means `archive.is_none() && !archive_request`, and
   `Entry::native_path()` checks only `virtual_node.is_none()`. Those checks are
   valid for the current two-way composition, but must change before a third
   provider reaches those paths. Empty `PathBuf`, labels and URI-looking strings
   cannot be compatibility keys or native targets.
3. **Selection survives projection without retargeting.** Capture provider location,
   navigation/view and independent selection revisions. Reused row numbers,
   filtering, refresh, sorting and later tree occurrences cannot substitute targets.
   A nonempty selection whose nodes disappeared must fail, never activate focus
   fallback. Navigation/context-only rows remain ineligible operation targets.
4. **Browse must define completion.** Specify page/continuation revision, partial vs
   final results, missing metadata, root/parent semantics and live vs snapshot
   listings. Do not require host separator, case folding or lexical `..` semantics
   of an opaque provider address. Bound pages, retained bytes, queued work and IDs.
5. **Read needs a retained versioned lease.** `ContentSource` already enables
   non-native content, but currently requires known length and exact-length
   `read_at` replies. Unknown-length/sequential/replay sources require an explicit
   extension; no full-file buffering or fake zero length. Define source change,
   independent readers, EOF, integrity, cancellation and backend close ownership.
   `LiveValidated` is not an immutable snapshot.
6. **Removal and revocation are distinct.** Prevent new requests after unregister;
   decide explicitly whether existing leases may finish or are revoked. A pane
   closing must not destroy a legitimate viewer/copy lease. A revoked lease must
   not read bytes from a newly registered instance with recycled IDs. Keep workers,
   resource permits and dynamic library/worker lifetime until work actually ends;
   retire late output and heavy destructors on backend workers.
7. **Preserve native safety through extraction.** Keep lossless UTF-16/Unix bytes,
   long paths, explicit link navigation, no-follow destructive recursion, native
   locale metadata, watcher overflow and verified Copy/Move contracts. Optional
   capabilities may fail explicitly; a generic `writable` boolean cannot replace
   prepared handles, independent completed-destination readback or recovery proofs.
8. **Persist addresses, not sessions.** Validate provider-specific codec version and
   bounded bytes; reject malformed known codecs, retain well-formed unavailable
   providers without silently navigating elsewhere. Preserve legacy native bytes
   and archive parent anchors. Allocate a real settings version with the current
   settings owner; FILTER's version reservation is separate.

## Reviewable first Browse/Read tests

Keep identity registry tests and the first end-to-end provider slice as separate
checkpoints. Pure registry GREEN does not demonstrate navigation, cancellation or
content routing. Proposed tests use in-memory providers and native-service spies
that fail on any call; no host filesystem access or new dependency is necessary.

| Test | Smallest fixture and observable assertion |
| --- | --- |
| Owner and generation | Two packages/instances use identical node bytes and labels. Foreign mount access, stale unregister/re-register handles and exhausted ID allocation fail before provider invocation; fresh owner still works |
| Provider address semantics | Root/parent and children include case-distinct, duplicate-display and non-host-separator names. Resolve/list/select use opaque identity; stored unavailable restore preserves address and anchor without choosing a native directory |
| One complete Browse → selection → F3 path | Fake root lists two files in bounded pages; capture the selected node, reorder/rename display labels, open content through the captured provider lease and assert exact bytes. All native-spy counters remain zero |
| Stale selection and continuation | Refresh recycles a row ID and invalidates the old continuation; neither stale page nor selected old node can become the newly focused file. Explicit stale/source-changed result, no substituted read |
| Read isolation and validation | Two readers interleave offsets over one source. Change the source version between reads; old lease fails, new lease succeeds. Wrong-owner read/close handles never reach either provider |
| Cancellation and fairness | Block provider A inside a controlled cancellable request, then complete pane B's listing/read. Cancel A, deliver late output and reject a saturated queue; no revival, no permit release before actual return, accepted requests receive one terminal outcome |
| Lifetime and optional capability | Close A's pane while its viewer lease remains; exercise the declared finish/revoke policy. Watch/native action/write are Unsupported, including after provider disappearance, with no native/temp fallback |

The first integration can use known-length `ReadAt` only and explicitly reject
other source kinds. It should reuse the actual viewer content bridge rather than
asserting only that a fake trait returned bytes. Existing native/archive operation
regressions remain required when their adapters are connected; fakes cannot prove
OS no-follow, raw-name, Trash, ChangeTime, publication or recovery behavior.

## Plan review and Salamander reference

The proposed M1 identity/storage registry is a useful independent first checkpoint.
Its remaining M1 items (Entry/index/selection migration and a complete two-pane
navigation-to-F3 slice) are substantially larger; report those separately so an
unwired registry does not imply production VFS completion. Preserve explicit
service ownership in the composition rather than assuming a registry lookup makes
all native consumers provider-aware. `FsError.path`, `Volume.root`, storage/cache
keys and saved bookmarks also remain path-specific contracts, even after Entry
gains a node field. Version-scoped capability availability must be rechecked at
preparation/dispatch, not cached permanently from package discovery.

The local Salamander source is a behavioral reference, not a compatible API:

- `D:/experiments/salamander-main/src/fileswnd.h` defines separate `ptDisk`,
  `ptZIPArchive` and `ptPluginFS` panels. It is evidence for working provider UI
  behavior, not evidence that its native disk access is itself plugin based.
- `src/plugins/shared/spl_fs.h::CPluginInterfaceForFSAbstract` separates OpenFS,
  CloseFS, DisconnectFS and internal/external path conversion. Its
  `CPluginFSInterfaceAbstract` supplies ChangePath/ListCurrentPath,
  TryCloseOrDetach, ReleaseObject and optional operation families.
  GetSupportedServices is refreshed after path/list changes; capability
  availability is contextual rather than a static native/archive distinction.
- `src/plugins.h::CPluginFSInterfaceEncapsulation` wraps plugin entry/leave and
  warns against direct callbacks that can refresh a panel while its data are in
  use. Carry forward guarded dispatch/lifetime discipline; do not carry over
  pointer identities, ANSI/MAX_PATH buffers or synchronous UI filesystem work.
- `src/plugins/nethood/nethoodfs2.cpp::GetSupportedServices` advertises context
  menu/icon/path presentation, rather than pretending discovery nodes support
  ordinary file reads/writes. Network namespace presence is not a native path,
  transport implementation or authentication grant.

Remaining M0 evidence is operational: record current native Windows/Linux targeted
test baselines and first-viewport/cancel/RSS measurements before provider cutover.
This source inventory adds no new runtime or performance pass claim.
