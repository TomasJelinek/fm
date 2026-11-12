# Engine lifetime integration ownership review

Date: 12 November 2026. Read-only review against the working LIFE-01 changes after baseline `6f06ec4`.
This note records inspection findings, not a claim that all engine shutdown paths have been tested.
See the [primitive evidence](2026-11-12-retirement-primitive.md) and
[LIFE-01 implementation plan](../superpowers/plans/2026-11-12-runtime-independent-retirement.md).

**Conclusion:** The root run/command boundary, Copy/Delete job guards, shared snapshots and drag
ownership changes are useful independently. The current batch must not claim LIFE-01 fully closed:
service references and provider-bearing work in other independently spawned actors still need guards.
No additional blocker was found by inspection in the changed root/receiver/snapshot/drag paths below.

## Follow-up status

This review describes the earlier 74-slot implementation. The later
[child-task ownership batch](2026-11-12-task-services.md) guards Find, preference and
font service children, checksum payloads and Custom Command captures/processes.
Its ledger is 144 payload slots plus four intake waiters and a separate eight-slot
process tracker. These changes address the corresponding findings below; desktop,
watch, clipboard and builder-replacement gaps remain separately open. Historical
observations and original verification are retained rather than rewritten.

## Reviewed changed boundaries

- `engine_lifetime.rs:25`: startup creates the retirement worker and reserves engine resources before
  provider construction. Partial reservation failure has only empty leases/tokens to release. App
  `main.rs:133` calls `Engine::lifetime()` before constructing local/native providers and Tokio.
- `lib.rs:159, 256`: an Engine that is never run keeps its services inside `WorkerOwned`; dropping it
  before a runtime exists therefore submits the original owners to the plain cleanup worker.
- `lib.rs` public `Engine::run` is synchronous and wraps `CommandInbox` using the dedicated Engine
  command lease before passing it to `run_inner`; the whole pinned future is then inside `EngineRun`
  before its first poll.
  These handoffs contain no await or fallible provider operation.
- `engine_run.rs:6-50`: the Run bundle declares its future before its original service owners.
  Normal completion explicitly retires that bundle and awaits acknowledgement; abort/drop submits the
  suspended bundle through its existing lease. The original service copy protects destruction of
  inner locals during normal completion, but is not an anchor for independently surviving child tasks.
- `engine_commands.rs:7-25`: close plus blocking receive-to-None runs only inside the retirement guard.
  It retains the receiver and drains commands sent using permits reserved before close. Plain
  Receiver::drop or a one-time try_recv drain would not retain those later payloads. An indefinitely
  held producer permit can stall this fixed worker and its queued cleanup; the occupied slot remains
  held. This is bounded retained ownership, not a finite completion-time guarantee.
- `lib.rs:221`: `Arc<WorkerOwned<Mutex<[SnapshotState; 2]>>>` protects the actual final shared owner,
  including clones surviving the root future. `tree_roots.rs:100` still performs its ordinary root
  drain on a worker; cancelling its queued closure leaves the guarded snapshot owner intact.
- `drag_drop.rs:19, 277, 342, 448, 659`: shared state and pending accepted drops are guarded; backlog
  drain reserves before taking payloads out of shared state. Shutdown leaves final retired payloads
  in guarded state and no longer starts Tokio cleanup from Drop. Native pins/captures remain on their
  existing provider paths; the change does not grant or reinterpret operation authority.

## Slot ledger

The configured 74 payload slots derive from existing admissions:

| Owner class | Slots |
| --- | ---: |
| Accepted Copy jobs/native-drop reservations sharing the Copy capacity gate | 32 |
| Accepted mutation jobs | 32 |
| Serialized Copy and mutation intake/rejection owners | 2 |
| Copy and mutation service contexts | 2 |
| Drag state, pending accepted drops, serialized backlog drain | 3 |
| Whole run bundle, whole command inbox, shared panel snapshots | 3 |
| Total | 74 |

Three fixed waiter tokens serve the two actor intakes and drag backlog. Native-drop reservations are
already included in the Copy gate's 32 permits; they are not an additional unbounded source. A job's
lease follows its preparation/result/queue/execution transitions. Payloads awaiting the initial lease
remain within the guarded parent run future. The generic primitive takes no payload during reservation.

The actor review separately found that destructuring `((input, service, locale), lease)` before an
error return could release the later-bound lease before a final locale owner. The actor owner accepted
this finding, reproduced both error/panic paths, and applied two-stage unpacking. The regression
`operations::tests::retirement::failed_or_panicked_preparation_keeps_slot_until_final_locale_owner_drops`
passes in the final Windows affected-unit run and the focused Linux operations run. The lease now
remains bound before the payload locals, so it cannot release capacity while their destruction blocks.

## Remaining service ownership gap

`Run::_services` is not sufficient to guarantee all final service destruction on the cleanup worker.
Dropping the run future drops its JoinSet, which requests child cancellation rather than synchronously
destroying every child future. The cleanup worker can then release `_services` while a child still
holds the final ordinary service Arc. Its later destruction may occur on a Tokio executor.

Concrete retained owners include:

- `find.rs:64-99`: sorting, run_slot and run_validations are independent asynchronous tasks retaining
  locale/FindService references. Native child work at `find.rs:323, 485` can also outlive those tasks.
- `desktop.rs:53-68`: icon/context/reaper actors are separate tasks. Terminal/open-entry methods create
  further async-to-blocking handoffs, for example `desktop.rs:161-167, 224-226`.
- `preferences.rs:186, 200`: load/save closures retain the settings service separately from their
  awaiting parent. During runtime shutdown a queued blocking closure can be cancelled before starting;
  retaining only the parent future's service anchor does not cover that closure's final owner.

A coherent follow-up is one shared `Arc<WorkerOwned<EngineServices>>` anchor backed by one startup
lease, plus an EngineTasks/TaskLifetime seam for async, blocking and detached spawns. Envelopes must
retain the inner future/closure before the anchor in destruction order. The same anchor must follow
nested tasks; a top-level JoinSet wrapper alone is incomplete. Provider-bearing task outputs require
an existing job guard or a guarded output envelope before returning to the async scheduler. Do not
silently unwrap such output into an unprotected JoinHandle result. This seam needs a dedicated audit
and tests rather than a mechanical rename of task APIs.

## Builder replacement limitation

Repeated `Engine::with_*` calls replace fields inside `WorkerOwned<EngineServices>` through DerefMut,
for example `with_locale`, `with_settings` and `with_network(None)`. A displaced prior service Arc can
therefore be destroyed on the caller during builder configuration. Guarding final Engine destruction
does not protect the old value being replaced. Production startup sets each optional service once;
this is an existing configuration-path gap, not a regression introduced by the new final-state guard.
The follow-up must either retain/retire displaced configurations safely or make configuration ownership
explicitly one-time. Do not claim blanket Engine service-lifetime safety from this batch.

## Remaining detached request/resource owners

Service anchors alone do not protect newly captured handles and provider-bearing requests:

- `checksum_actor.rs:73` starts an independent actor using raw Tokio spawn. Its input receiver carries
  `Input::Start` with captured checksum sources. In `run` at `374-425`, a dequeued Start is retained
  across the queued-progress send before preparation. The preparation result can also contain a raw
  captured source Arc before transfer into the backend. Root-future retirement does not own this task.
- `checksum.rs:122` starts another detached backend. Its input Work and native result paths at
  `281-376` rely on explicit normal cleanup, including Tokio blocking retirement. Runtime destruction
  can bypass that drain. A guarded adapter/backend actor or guarded per-job input/output handoffs are
  needed, preserving existing checksum quotas and source leases.
- `user_commands.rs:59` starts its detached actor. `execute` at `210-355` receives a raw Prepared result
  from a blocking worker and retains it through later awaited progress/launch work. Prepared owns
  native directory/target pins through `user_command_prepare.rs:13-23`; ordinary final cleanup at
  `user_commands.rs:355` is not guaranteed to execute after forced cancellation.
- `user_command_runtime.rs:62` has a detached fallback process tracker. Its process/permit lifetime
  must be included in the later task-spawn audit; retaining a service anchor alone says nothing about
  the separately returned TerminalProcess object's destruction.

The shortest safe follow-up is to guard these actual actor/request/result owners using their existing
admission bounds, then thread the shared service anchor through every independent child lifetime.
Keep native execution semantics, source proofs, quotas and process ownership unchanged.

## Qualification implications

Primitive tests prove the mechanism without Tokio; they do not qualify all application ownership.
Current lifecycle test source covers never-polled run/engine destruction, consumed plus queued capture
retirement while progress delivery is pending, normal Shutdown with queued captures, a previously
reserved late send, and runtime shutdown after a manual first poll with a consumed capture blocked
on the full outbox plus a second capture still queued. The latter test was added during this review.
All eight lifecycle cases now pass, including interruption during awaited tree shutdown. The original
Linux shutdown fixture failure is retained separately; the final test uses bounded runtime shutdown,
matching the application, before dropping the guarded future.

Qualification evidence under `_build/qa/life01-engine-20261112/`:

- `affected-units-green.log`: all eight lifecycle cases and the locale-owner error/panic regression
  pass within the Windows engine suite (893 passed, 7 existing ignored, 0 failed/filtered). The nine
  affected unit suites total 2,385 passed and 27 existing ignored, with no failures.
- `linux-final-engine_lifecycle.log`: 8 passed, 0 failed; includes never-polled, late reserved send, pending
  dispatch, runtime shutdown, normal shutdown and interrupted tree-shutdown cases.
- `linux-final-operations--.log`: 200 passed, 0 failed, including the locale-owner error/panic regression.
- `linux-clippy-final.log`: strict core/engine all-target Clippy passes after the named service
  grouping and test-helper alias corrections.
- `linux-engine_lifecycle-before-bounded-shutdown.log`: preserved first Linux fixture failure evidence.

The tests use gated fake resources and bounded watchdogs to establish the ownership boundary rather
than relying on sleeps. These results qualify the listed changed boundaries, not the deferred service
and detached-actor paths above. For the later service seam, use a current-thread runtime that cannot process child aborts while the
cleanup worker retires the parent; verify final service destruction after the child is released. Add
checksum captured-input and Custom Command pinned-result cancellation cases independently.

Existing successful normal-operation tests must remain green. This batch's result should be described
as partial LIFE-01 remediation with the concrete remaining paths above, not a complete lifecycle fix.
