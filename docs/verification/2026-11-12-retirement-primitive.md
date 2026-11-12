# Runtime-independent retirement primitive verification

Date: 12 November 2026. Scope: new engine `retirement.rs` and `retirement_tests.rs` only.
This implements the shared primitive from the [LIFE-01 plan](../superpowers/plans/2026-11-12-runtime-independent-retirement.md).
Engine entry, actor integration and runtime-shutdown qualification are recorded separately in the
[integration ownership review](2026-11-12-engine-lifetime-review.md); these primitive results
must not be reported as closure of LIFE-01.

## Ownership and failure behavior

- `RetirementDomain::try_start(payload_capacity, waiter_capacity)` creates one named standard-library
  cleanup thread and preallocates both slot arrays. Capacities come from the integration admission
  ledger; this module has no global queue or hardcoded engine/job limit.
- Startup validates positive payload capacity and returns ordinary thread-creation errors before
  handing out a lease. The injected failure test does not consume provider-bearing state.
- `try_reserve()` accepts no payload. `RetirementLease::own(value)` establishes ownership before any
  future poll; guard Drop transfers its existing box to its reserved slot without allocating another
  payload box. The worker releases the slot only after the destructor returns or unwinds.
- `new_waiter()` reserves one of the configured registration slots before provider construction.
  `waiter.reserve().await` borrows that token exclusively and accepts no payload. Dropping a pending
  acquisition unregisters its waker; dropping the token returns its registration slot. Capacity return
  wakes registered observers, which must repoll real capacity; no FIFO fairness guarantee is made.
- `guard.retire()` and `lease.retire(value)` enqueue synchronously and return a runtime-independent
  acknowledgement future. Dropping that future cannot cancel cleanup. Successful acknowledgement
  follows destruction and slot release; a panicking destructor reports `RetirementPanic`.
- `into_parts()` supports worker-only type transitions. The caller must retain the returned lease
  through raw execution/destruction, or immediately rewrap transformed output. There is deliberately
  no `into_inner()` method that would release the reservation before destruction finishes.
- Nested cleanup in the same domain runs directly on its current cleanup worker. Other domains keep
  their own worker affinity. No UI/executor inline fallback, thread per Drop, or destructor join exists.
- Live leases/waiter tokens retain worker availability after the last public domain handle is dropped.
  Once handles/leases/tokens are gone, the worker drains queued payloads and exits. Ordinary unwinding
  payload panics are contained; this does not claim recovery from process abort/double panic.
- Cleanup waits have no finite completion promise for an arbitrary blocked provider destructor.
  Its slot stays occupied, bounding retained ownership instead of admitting replacement work.

## Evidence

All outputs are under `_build/qa/life01-retirement-20261112/`.

1. `primitive-red.log`: the first std-only regression failed with `provider dropped on caller` against
   a minimal inline-destruction skeleton (1 failed). This is a primitive RED, not a reproduction of
   an integrated Engine forced-shutdown failure. The first invocation lacked rustc on PATH; the
   recorded run uses the repository's pinned Windows Rust environment.
2. `primitive-green.log`: the corrected initial regression passed (1 passed).
3. `primitive-focused.log`: 13 primitive cases passed, including gated destruction, no-runtime Drop,
   saturation, nested cleanup, startup failure, panic containment and pinned-future ownership.
4. `primitive-waiters.log`: the 20 cases at that stage passed, zero failed/ignored/filtered. Compilation used
   `rustc --edition=2024 --test -D warnings`; no Tokio runtime, Cargo, UI build or native performance
   measurement was involved. `primitive-waiters-metadata.json` records compiler and source/executable
   SHA-256 values for this exact run.

The seven additional cases cover fixed waiter admission, wakeup, cancellation, waker replacement,
multiple registered waiters repolling actual capacity, waiter-retained worker lifetime, and worker exit.
Tests use channels/condition variables and manually polled futures; five-second waits are watchdogs,
not timing assumptions. The integration must still prove its slot ledger, early engine reservation,
actor handoffs and cancellation/runtime-shutdown paths before this mechanism is considered qualified
for application use.

Final primitive review found an acknowledgement observer lock boundary: replacing an old Waker could
run its destructor while holding the completion mutex. `primitive-waker-red.log` records the regression
failing; the implementation now removes that observer under the lock and destroys it after unlocking.
`primitive-final.log` records all 21 cases passing after the fix.

`primitive-clippy.log` preserves the initial strict lint refusal of the deliberate custom Wake fixture.
That fixture cannot use Waker::noop(): its final destructor is precisely the behavior under test. A
local documented `manual_noop_waker` allowance fixes only that fixture; `primitive-clippy-green.log`
records strict standalone Clippy success. The resulting executable passed all 21 tests again in
`primitive-qualified.log`, zero failures/ignored/filtered. `primitive-qualified-metadata.json` records
its exact source/test/executable hashes and compiler command. These are the final standalone results;
no shared integration source or Cargo build was modified/run by this primitive task.
