# Remaining actor ownership inventory

Date: 2026-11-12. Source base `b9f8ee4`, with the current clipboard/desktop/watch
migration present. This is a read-only inventory, not executed failure evidence
or an exhaustive LIFE-01 clearance. No production edits, Cargo commands or tests
were performed for this note. Line numbers identify the inspected source and may
move. The three reviewed migrations do not cover the boundaries below.

## Highest-priority concrete native owners

### Make File List jobs and generated results

`crates/midnight-engine/src/file_list_actor.rs:26` stores Pending containing a
source and FileListDestinationPin. The pin (`file_list_output.rs:10`) owns
`Arc<dyn TransferDirectory>` and `Arc<dyn TransferTarget>`; native directories
hold OS File handles. Generated Text (`file_list_worker.rs:11`) owns Spool, whose
File and pinned root are ordinary fields (`midnight-worker-platform/src/spool.rs:34`).

A completed conflict leaves these values in `Arc<Job>`. Replacing a generation
(`file_list_actor.rs:106`), canceling (`:172`) or normal shutdown (`:182`) can drop
the final completed job directly during engine dispatch. A confirmed Pending is
also moved into an unguarded outer/native task pair (`:140-169`). Generated viewer
or clipboard output (`:212`) can be a completed but unconsumed blocking result,
or an event owned by an aborted async delivery future. These are real native
handles, not just the large immutable FileListCapture display metadata.

The UI already retires stale events and closed sessions through its retired queue
(`midnight-ui/src/presenter/file_list.rs:502,591`); that protection does not cover
an output discarded before delivery. The follow-up must cover stored jobs,
confirmation handoff and generated output ownership, preserving verified staging
and cancellation. Derive capacity from four retained jobs, two active workers,
superseded work and accepted output ownership; do not guess that the job-map size
alone bounds every owner. Useful regressions: conflict then Cancel/replacement,
queued confirmation cancellation, and a completed generated output whose waiter
is destroyed, with gated native pin/Spool destruction and retained admission.

### Dynamically discovered network provider

`network_neighborhood.rs:48-54` receives a newly opened
`Arc<dyn NetworkNeighborhood>` from `spawn_blocking(discover)`, then stores it in
an unguarded Arc<OnceLock>. `discover` (`:154`) constructs NetworkProvider::open.
The concrete client owns an FFI Instance; its Drop invokes the plugin's destroy
callback and then releases or quarantines the module
(`midnight-native-filesystem-host/src/ffi.rs:207`).

That provider is created after EngineServices, so adding only the existing shared
TaskLifetime cannot protect the new discovery result/OnceLock. Cover completed
but unconsumed discovery, unpolled navigation, and final controller/task release
with a bounded owner established before the provider can escape native creation.
Keep the existing ABI failure/quarantine policy and two-request admission. A fake
provider with gated Drop can prove the boundary before native plugin qualification.

### Archive mounts and retained actor mailboxes

Archive navigation accepts ArchiveAction::List with a live mount and returns an
ArchiveListing from the native worker (`archives.rs:142-245`). A stale/rejected
listing can skip `remember_listing` and leave the result as the final mount owner.
The Snapshots guard protects accepted stored roots, not every independent action,
completed result or event delivery. The concrete Mount already delegates child
process wait to its supervisor (`midnight-filesystem-host/src/session.rs:699`);
this is not a claim of UI-side child waiting. Its ordinary fields still contain
channel Files and CapturedFile provider/resources (`session.rs:43` and
`midnight-plugin-host/src/content.rs:49`).

Archive size lanes (`archive_sizes.rs:19,43,73,95`) queue strong Arc<ReadyRoot>
values, which can retain ArchiveLocation mounts. Controller Drop closes lanes and
the normal worker drains them (`archive_sizes/worker.rs:115-136`). A native unwind
before that drain needs a final-Lane-owner regression/guard; the metadata-only
scan algorithm does not make the queued owner metadata-only.

The tree model has an analogous follow-up audit seam: Mailbox holds an optional
BranchCompletion (`tree_controller/mailbox.rs:18,121`), whose result/registration
retain provider resources (`tree_actor.rs:115`). Normal model-worker drain and
Controller Drop cancellation already exist (`tree_controller/worker.rs:140`,
`tree_controller.rs:191`), and late completion into a closed mailbox retires on
the branch worker. Preserve those paths. Test model-worker unwind with an already
queued completion before asserting that every last mailbox release is guarded.
This is a targeted remaining audit, not a claim that all tree shutdown paths fail.

## Service-owner-only boundaries and cancellation

The following tasks still clone original service Arcs without TaskLifetime on
both independently cancellable layers. Their normal result values are metadata;
new native-output guards are not justified merely because they use spawn_blocking.

| Boundary | Concrete source and retained service |
| --- | --- |
| Native listing, space and folder-size tasks | `lib.rs:959,1065,1090`; FileSystem, LocaleService, elevated/fallback services |
| Locale/network settings work | `lib.rs:779,794`; LocaleService and NetworkService |
| Shell menu | `shell_menu.rs:48,51`; ShellMenuService, with native shell objects local to its call |
| Path picker | `path_picker.rs:58`; PathPickerService, returning an optional path |
| Volume discovery/details | `volumes.rs:35,145,193`; FileSystem and LocaleService |
| Explicit volume details | `volume_information.rs:20,28`; FileSystem |
| Folder-presence and entry-size probes | `folder_presence.rs:44`, `entry_sizes.rs:125,131`; FileSystem |

PathPicker also has a concrete cancellation follow-up: it implements explicit
shutdown (`path_picker.rs:95`) but no Controller Drop cancellation. Its started
blocking chooser receives that token and later uses it for blocking result send.
If the controller disappears while a live UI queue is full, dropping JoinSet
cannot abort the started native call/publisher. Test Controller Drop with a full
result queue and a cancellable fake chooser, analogous to the clipboard case.
This note does not claim that every native OS dialog can be forcibly interrupted.

## Cases deliberately not classified as new native-owner gaps

`search.rs` and `comparison.rs` operate on display requests/results and the already
guarded Snapshots alias; the inspected computations do not return new provider
handles. FolderPresenceRequest likewise contains a display TreeRootSnapshot and
IDs. Its retired metadata vectors may deserve allocation-lifetime work, but they
are not equivalent to Watch's mounted ScopeTarget. Archive Catalog discovery
returns package paths/format metadata, unlike NetworkProvider discovery's FFI
instance. Existing cleanup/supervision protections must remain part of each next
boundary review rather than being replaced by blanket task wrapping.
