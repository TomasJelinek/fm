# Custom Command lifetime follow-up

Status: implemented and qualified on Windows and Linux.
Baseline: `5388a97`.

## Boundaries

At the baseline, the actor returned raw `Prepared` pins from a blocking preparation worker,
retained them across process admission/progress awaits, and returned a raw child from
a blocking launcher. Its detached Tokio fallback lost ownership when that runtime
shut down. Normal desktop tracking also separated the process permit from the child.

Preserve launch validation, selected native identities, preview retention, process
quotas, cooperative cancellation and user-owned child execution. Never kill a launched
child as part of cleanup. Wait errors do not establish exit or release process quota.

## Implemented ownership

- Add 32 command payload slots plus one service-context slot to the shared retirement
  domain, derived from the existing 32-job admission limit. No new waiter is needed:
  synchronous admission can reject before accepting a new job or launching anything.
- One job lease follows accepted Work into guarded Prepared ownership. Every native
  preparation/launch closure retains the same guarded service context. Pins precede
  the service anchor and retained admission resources in destruction order.
- Reserve one fixed plain process tracker during Engine startup, before providers.
  Its eight slots match the existing desktop process quota. A ticket is reserved
  after native process admission and before launch; successful native return is
  immediately wrapped before crossing the JoinHandle boundary.
- A process wrapper retains its child, the shared process permit, an observed-exit
  marker and the reserved ticket. Drop only submits that owned payload to the fixed
  tracker. The tracker polls uncertain children at the existing 100 ms interval,
  retaining permits through errors and shutdown until confirmed exit and actual
  child destruction. It creates no thread per child/drop and does not depend on Tokio.
- Normal desktop tracking keeps its existing polling behavior. The tracked-child
  permit becomes shared with the wrapper, so removing a receiver/future cannot
  release quota before deferred cleanup. Closed-send fallback submits to the same
  tracker instead of starting another Tokio task.

Waiting for a long-running child on the shared retirement worker was rejected: that
would hold unrelated pin/service cleanup behind the child's lifetime. The separate
fixed tracker keeps that wait bounded by eight process owners without that coupling.
A blocked native try_wait/destructor may stall this tracker; it does not justify
replacement threads, inline cleanup, process killing or early quota release.

## Startup contract

`user_commands::Retirement { domain: RetirementDomain, service: RetirementLease }`
and one `ProcessTracker::try_start() -> io::Result<ProcessTracker>` are constructed
before provider services. Controller gains the former argument; desktop's
`command_runtime` forwards the tracker into Runtime. Startup failure therefore
occurs before any provider-bearing payload exists. The command domain adds 33 slots;
the separate tracker owns eight additional process reservations.

## Required evidence

- [x] RED: fallback child and quota survive destruction of a current-thread Tokio
      runtime, then retire off caller only after a gated confirmed exit.
- [x] RED: Prepared native pins cannot be destroyed on the executor when an actor
      awaiting process admission/progress is aborted.
- [x] Unpolled/cancelled blocking handoffs and returned launch results retain their
      guards; native launch permits remain held until the call actually returns.
- [x] Normal tracker receiver loss and a blocked tracking send use the same bounded
      process owner, without killing the process or releasing its permit early.
- [x] Wait errors/panics retain admission; confirmed exit and a blocked destructor
      retain the slot through actual destruction. A later child can still retire.
- [x] Startup worker failure, ticket exhaustion before launch, and idle shutdown
      have explicit deterministic regressions.
- [x] Existing command preview, queue rejection, retained-result quota and native
      cancellation tests stay green; strict scoped Clippy passes.

The RED gates ran against the original ownership paths before implementation:

- `_build/qa/user-command-lifetime-20261112/linux-red.log`: the runtime-shutdown
  regression failed because a still-running process returned its quota (1 rather
  than 0 available permits). One failed test, 890 filtered; body 0.12 seconds.
- `linux-pin-red.log` in the same directory: dropping the real suspended execute
  future retired Prepared's native directory on the caller thread. One failed test,
  897 filtered; body 0.04 seconds. The later fixture adds an explicit preparation
  gate to remove a possible first-poll scheduling race; it preserves that assertion.

A separate read-only review by the viewer agent found no actionable issue in the
Prepared/Context destruction order, native return/id-panic adoption, discarded
launch/send results, uncertain waits, closing wakeups, fixed eight-slot bound and
quota release after native destruction. The guarded service payload excludes the
Runtime/tracker, avoiding a producer/queued-child ownership cycle. The reviewer
ran no tests or builds for this review.

`Controller::shutdown()` retains its existing cooperative await: a native provider
that ignores cancellation can outlast it. Boundedness here means fixed admission
and retained ownership, non-joining Drop, and the application's outer shutdown
grace; it does not promise finite native call or child-exit time. Interrupting that
outer wait leaves the actual payloads guarded. No launched child is killed.

The first Windows GREEN filter passed all 14 tests (0 failed/ignored, 906 filtered;
0.59 seconds), including both repaired RED cases, seven tracker tests and all four
existing actor tests. Evidence: `_build/qa/task-services-20261112/user-green.log`.
The expanded Windows engine run passed all 17 `user_command` tests, including the
three added regressions for unlaunched/cancelled reservation, a cancelled tracking
send plus normal receiver loss, and an unpolled actor's final native service owner.
Evidence: `_build/qa/task-services-20261112/final-windows-tests.log`. That first full
run had one independent checksum fixture failure (918 passed, 1 failed, 7 ignored),
so that first run is retained as failed shared-gate evidence.

The final Windows shared rerun passed all 919 engine unit tests (7 existing ignored,
0 failed/filtered), including all 17 user-command tests. Eleven affected integration
targets also passed all 162 tests. Evidence:
`_build/qa/task-services-20261112/final-windows-tests-2.log`. Strict engine all-target
Clippy passed in `clippy-windows.log` in the same directory (21.35 seconds).
The native Linux shared build passed all 17 `user_command` tests (0 failed/ignored,
899 filtered; 0.50 seconds). Evidence:
`_build/qa/checksum-lifetime-20261112/linux-green-user_command.log`. Strict engine
all-target Clippy also passed in `linux-green-clippy.log` in that directory
(1 minute 31 seconds). The fresh engine test artifact SHA-256 is
`29719e00843a31e57e39a2bc18204cf9776489245139b6810e92fb5c673d4dcf`;
`linux-green-executables.sha256` records the shared gate's exact executables.
These Linux tests were run by the checksum agent in its coordinated shared gate;
the results and logs were inspected before updating this note. Source remains frozen.
