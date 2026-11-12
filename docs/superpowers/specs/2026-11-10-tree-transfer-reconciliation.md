# Native tree reconciliation after transfers

Read-only design review, 10 November 2026. No production edits or builds.

## Decision

Implement a native-only, in-session maintenance sweep that refreshes the hidden
root and every previously loaded native directory branch, parent before child,
using the existing single provider-work lane. Trigger it after terminal transfer
events for both currently adopted native panes. Preserve the session, original
admission root snapshot, filters/sort/search, surviving occurrences, expansion,
and acknowledged selection.

Do not add successful-root pruning or operation-specific invalidation lists.
Actual fresh listings reconcile successful roots, partially moved subtrees,
cancelled/restored sources, destination additions, and unrelated changes observed
at the same time. The current selection never determines the sweep scope.

The guarantee is that a successfully finished sweep has removed every absent
entry observed in its root/loaded-branch listings. It is a live listing, not a
filesystem snapshot or watch service. An inaccessible/provider-failed branch
must produce an explicit refresh failure and retain its last known rows; an error
must not be reported as successful reconciliation or an empty directory. A later
external mutation can make any listing stale again.

## Why this is smaller than receipt pruning

Receipt pruning would require a new node-removal command plus a separately retained
list of original Move roots and a second mechanism for partial directory changes.
`RootCompleted` excludes failed/cancelled roots, and `source_directories` reserves
selected-root parents in a 64-path list, so it is not a complete changed-descendant
set. It also does not reconcile a destination or another pane observing ordinary
Move. Pruning only successful roots is therefore insufficient.

The tree already has the correct reconciliation algorithm:

- `DirectoryTree::install` retains surviving `BranchId` values by `NodeRef`,
  keeps their expanded descendants, and cancels/removes absent subtrees.
- `ProjectionState::build` retains surviving `TreeRowId` values and drops records
  for removed model nodes.
- `Selection::rebased` preserves surviving marked occurrences, and the presenter
  already accepts structural publications and replays surviving exact toggles.

The missing pieces are native root relisting, a fresh backend-owned root action
snapshot, bounded sweep scheduling, a correlated completion/rejection response,
and preventing automatic whole-pane refresh from replacing the adopted tree.

## Contract

Add one read-only request in `midnight-core/src/tree_protocol.rs`:

```rust
pub struct TreeRefreshRequest {
    pub view: Arc<TreeProjection>,
    pub request: TreeRequestId,
}

TreeCommand::RefreshLoaded(TreeRefreshRequest)
```

This requests refresh of the named native session's current retained loaded
branches. It grants no filesystem mutation, content-open, or path authority.
Admission requires the exact acknowledged current view and a fresh request ID.
Archive sessions return Unsupported. No native paths, row numbers, or selection
are accepted as refresh targets.

Use correlated events:

```rust
TreeEvent::Refreshed {
    stamp: TreeViewStamp, // accepted request's original stamp
    request: TreeRequestId,
    result: Result<(), TreeDiagnostic>,
}
TreeEvent::RefreshRejected {
    stamp: TreeViewStamp,
    request: TreeRequestId,
    error: TreeDiagnostic,
    retryable: bool,
}
```

`Refreshed` is sent after every sweep publication has been installed and emitted,
including provider failures; it never means merely queued. Reject busy/stale-view
admission with `retryable: true`; unsupported/session exhaustion/invalid input is
not automatically retried. Navigation cancellation discards the old session and
its sweep rather than publishing into the new scope. All diagnostics remain
bounded. Existing `Published` events carry intermediate/final projection changes.

Presenter response correlation uses the saved request stamp/request plus the
current navigation/session, not equality with the latest revision: the sweep
itself advances view revisions. A queue rejection against a retired revision
still clears the matching in-flight request and retries against the current Arc.

## Native root relisting without replacing the admission snapshot

`providers/root_seed.rs` currently consumes its root rows once and returns
StaleReference on every subsequent root list. Add a private explicit policy such
as `RootReadPolicy::{CapturedOnce, NativeRefresh}` to `wrap`. Native capture opts
into `NativeRefresh`; archive capture remains `CapturedOnce`.

The first root list must still consume the exact already accepted snapshot,
without new enumeration. Only subsequent native root lists delegate to the
existing injected `NativeMount::list(root, ...)`. This uses the installed native
filesystem service, raw token-bound path, normal listing limits, cancellation,
and existing node binding. It is not a fallback to the host transfer service.

Keep `TreeProjection.root` and its original entries/index Arcs unchanged. They
identify admission, not a permanently current directory listing. Replacing them
would break `TreeSelectionSnapshot::rebase`, presenter `publish_tree`, options
correlation, ReadyRoot capture, and root-alias viewer validation. Live rows can
already differ from this admission snapshot; `TreeProjection::new` permits new
rows with `original_root_entry: None`.

Add a separate immutable `TreeProjection.root_actions: TreeRootSnapshot` for
current root-file action authority. On initial construction and archive sessions
it shares the admission root's entries/index Arcs. Native maintenance builds a
fresh action snapshot on the model worker from the retained native root nodes:
resolve each current top-level node through `MountedTarget::value()`, require
`ProviderTarget::Native`, and construct its exact raw native Entry and index.
Nothing in the UI reconstructs operation paths from displayed text. This is the
same live native pathname authority as a normal accepted root listing; it is not
a destructive identity pin or a new provider capability.

Keep stable root alias IDs in ProjectionState records, initially the accepted
root EntryIds; allocate new nonzero collision-free IDs for new root occurrences
using a checked monotonic allocator. Reusing a removed alias is forbidden.
The existing `original_root_entry` field can keep its name for this slice, but
its documented meaning becomes the captured root-action entry alias. Its
validation uses `root_actions`, not the admission root. A path observed absent
loses its old occurrence; surviving native tokens continue to represent raw
path + entry kind, not inode identity, as today.

Rebuild the action snapshot only when the root listing is reconciled, not for
every loading/selection publication. Validate its pane/navigation, native raw
paths, kinds, unique IDs, parent location, and payload/index bounds. Charge the
additional retained snapshot within explicit existing model/projection budgets;
initial shared seed allocations need not be duplicated. If materialization fails,
reject that proposed publication rather than publishing rows with missing action
authority. Build candidate projection/action snapshot/selection together before
replacing the accepted publication, using the existing transactional projection
pattern for reprojection.

Update all root-alias action consumers as part of the same change:

- Presenter `viewer_launch` captures `root_actions.entries/index`.
- Viewer worker `capture_tree::normalize` validates those exact Arcs and resolves
  each root alias against `root_actions`; existing foreign/descendant checks stay.
- Presenter Enter/double-click root-file routing resolves aliases against
  `root_actions`; newly discovered native root files keep ordinary F3/Enter.
- `clear_tree` must not reinterpret a new action alias against the old flat index:
  restore a flat caret only when the exact native path exists in the retained
  admission index, otherwise use the existing navigation fallback.
- Initial `Selection::from_root` still uses the admission snapshot, before the
  first maintenance. Existing seed selection and ReadyRoot authority are unchanged.

Do not skip newly discovered files, fabricate seed aliases, or disable their F3/
Enter functionality. Without this separate action snapshot, native root relisting
would expose rows whose required root-file actions regress. The second snapshot
is smaller than replacing the admission root across every selection/presenter/
options/provider contract, and it retains genuine backend native-node authority.

## Bounded model scheduling

In `providers/tree.rs`, expose a worker-only snapshot of loaded native directory
branch IDs in ancestor order, including the hidden root. Add a small `listed`
fact to `Node` if necessary: set it only after successful `install`, retain it on
collapse, reset it when attaching/replacing a container. A mere disclosure probe
must not cause maintenance to enumerate an unopened branch. Exclude links,
special entries, attached containers and their foreign-provider descendants.
The snapshot contains at most the existing 10,000 model nodes; no provider calls
are made while constructing it.

In `tree_actor.rs`, retain one maintenance state:

```text
original request stamp/id
bounded VecDeque<BranchId>, hidden root first
bounded first diagnostic + failure count
```

Snapshot candidates at acceptance. Before issuing each item, verify the branch
still exists, still belongs to the native root context, and was previously
loaded. Skip descendants removed by an earlier parent refresh. Process only one
candidate at a time through existing BranchJob/Completion and WorkerBudget;
do not enqueue all candidates in the eight-command branch queue. Keep explicit
user branch work and maintenance fair; speculative probes have lower priority.
Newly expanded branches list normally and do not require recursive enumeration
of unknown children by the sweep.

The hidden root is an internal BranchId, not `TreeRowId(0)` granted through the
ordinary visible-row branch API. Add an internal pending-work origin enum
(`User` versus `Maintenance`) so root work need not pretend to be a visible row
and its errors do not retire the whole Session through `fail_session`.

Apply each result with existing installation/projection/selection logic. A failed
parent retains previous rows and skips its descendants for this pass, with an
explicit failure result. Continue independent branches so one failure does not
leave every other pane subtree stale. Root failure stops the pass because its
membership is unknown. Invalidate folder-size snapshots/sort values once when
maintenance is admitted, using the existing Refresh invalidation path.

Do not reuse `tree.rows()` as the complete candidate set: collapsed retained
branches are omitted there. The model-owned loaded-state snapshot is the authority.
All sweeping, sorting, filtering, allocation, and reclamation stay on workers.

## Exclude the application's temporary Move namespace

Do not run an ungated sweep on `RootCompleted`. Move can subsequently stage and
restore another source; a listing during that window could temporarily omit a
failed source and permanently discard its mark on selection rebase.

Terminal progress is already emitted after cleanup and mutation-gate release in
`copy_operations::run`, but a later queued job can start before reconciliation.
Pass the existing shared mutation gate into the tree controller/worker and take
it for each maintenance list through installation of that result. Carry its
permit through BranchJob/Completion; release it after model/projection state is
installed and before outbox backpressure. No new mutation capability is created.

Acquire the gate before the native listing/branch permit. If either is unavailable,
hold neither while waiting; use the worker's existing mailbox-aware semaphore
wait pattern. Extend that wait to the actual unavailable gate instead of spinning
on an already-free listing semaphore. Cancellation/navigation cannot release a
permit while its native call is still running. Per-step gating avoids monopolizing
the mutation lane for a sweep of many branches while ensuring no step observes
the application's temporary source staging. Maintain the existing one-provider-
call-per-pane limit.

## Presenter trigger and lifecycle

Add a small per-pane state (e.g. `tree_refresh.rs`): session key, desired epoch,
one sent request/epoch, and last failure. Terminal Copy or Move events mark both
currently adopted **native** trees dirty, regardless of terminal outcome. This
deliberately avoids interpreting incomplete path samples or inferring the
operation's original selection. It covers source/destination panes, nested paths,
ordinary clipboard Move, partial errors, cancellation, and completed writes.
At most two native sessions are refreshed; repeated terminal events coalesce.

For native adopted trees, replace the existing path-matched
`operations.refresh` action in `copies.rs` with this maintenance state, for both
source and destination. Flat panes retain ordinary refresh. Archive panes retain
their existing behavior and never gain native refresh.

In `flush_trees`, after Commit/replayed selection/options have settled and after
queued successful-root Remove edits have settled, send one RefreshLoaded request
using the latest acknowledged Arc. Outer send refusal leaves desired work intact.
A retryable backend rejection retains desired work for a later input pump; do
not spin in the same callback. An accepted request remains in flight until its
correlated terminal event, not until the first Loading publication.

If another transfer ends while a sweep is running, keep the latest desired epoch
and run one additional pass after completion. Clear only the epoch completed by
that request. On persistent provider failure, show the diagnostic and stop
automatic retry for that epoch; a later terminal event or explicit refresh can
request a fresh pass. Clear all state on navigation/session retirement/close.
Never apply an old completion or rejection to a new session with reused row IDs.

Keep progressive RootCompleted deselection unchanged. Structural refresh is
asynchronous after transfer cleanup; it does not move the caret to another source
or erase marks on surviving failed/unrelated occurrences. Existing caret fallback
to a surviving ancestor is appropriate when its actual row disappears.

## Scope, races, and qualification

This implements native maintenance of an adopted session, not flat navigation
refresh, archive root reload, watchers, recursive discovery, or arbitrary
descendant-file activation. It reuses existing native listing semantics and does not
promise inode snapshots, atomic whole-tree views, or protection against external
same-path replacement. If an external entry genuinely disappears during a sweep,
its mark disappears with the row; preserving a nonexistent row is not required.

Tests must establish:

1. Top-level file/directory and nested directory Moves disappear after maintenance;
   failed/skipped siblings remain marked; original session and expansions survive.
2. Cancellation/error after completed child removals updates every previously
   loaded surviving descendant, including collapsed cached branches and filters.
3. Destination native tree shows additions; ordinary Move/Copy terminal events use
   the same path; new rows cannot reuse removed occurrence IDs or fabricated aliases.
4. New and surviving native root files both support F3/Enter after maintenance;
   viewer normalization rejects a forged/foreign action snapshot and preserves
   captured source order across later maintenance. Archive root remains one-shot.
5. A fake Move paused between source staging and restore blocks maintenance listing;
   after failed Move cleanup, the restored row and mark survive. A second queued
   job cannot be observed mid-staging. Gate/listing waits do not busy-poll or block UI.
6. Outer command saturation, stale-view backend rejection, intermediate Loading,
   and a second terminal event during the pass all retain exactly the required
   pending refresh. Navigation/close retires work and old events safely.
7. Large loaded trees stream through one provider slot without filling the eight-
   request queue, unopened branches are not enumerated, and parent removal skips
   queued descendant work rather than failing the Session.
8. Root/branch provider errors report failure and preserve last-known marks/rows;
   no false empty/success result, unbounded retry, or hidden-root silent error.

## Implementation order and files

1. Backend tests for native second root listing and session-preserving removal;
   implement private policy in `providers/root_seed.rs` and call sites in
   `providers/native.rs` / `archive.rs`.
2. Core request/events and the separate bounded `TreeProjection.root_actions`
   snapshot; native root action materialization/alias bookkeeping in
   `tree_projection.rs`; worker-only loaded-state snapshot in `providers/tree.rs`;
   one sweep state in `tree_actor.rs`; routing, correlation, cancellation, and
   completion in `tree_controller.rs`, `tree_controller/mailbox.rs`, and
   `tree_controller/worker.rs`. Move mutation-gate creation before tree controller
   construction in engine `lib.rs` and pass the same Arc.
3. Presenter dirty/in-flight state and native terminal refresh routing in
   `copies.rs`, `tree.rs`, a focused `tree_refresh.rs`, and tests. Use the exact
   action snapshot in presenter `shortcuts.rs`/root-file Enter and viewer engine
   `capture_tree.rs`; retain initial seed selection and safe flat-caret fallback.
4. Real F6 interaction tests assert post-terminal reconciliation, selection and
   session preservation, then qualify Windows/Linux tree/Copy/Move/viewer suites.
   Update TREE_PANEL/SECURITY and verification scope to distinguish this native
   maintenance from the still-separate normal root-refresh retention workflow.
