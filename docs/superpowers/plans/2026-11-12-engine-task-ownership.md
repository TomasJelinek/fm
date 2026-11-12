# Engine child-task ownership follow-up

Date: 12 November 2026. Continues LIFE-01 and the approved repository audit.

## Scope

Protect actual final service owners after an EngineRun is retired while independent
Find, preference and font tasks are still cancelling. Protect checksum captured
inputs/results and Custom Command pins/processes independently; a service anchor
alone cannot protect newly captured resources. Preserve native authority, operation
quotas and execution behavior. Delete continues to use the shared Copy/Move progress
window, with item throughput and cancellation on window close.

## Implementation sequence

1. Reproduce final Find/settings service destruction on the executor after the
   parent owner retires. Use current-thread runtimes and controlled queued native
   work, not timing guesses.
2. Reserve one separate service-bundle lease at startup. Share its guarded owner
   across the run and migrated child tasks. Future/closure envelopes drop inner
   captures before their service anchor; nested blocking work inherits the anchor.
3. Migrate Find slots, validation/sorting, settings persistence and font enumeration.
   Do not claim that unrelated unconverted service actors are thereby qualified.
4. In parallel, guard checksum inputs, preparation results and execution with the
   same admitted job lease. Add 36 retirement slots and one intake waiter, derived
   from 32 jobs, two manifest saves, one intake and one service context.
5. In parallel, guard Custom Command preparation/launch ownership (33 slots). Create
   a fixed eight-slot plain process tracker before providers; process quota stays
   held until observed exit and native cleanup, including runtime shutdown.
6. Run targeted ownership regressions and affected functional tests. After integration,
   run strict affected-target Clippy and build a fresh debug application. Document
   evidence and still-unmigrated actors; commit/push only the verified batch.

## Limits and checks

The shared service guard does not guard provider-bearing return values; those need
an admitted job/output owner. It does not fix replacement by repeated builder calls.
No cleanup path may start an unbounded thread or depend on a running Tokio runtime.
No process is killed as an implicit side effect of dropping its tracker. Runtime
shutdown tests use bounded shutdown deadlines and independently release native gates.

## Batch result

The six steps above are implemented and qualified for the named boundaries.
[Final verification](../../verification/2026-11-12-task-services.md) records Windows
and Linux affected suites, strict Clippy, the debug executable hash, private runtime
registration, and native/software startup smoke. Independent ownership review found
no actionable issue in this batch. Remaining actors and interactive D3D pacing stay
open in the implementation backlog; this is not completion of the full audit goal.
