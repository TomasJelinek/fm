# Remaining service-task boundary peer review

Date: 12 November 2026. Inspected source: `b3fa92e` and the explicitly coordinated
next-batch proposals. This is a read-only source/design review, not test evidence.
No Cargo or native timing command was run by this reviewer. Production implementation
and qualification belong to the task owners. LIFE-01 remains partial.

## Original service ownership

The existing EngineServices owner already contains filesystem, locale, elevation,
navigation fallback, network settings, shell menu and path-picker services. The
shared TaskLifetime can retain that owner across their independent task layers;
these service-only hooks need no additional retirement slots. Every independently
surviving native closure needs its own anchor even if its async parent is anchored.
Store a controller's anchor after its raw service fields. Keep field/closure capture
destruction inside the existing TaskFuture/Blocking envelopes.

| Boundary | Minimal hook | Ordinary returned values |
| --- | --- | --- |
| `lib.rs` LoadLocale | Outer async and native locale query | NumberLocale strings/grouping or FsError |
| `lib.rs` ManageNetworkConnections | Outer async and native dialog call | Unit or FsError |
| `lib.rs` Navigate listing | Outer async and native ListingTask; cover filesystem, locale, elevated service and fallback together | Entry batches, listing metadata, fallback path and FsError |
| `lib.rs` Navigate space | Outer async and native storage query | StorageKind and SpaceInfo |
| `lib.rs` full folder-size scan | Outer async and native scan | Native worker returns unit; published sizes have the diagnostic exception below |
| `shell_menu.rs` | Controller scope, outer async and native menu call | ShellMenuOutcome enum or FsError |
| `path_picker.rs` | Controller scope and direct native closure | Optional PathBuf or FsError |
| `volumes.rs` | Outer run plus discovery/detail async tasks and their native closures; pass scope into Details::schedule | Volume metadata and Completed/VolumeDetails values |
| `volume_information.rs` | Outer request loop and native query closure | VolumeInformationResult metadata |
| `folder_presence.rs` | Both directly submitted native loops | Presence values, with diagnostic exception below |
| `entry_sizes.rs` | Controller scope, outer compute task and native scan | Native worker returns bool; published sizes have the diagnostic exception below |

Volume-information panic delivery uses a second blocking closure containing only
plain error/request data and Outbox. Entry-size finished-only tasks likewise hold
plain reply data. Folder-presence retired request batches contain display root
metadata and IDs. Their existence alone does not justify another native-service
anchor or a new retirement slot. Existing guarded Snapshots remain important.

The actual Windows shell menu's PIDLs/menu/COM values remain local to its native
call; the public result is an enum. PathPicker returns a path, not a dialog/OS
handle. Neither requires a new returned-native-object wrapper. Preserve thread
affinity and the existing launch/query admission permits.

## PathPicker cancellation

Controller currently has explicit shutdown cancellation but no Drop cancellation.
Its started native closure passes a child token to the chooser, then uses the
parent shutdown token for result backpressure. Dropping JoinSet cannot abort that
started closure. Controller Drop should cancel the same existing token so a
cooperative chooser and a publisher waiting on a full live result queue can stop.
This does not promise forced termination of an arbitrary native modal dialog.
The native worker must retain its existing one-chooser permit through actual
chooser completion and reply handling.

Useful focused regressions are: final service release after an unpolled actor;
queued native work canceled before invocation; a native call that outlives its
aborted async parent; and PathPicker Controller Drop with a full result queue.
For the last case, gate actual chooser completion and observe token cancellation,
worker exit and permit release without draining the result queue. Do not infer
Send::Pending solely from a delay. Existing TaskLifetime primitive Ready/panic
coverage complements real actor wiring tests; it does not replace them.

## Diagnostic ownership exception

The earlier remaining-actor inventory called these outputs ordinary metadata too
broadly. FolderPresence::Unavailable can retain TreeDiagnostic.details, whose
GeneratedTextSource is `size_warnings::Text` with a SpoolPrefix. FolderSize.warnings
retains FrozenReport -> ReportMark -> State -> AppendSpool. SpoolPrefix and
AppendSpool contain real File/root owners, without an intrinsic retirement guard.
The file-list size capture can retain the same warning storage through SharedSizes.

Current folder-presence/size generation and blocking_send run on native workers;
a rejected/canceled native send drops its event there. The UI has explicit retired
queues for stale/replaced size results, and guarded Snapshots protect retained
backend roots. Those are real protections. They do not establish that every clone
in SharedSizes, SizeRequest, a completed result/event receiver or generated detail
source has runtime-independent final destruction. TaskLifetime only retains the
original services; it cannot claim protection of newly created diagnostic storage.
Keep this separate ownership follow-up visible. No failing test or blanket shutdown
defect is asserted here, and no diagnostic/output redesign is part of this review.

## File List proposed admission review

The proposed six-token cap is sound as an enforced total logical-owner admission
limit. It includes generated outputs retained after cancellation/removal of their
Job, not merely the four-entry job map and two currently running workers. Each token
reserves two cleanup leases before start may cancel/replace an existing generation.
One lease protects Job (including Pending and captured size state); the second
protects its generated Spool. Both guarded payloads retain the shared token until
actual destruction. A generation produces at most one Spool; confirmation reuses
its captured Pending source rather than generating another one.

Under those conditions twelve payload slots and no extra waiter suffice. A native
worker and its async delivery task share the existing worker permit, so abandoned
native work and completed-but-undelivered output cannot create replacement capacity.
Pending must remain inside guarded Job until native execution. Exhaustion must
refuse visibly before changing the accepted generation. Test retained generated
outputs after Job removal, both cleanup orders, blocked native Drop, and failed
replacement preserving the previous generation. This is design review; final code
and qualification must still establish those premises.

## Dynamic network provider proposed admission review

Current Controller performs exactly one discovery and never retries/replaces it.
One startup lease can therefore cover the successful newly created provider across
its entire lifetime. Adopt the raw Arc inside the native discovery closure before
returning through JoinHandle. A private cloneable Provider wrapping
Arc<WorkerOwned<Arc<dyn NetworkNeighborhood>>> can then be stored in the same
OnceLock and cloned into navigation without exposing a raw owning Arc.

Failure is plain NetworkFailure and releases the unused lease. Discovery panic
unwinds within the native closure. The existing two-request semaphore continues to
bound real navigation calls; original locale work uses TaskLifetime at both layers.
Test completed-but-unconsumed discovery and final OnceLock/controller/task release
with a gated provider destructor. A future discovery retry/replacement feature would
need new admission accounting; the one-slot proof relies on today's exactly-once
contract. The coherent implementation was subsequently reviewed below.

## Coherent implementation review

The root service changes were inspected after formatting: all listed service-bearing
outer/native layers use the existing TaskLifetime envelopes. Picker, shell-menu and
entry-size controller service fields precede their scope. Volumes passes an anchor
through its coordinator, discovery and details children. Volume-information's
fallback closure contains only plain error data. Picker Drop now cancels its existing
shutdown token before field destruction. No actionable anchor-order defect was found.
The six new actual-controller tests inspect wiring; this reviewer did not execute them
or claim that all six independently reproduced baseline defects.

ManageNetworkConnections still holds its one permit only in the outer async task.
Inspection found no same-engine individual-abort path that frees it and then admits
a second surviving native dialog: that task has no exposed AbortHandle, and the engine
aborts its general task set only after leaving command dispatch. Keeping a native
permit clone would strengthen the local invariant, but no current admission bypass
was demonstrated. Do not claim this particular permit universally survives async abort.

The dynamic-network implementation was also reviewed. The successful Provider is
wrapped inside spawn_discovery before the native result can escape. OnceLock and
navigation retain only the private guarded wrapper; get() returns a borrow. Its
startup factory captures no original engine service, so the reserved provider lease
covers discovery ownership; original locale references have both navigation anchors.
The one-slot/no-retry premise holds. The seven added lifetime regression seams cover
completed output, controller/OnceLock release, unpolled navigation, failed/panicking
creation, canceled queued creation and a running native request. Qualification belongs
to its owner; no actionable production defect was found by this source review.

File List's coherent Job/Spool token, same-source confirmation and native/delivery
permit implementation matches the proposed bound. One separate pre-adoption finding
was sent to its owner: file_list_sizes::Captured::bind temporarily cloned an archive
mount while holding Snapshots, released the lock for a second published-map lookup,
and then dropped that clone on the dispatcher before Job construction. Concurrent
native-root replacement could make it the final mount owner.

The owner corrected this lookup and the correction was independently reviewed.
Captured::bind now borrows the ReadyRoot mount while the original Snapshots lock
remains held. Published::snapshot_for neither relocks nor creates a strong source
owner; it preserves exact weak pointer identity and the numeric mount check. Only
the retained size snapshot leaves the lookup. The new source regression preserves
an accepted size snapshot across root retirement and rejects a replacement mount
with the same numeric ID. This closes the reported transient-mount ownership finding
without another slot or weaker identity checks. No additional actionable defect was
found in the reviewed File List implementation. Test execution remains with its owner.

The startup expression now totals 166 slots (153 + twelve File List + one network),
with the same six waiters and unchanged eight/sixteen process quotas. This arithmetic
is source inspection, not execution evidence or a broader lifecycle completion claim.
