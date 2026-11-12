# Shared task service lifetime review — 2026-11-12

Read-only review found no actionable ownership defect in the scoped shared-service changes
listed below. This is a source review, not an independent execution or full LIFE-01 clearance.

## Reviewed boundary

Base repository HEAD: `5388a97`, with the current LIFE-01 working changes.

- `midnight-engine/src/task_lifetime.rs`
- `midnight-engine/src/engine_run.rs` and `lib.rs` shared service-owner construction
- `midnight-engine/src/find.rs`, `find_order_task.rs` and the native worker return boundary
- `midnight-engine/src/preferences.rs` and `fonts.rs`
- Source regressions in `midnight-engine/src/task_service_tests.rs`

The reviewer made no changes to those sources and ran no Cargo command or test target.

## Ownership reasoning

`Engine::run` clones active service references while the original admitted `WorkerOwned`
service bundle is still owned. It then gives the original bundle one shared `TaskLifetime`
anchor. The outer `EngineRun` owns that anchor after its inner future field. Normal completion
may drop active service clones during polling, but the original guarded owner remains retained.
An unpolled or aborted run still retires through the existing admitted run guard.

`TaskFuture` declares its pinned inner future before its anchor. Dropping an unpolled,
cancelled, completed or panicking future therefore drops captured service clones before the
last guarded service owner. Returning Ready does not depend on inner coroutine-local drop
order: the outer anchor survives the call to poll.

`Blocking` stores the complete closure before the anchor and invokes it while that envelope
is alive. This covers cancellation before invocation and unwinding during invocation.
A running native child has its own anchor even after its spawning async actor/runtime exits.
The result is deliberately outside this service-only envelope.

Every newly migrated Find native search, result validation and locale sort closure is wrapped.
The search traversal owns and releases its native FindSource objects within the native call;
its returned summary/results contain metadata rather than a provider handle. Each preference
load/save and font enumeration closure is similarly wrapped. Their returned preferences,
errors and font-name strings do not carry service authority.

The idle actor futures are wrapped independently, so dropping the parent run first cannot
make an independently cancelled actor the final unguarded service owner. The Find Services
record also declares its native reference before its scope.

## Limits and follow-up

This review does not certify actors outside Find/preferences/fonts, arbitrary future users
of TaskLifetime, platform helper subprocess lifetimes, or resource-bearing outputs. A new
captured provider/native object still needs its own admitted retirement guard; a service anchor
is not a substitute for that ownership.

The shared anchor retains the whole original service bundle while any anchored child remains
alive. Existing actor/work admission bounds still apply; this change does not add a worker or
queue. It trades shared service retention for guaranteed final-release placement.

The source tests cover actor cancellation after parent retirement, immediately completed and
panicking futures, uninvoked/panicking blocking closures, and a running native child surviving
runtime shutdown. Execution results belong to the root agent's qualification logs; this review
does not claim those tests passed. Final combined integration and cross-actor validation remain
separate gates.

## Follow-up: Custom Commands and checksum ownership

The same reviewer subsequently inspected the coherent Custom Commands changes in
`user_commands.rs`, `user_command_runtime.rs`, `user_command_process_tracker.rs` and their
desktop reaper handoff, plus `checksum_actor.rs`, `checksum.rs` and `checksum_retirement.rs`.
No actionable ownership defect was found; no tests/builds were run by this reviewer.

Custom Commands retains prepared native pins ahead of its guarded service owner and quota.
Its launch result adopts the child, process quota and pre-reserved tracker ticket before calling
the provider's `id()`. Dropping an unconsumed launch result or pending reaper handoff submits
that owner to the fixed plain tracker. Uncertain/panicking waits retain ownership. Confirmed
exit avoids another wait, and actual native child destruction finishes before tracker slot and
process quota release. The queued service payload excludes Runtime/the tracker, avoiding an
ownership cycle. Drop never joins the process worker; live native children remain bounded.

Checksum admission guards captured requests before validation using the serialized intake.
Preparation transfers the same lease to Work; the staged local bindings retain that lease
through error/panic cleanup. Work drops captured sources before retained quota, worker permit
and guarded Service. Save reserves queue space before adopting a guard under the admission
state lock. Unpolled tasks and unconsumed preparation results remain guarded; returned
ChecksumRun/row values contain metadata and quota rather than native provider handles.
The code owner's planned unconsumed-result and gated-rejection tests target these boundaries;
their results are not asserted here.

These source reviews do not promise that direct `Controller::shutdown().await` completes
within a fixed duration when a provider ignores cancellation. It preserves cooperative waiting.
The bounds are fixed ownership/admission, non-joining Drop, and the application's outer shutdown
grace. Resource guards remain responsible when that outer wait is interrupted.
