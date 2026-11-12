# Runtime-independent retirement implementation plan

> For agentic workers: use superpowers:subagent-driven-development or
> superpowers:executing-plans task by task. Checkmarks record completed source/targeted evidence only; final qualification is separate.

**Status:** Partially implemented. Root/Copy/Delete/drag retirement and the fixed-slot primitive
have targeted tests. Independent service/checksum/custom-command ownership remains open; see
[the review](../../verification/2026-11-12-engine-lifetime-review.md). This is not full LIFE-01 closure.

**Goal:** Retire engine-owned captures and native resources away from callers/executors when the run
future is never polled, aborted during dispatch/shutdown, or destroyed as Tokio stops.

**Architecture:** Establish a reserved retirement capability synchronously before accepting ownership.
A fixed plain-thread retirement domain owns suspended run futures and guarded actor resources without
requiring Tokio. Reservations remain occupied until actual destruction finishes.

**Tech stack:** Safe Rust, standard threads/synchronization, existing Tokio engine. No new dependency
or platform mutation API is proposed.

**Spec:** [LIFE-01](../../IMPLEMENTATION_BACKLOG.md#lifecycle-follow-up-found-on-12-november),
[code style](../../../CODE_STYLE.md) and [security requirements](../../../SECURITY.md).

## Scope, evidence and constraints

- `crates/midnight-engine/src/lib.rs:372`: async `Engine::run` initially owns the engine and receiver.
  Its body has not established cleanup ownership when an unpolled future is dropped.
- `lib.rs:721, 741, 780`: awaited dispatch can own already-dequeued captures or delivery payloads.
  A receiver-only guard does not cover these owners.
- `lib.rs:1024-1074`: normal shutdown retires the receiver after awaiting tree shutdown. Forced
  cancellation can bypass that path. Preserve normal shutdown ordering and whole-receiver retention,
  including sends using permits reserved before close.
- `copy_retirement.rs:28` and `drag_drop.rs:664`: destructor fallbacks currently call `spawn_blocking`.
  A plain cleanup worker must not execute those fallbacks unchanged.
- Pinned Tokio 1.54.1 `runtime/blocking/pool.rs:614-621` shuts down a submitted task immediately when
  the pool is stopping. Retaining a Tokio handle alone does not guarantee its destruction thread.
- `crates/midnight-explorer/src/main.rs:630`: engine completion shares a 2.5-second exit deadline;
  runtime shutdown subsequently has a one-second timeout. Preserve bounded application exit.
- Actor ownership is a separate boundary: `JoinSet::Drop` aborts child tasks; it does not transfer
  their future destruction to the thread dropping the set. Do not claim the outer wrapper fixes it.
- Preserve native pins before provider-lease destruction, confirmation, checksums, cancellation,
  source-proof deletion and existing queue/worker limits. No changes to operation authority.
- No synchronous destructor waits, per-drop replacement threads, unbounded queues, leaked owners,
  caller-side fallback destruction, or unsafe future movement. Keep pinned futures pinned.
- An arbitrary blocked provider destructor has no guaranteed finite completion time. Bound retained
  ownership and thread count; keep its reservation until it returns instead of admitting replacements.
- Coordinate Cargo/native timing leases; all generated evidence belongs under `_build/`.

## Review focus

1. Ownership accepted before worker startup or capacity succeeds: startup must fail without consuming
   provider-bearing state (task 1).
2. Cancellation after dequeue or before first poll: the complete suspended future must be owned by the
   retirement domain (task 2).
3. Independently spawned task/output owners: protect the actual last-owner boundary (task 3).
4. Nested destruction, worker panic and runtime shutdown: keep the cleanup domain usable without
   Tokio and never silently discard its queued payloads (tasks 1-3).
5. Blocked cleanup and late reserved sends: capacity and provider lifetimes must remain held until
   actual destruction, including normal shutdown (tasks 1, 2 and 4).

## Task 1: Runtime-independent retirement ownership

**Files:** new `crates/midnight-engine/src/retirement.rs` and `retirement_tests.rs`;
module export in `lib.rs`. Keep future-specific code in task 2's separate module.

**Proposed interfaces:** `RetirementDomain::try_start() -> io::Result<RetirementDomain>`;
`try_reserve_run(&self) -> Result<RunLease, RetirementBusy>`; typed `WorkerOwned<T>` carrying a
reserved slot. Reserve first, then pass owned payloads; reservation APIs take no provider values.

- [x] Derive and document a finite slot ledger from existing engine/controller admission limits,
  including the 32-operation bound, live actors and bounded dispatch/rejection ownership. Reserve a
  run's required capacity before engine construction; do not introduce an arbitrary cleanup backlog.
- [x] Add failing tests for no-runtime retirement, injected thread-creation failure before ownership,
  exhausted reservations, nested retirement and a blocked destructor retaining its slot.
- [x] Implement one fixed named cleanup worker with reserved slots, no fallible/full-queue ownership
  handoff in `Drop`, and payload destruction outside synchronization locks. Live reservations keep
  the domain alive. Do not join its worker from executor/UI destructors.
- [x] Contain a single unwinding payload destructor so it cannot strand unrelated queued owners;
  preserve Rust's process-abort behavior for double panic. Verify later payloads still retire.
- [x] Implement acknowledged normal retirement; dropping an acknowledgement waiter must not cancel
  cleanup. Run the focused primitive tests and record both failure and passing evidence.

## Task 2: Protect the entire Engine run lifetime

**Files:** new `crates/midnight-engine/src/engine_run.rs` and `engine_lifecycle_tests.rs`;
modify `lib.rs`, `copy_retirement.rs`, `drag_drop.rs`, and app startup in
`crates/midnight-explorer/src/main.rs`.

**Interfaces:** engine construction consumes an already-reserved `RunLease`;
`Engine::run(self, Receiver<Command>) -> EngineRun` becomes synchronous. `EngineRun` implements
`Future<Output = ()>` and owns the pinned async implementation before returning to its caller.

- [x] Write a RED test that queues tracked captures and drops an unpolled run without any Tokio runtime.
- [ ] Write a RED mid-await test: prefill the event channel, poll through captured Delete admission
  until progress delivery is pending, and retain a second capture in the command queue. Observe
  dequeue through a fake source notification; do not infer the boundary from sleeps.
- [x] Start/reserve retirement before constructing provider-bearing engine state. Worker-creation or
  reservation failure returns an ordinary startup error before ownership moves; never return an
  unguarded provider-bearing future inside an error or panic because Tokio is absent.
- [x] Implement synchronous `EngineRun` construction and transfer its suspended pinned future to the
  reserved cleanup slot on abort/drop. Also guard engine state dropped before `run` is invoked.
- [x] Replace Tokio-dependent `WorkerOwned` and drag-drop destructor fallbacks. Nested cleanup already
  executing on the domain worker may destroy directly there; other owners require reserved slots.
- [ ] Preserve normal explicit shutdown and retire remaining engine service state before successful
  completion. Returning `Ready` is not enough: locals may be destroyed during the final inner poll.
- [ ] Add runtime-shutdown and interrupted-tree-shutdown tests using gated fake providers. Assert
  queued and consumed captures retire off callers/executors, and blocked native ownership persists
  until its gate releases. Run all focused lifecycle tests.

## Task 3: Close independently spawned actor ownership boundaries

**Files:** `operations.rs`, `operation_worker.rs`, `copy_operations.rs`, `copy_decisions.rs`,
`tree_roots.rs`; dedicated actor retirement tests. Add other handoff files only when the ownership
trace identifies a concrete final-owner gap, not as an unrelated actor rewrite.

- [ ] Trace ownership through actor receivers, shared snapshots, preparation outputs, task abort and
  completed-but-unconsumed task results. Record each protected last-owner boundary and its slot.
- [ ] Add RED operation tests at Preview delivery, awaiting confirmation, predecessor/mutation-gate
  waits, and unconsumed preparation output. Record native-pin-before-provider destruction order.
- [x] Guard operation Jobs inside the blocking preparation closure before returning them, retaining
  the same ownership/reservation through prepared/queued collections, execution and disposal.
  `operations.rs:342` currently stores raw Jobs; `operation_worker.rs:300` has async disposal entry
  points whose unpolled futures also own raw Jobs.
- [x] Retain Copy's existing guarded preparation/job handoffs (`copy_decisions.rs:62`) while replacing
  their Tokio-dependent retirement implementation. Audit result paths without changing Copy behavior.
- [ ] Protect independently spawned snapshot/controller owners where the trace proves that dropping
  the root future can leave a final owner on an executor. Preserve existing provider/worker permits.
- [ ] Run actor cancellation, output-retirement and existing captured Copy/Delete ownership tests.

## Task 4: Qualification and truthful closure

- [ ] Test saturation with a gated destructor: no inline fallback, no replacement threads, no early
  permit reuse; rejected reservation leaves the caller's unconsumed payload untouched.
- [ ] Test normal Shutdown followed by queued captures and a send reserved before receiver close.
- [ ] Run focused engine lifecycle/actor tests and strict engine/app Clippy on Windows and Linux;
  then run affected unit suites once after implementation stabilizes. Use private fixture directories
  and the coordinated Cargo configuration; preserve RED logs and exact executable/build provenance.
- [ ] Independently review startup failure, nested Drop, output handoff and runtime-shutdown paths.
- [ ] Record actual evidence and remaining limitations before updating LIFE-01. Do not claim complete
  coverage if only the outer Engine future is protected. Commit only when separately instructed.
