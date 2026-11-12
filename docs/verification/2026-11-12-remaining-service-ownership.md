# Remaining desktop, watch and clipboard ownership boundaries

Date: 2026-11-12. Status: inspection findings and proposed follow-up only.

This review reads the working tree based on `5388a97`, including the current
TaskLifetime, checksum and Custom Command guards. It adds no production changes
or new test results. Line references below identify that inspected source; they
can move in later commits. LIFE-01 remains open. Existing normal-path tests do not
establish cancellation-safe ownership at these remaining boundaries.

## Desktop service tasks

`crates/midnight-engine/src/desktop.rs:53` and `:59` spawn icon/context futures
with raw service Arcs. Their native closures at `:441` and `:514` independently
clone those services. Terminal, open, properties and default-editor delivery
have equivalent unanchored task/closure pairs at `:165`, `:228`, `:298` and `:345`.
The root service owner is guarded, but an independently aborted child can release
a later final raw clone after that original guarded owner has retired.

Pass the existing TaskLifetime into Desktop. Apply both `scope.future(...)` and
`scope.blocking(...)`, including nested native work; anchoring only the outer
future is insufficient. Preserve inner payload before anchor destruction order.
This needs no new service reservation. Icon RGBA, location vectors, property
metadata and editor configuration results are plain data. Their native icon/COM
objects already remain within the native call, so those returned data values do
not need separate provider guards. This is a trait-owner lifetime gap, not a
claim that every current platform service has an expensive destructor.

## Ordinary terminal, editor and opener processes

The concrete native-owner gap is broader than service Arcs:

- `desktop.rs:171` returns a raw `Box<dyn TerminalProcess>` from the blocking
  terminal launch; `:178` also calls its `id()` before any guarded adoption.
- `desktop.rs:230` can return an `OpenEntryResult` containing a native child.
- `desktop_editor.rs:99` returns a raw editor child and launch permit.
- `desktop.rs:631` keeps raw children in both its receiver and local vector.
  `:642` returns a vector of live children from another blocking call. Receiver
  closure, an aborted send/reaper, or a completed but unconsumed native output
  can therefore destroy a child on an executor and release its process quota
  without observing exit. A Windows process handle is a real native owner; on
  Unix, dropping Child also does not perform the required eventual reaping.

The new Custom Command TrackedProcess already protects that subset while it
passes through this reaper. Ordinary terminal/editor/opener children remain raw.
The smallest coherent migration is to share that fixed tracker owner: reserve
before launch, construct the wrapper inside the native closure before any `id`
call or return, and let every failed send/aborted output retire the same wrapper.
Generalize its service anchor without adding a service/tracker ownership cycle.
Keep the existing normal opener completion/error event path.

Terminal, editor and Custom Commands share eight existing process permits
(`desktop.rs:21`, `:77`). Their tracker tickets should share the same eight-slot
pool. Openers have sixteen independent permits (`:22`, `:79`); add a separate
fixed sixteen-slot tracker or explicitly partition one fixed twenty-four-slot
tracker. Do not accidentally impose the terminal limit on openers. Startup must
remain fallible before any launch, and uncertain waits must retain the native
owner and its existing permit until confirmed exit and actual destruction.
No process killing, new unbounded queue or long-lived polling on the shared
retirement worker is needed.

Captured editor targets are a second payload boundary. The explicit worker
`drop(target)` at `desktop_editor.rs:103` and rejection closure at `:138` protect
only closures that actually run. A cancelled, not-yet-started blocking closure
still owns a raw provider capture. Use an admitted WorkerOwned target/permit
bundle before submission, with a serial guarded rejection path. The current
four launch permits provide the accepted-work bound; retain them through actual
payload destruction. Derive the final ledger from that bound plus one serialized
rejection, rather than allowing arbitrary cleanup submissions.

## Watch workers

`watch.rs:124` wraps each of two blocking loops in an unanchored async task.
Before its first poll, that future owns filesystem/watch service Arcs. Its nested
closure at `:125` needs its own TaskLifetime too. Once the loop starts, native
subscriptions stay in its local Target/tree Runtime and are destroyed there;
`watch.rs:414-423` also drains retained shared scopes on normal exit. Controller
Drop already cancels and wakes both workers (`:185`). Preserve these good paths.

The remaining payload is Shared's optional TreeControl, not its ordinary listing
metadata. `watch/tree.rs:13` retains MountedTarget and RootRegistrationLease in
each ScopeTarget. If the loop never starts, or unwinds before its explicit final
drain, those resources can remain in Shared. A later final Arc<Shared> release
from an aborted task/handle has no worker-only destruction guarantee.

Use two startup leases, one for each `Arc<WorkerOwned<Shared>>`, alongside the
service task anchors. Keep normal scope replacement on the existing model worker
(`watch/tree.rs:81-125`) and normal subscription destruction on the watch worker.
Do not move live native subscriptions to async code or introduce a new watcher
thread per request. A deterministic regression should populate a real captured
scope, cancel before the loop starts, then release the last handle on the caller;
a second case should unwind before final drain. Gate final mounted-resource Drop
and prove it runs off caller while its fixed lease remains unavailable.

## Clipboard worker and rejected requests

`clipboard.rs:94-98` creates a bounded sixteen-request receiver and transfers it,
the raw ClipboardService and cancellation state to a blocking closure. Captured
and generated actions (`:15-20`) retain provider/file owners, unlike plain text
or clipboard path metadata. Cancellation before that closure starts can discard
the receiver's queued owners without the intended native-thread cleanup.
Rejected resource-bearing requests use another raw blocking closure at `:170`;
that rejection path has the same pre-start ownership gap.

Reuse TaskLifetime for the worker's service context. Guard the receiver before
it accepts any request; its final queue drain can then run on the shared cleanup
worker even when the native loop never starts. A single receiver reservation is
sufficient for this sixteen-entry private queue: submission uses only try_send,
with no externally retained reserve permits. Guard the one serialized rejection
before native submission, using a fixed retirement waiter if its previous owner
is still retiring. That suggests two payload slots and one waiter; prove the
intake handoff remains inside EngineRun protection until its guard is installed.
An alternative per-request design must explicitly account for queued, active and
serialized intake owners rather than copying the channel length alone.

Do not wrap/move an opened ClipboardSession into cross-thread retirement. It is
thread-affine and is already created, pumped and destroyed in the same blocking
loop (`clipboard.rs:99-152`; `midnight-core/src/desktop.rs:488`). The Linux session
owns an X11 clipboard connection/window; this is not merely metadata. A running
loop returns no native session through its JoinHandle, so the normal started
session path is not the defect described here.

## Focused evidence for the next batch

- Gate service Drop across an unpolled desktop/watch task and a nested native
  closure that survives its parent's abort. Prove final service destruction is
  off executor without adding service reservations.
- Gate ordinary terminal/editor/opener launch outputs, cancelled tracking sends,
  reaper shutdown and id/wait failure. Preserve eight/sixteen process admission,
  opener completion events and eventual reaping without killing any child.
- Exercise queued and rejected captured editor/clipboard requests with a blocked
  native worker pool; cancel before invocation, then verify provider destruction
  and lease release order. Test clipboard session creation/use/Drop thread
  identity separately so queue cleanup does not weaken its affinity contract.
- Exercise watch Shared with a retained mounted scope before worker startup and
  on unwind; retain existing no-follow/watch cancellation behavior.

These are proposed regressions, not executed evidence. No additional Cargo or
native timing run was started for this inspection.
