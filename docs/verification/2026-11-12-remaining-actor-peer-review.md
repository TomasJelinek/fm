# Clipboard, desktop and watch ownership peer review

Date: 2026-11-12. Base: `b9f8ee4`. This is an independent source/design review;
no Cargo command, test target or production edit was performed by this reviewer.
The implementing agents own qualification evidence. LIFE-01 is not cleared here.

## Clipboard proposal

Inspected `clipboard.rs`, its captured/generated tests, `engine_run.rs`,
`engine_commands.rs`, `task_lifetime.rs`, the dispatch/shutdown path in `lib.rs`,
and the ClipboardSession contract in `midnight-core/src/desktop.rs`.

The proposed receiver guard covers the private sixteen-entry channel: production
submission uses only `try_send`, and no reserved sender permit escapes. Once the
native loop starts, active Request/Prepared values stay within that loop; no
native or provider-bearing result escapes through its JoinHandle. Thread-affine
ClipboardSession must remain a local created after the guarded inbox binding,
with creation, pumping, publication and destruction on that same native thread.
The guarded worker's receiver/service/TaskLifetime field order keeps its final
service anchor after raw service references and queued captures.

The serialized rejection can borrow `Request::failure(&self)` to construct a
plain Event, adopt the request, await actual retirement, then deliver that event.
This removes the old cancellable cleanup closure without weakening refusal
backpressure. Preserve reporting of a retirement panic as an error. Raw intake
while waiting for its fixed lease is protected by the enclosing EngineRun guard;
that is an internal dispatch contract, not a guarantee for arbitrary unguarded
callers of a standalone Controller future.

One additional concrete cancellation gap was reported before implementation:
Controller had explicit shutdown but no Drop cancellation. If a started native
loop is waiting in `send_event` on a full but live result receiver, losing the
request sender and aborting JoinSet cannot stop the started blocking task. The
loop checks its global cancellation token, not request-channel disconnection.
Controller Drop must cancel it too. The owner accepted a regression with a full
result queue and a session recording opening/copying/destruction thread identity.

Useful focused seams are pre-invocation cancellation with queued captured and
generated requests, aborted rejection cleanup, a gated destructor retaining its
fixed slot, and the full-result-queue Controller Drop case. Existing cancellation
checks after native open and before publication must remain intact. These are
review requirements/proposed tests, not results asserted by this reviewer.

## Clipboard implementation review

The coherent implementation and `clipboard_retirement_tests.rs` were subsequently
reviewed. No further actionable ownership gap was found. NativeWorker is guarded
before channel admission; its receiver/service/scope fields retain queued sources
and the service anchor through pre-start cancellation and cleanup. Session stays
local. Active requests are explicitly dropped before result backpressure. Rejected
requests retain both scope and slot through acknowledged retirement, including
cancelled acknowledgements. Controller Drop cancels the native pump.

The startup ledger adds two payload slots and one fixed waiter, matching one
worker/inbox plus one serialized rejected request. The source regressions cover
idle service retirement, same-thread native session closure under result queue
backpressure, pre-start queue cancellation, and gated captured/generated rejection.
The pre-start queued-capture test already passed the baseline; it is a retained
guard test, not independent RED evidence. No test execution result is asserted by
this reviewer.

## Desktop implementation review

Reviewed `desktop.rs`, `desktop_editor.rs`, the shared process tracker/runtime,
startup wiring and their new lifetime regressions. No actionable production
ownership or quota defect was found. Both service task layers retain TaskLifetime.
Every terminal/editor/opener child is adopted in its native launch closure before
`id()` or return. Pending sends, completed but unconsumed outputs, receiver queues
and reaper vectors therefore own the same guarded child. Process quota survives
uncertain waits and final native destruction. Terminal/editor/Custom Commands
share eight tracker slots; the separate opener tracker retains sixteen slots.
The service anchor does not contain those trackers, avoiding an ownership cycle.

The final editor version keeps `Work { target, launch }` guarded continuously from
admission, with raw extraction confined to native execution. Target destruction
precedes launch service/permits; the lease survives unwind. Rejection reports
cleanup failure. Its five-slot bound follows four launch permits and EngineRun's
one serial intake. The completed-output regression uses a one-native-worker barrier.
A review caveat was sent to the owner: launch notification plus a 30ms delay in the
full-channel cancellation case proves abort-after-launch, but does not by itself
prove the delivery future reached Send::Pending. No production issue follows from
that evidence distinction; exact phase claims need an explicit barrier.

## Watch implementation review

Reviewed `watch.rs`, the SharedOwner type changes in `watch/tree.rs`, and the three
new lifetime tests/fixture. No actionable defect was found. Both pane controls are
startup-admitted `Arc<WorkerOwned<Shared>>` values, including handles and callbacks.
TaskLifetime covers the outer task and independently surviving native closure.
Native subscriptions stay in local Target/Runtime and unwind on their creator
thread; ordinary worker draining remains intact.

The source tests construct a real Session/NativeProvider scope, exercise before-
first-poll and native-unwind boundaries, and gate final mounted-provider Drop. A
retained second pane makes the unavailable-domain assertion prove that the gated
first pane still holds its slot. Final service destruction has its own case.
Qualification belongs to the implementing agents; this reviewer ran no tests.

## Final documentation consistency review

The current engine contract, implementation backlog and native-service qualification
agree with the startup code: 153 payload slots and six fixed waiter tokens. The
capacity is 32 + 32 + 11 + 36 + 33 + 2 + 5 + 2. Waiters belong to mutation, Copy,
drag backlog, checksum intake, rejected clipboard input and editor intake. Watch
uses two startup leases without another waiter. The process trackers remain separate:
eight shared terminal/editor/Custom Command owners and sixteen independent openers.
The sixteen-entry backing arrays do not increase the terminal tracker's admitted
capacity. The four editor launch permits plus one serialized intake justify its
five retirement slots.

The current documents retain File List, dynamic network/archive ownership, other
service tasks and builder replacement as follow-ups. They distinguish bounded
ownership/admission from a finite completion promise for arbitrary native calls or
destructors. No blanket LIFE-01 closure or native performance result is implied.
Earlier 74/144-slot reports are historical evidence, not the current startup ledger.

## CI snapshot

The initial inspection of run `43144250124` for `b9f8ee4` found seventeen successful
jobs, with Windows PDF still running and application checks not yet listed. A later
single snapshot requested for final documentation review found eighteen successful
jobs, the Ubuntu application check failed, and the Windows application check still
running. The aggregate run was not terminal; it must not be reported as all green.
Neither inspection started a polling loop or a workflow.

The completed Ubuntu job `130940094445` failed this command:

```text
cargo test --workspace --locked --test tree_controller --message-format=json
```

`foreign_view_is_refused_before_it_can_queue_behind_provider_validation` panicked at
`crates/midnight-engine/tests/tree_controller.rs:1177:5` with
`foreign view must never enter the model queue`. The target reported 17 passed,
one failed and one ignored in 0.30 seconds. This is baseline CI evidence, not a
local reproduction or a diagnosis of production behavior. The exact case was sent
to the parent and assigned for separate investigation. The completed job log was
read through the direct job API because the aggregate run was still active.
