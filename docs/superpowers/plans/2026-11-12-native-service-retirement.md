# Native desktop, watcher and clipboard retirement

Date: 12 November 2026. Continues the authorized LIFE-01 audit after `b9f8ee4`.
Previous turn made verified progress: guarded Find/checksum/custom work was committed,
qualified on Windows/Linux and deployed. This batch implements the next inspected
ownership gaps; it does not redefine the whole application goal as lifecycle work.

## Design

Reuse the existing startup-reserved retirement domain and shared TaskLifetime.
Desktop anchors both asynchronous and native calls. Guard terminal/editor/opener
children before returning from native launch, sharing the existing bounded process
tracker implementation while retaining eight terminal/editor/custom slots and sixteen
independent opener slots. Guard captured editor targets through rejected and queued
work; the existing four launch permits bound accepted captures.

Watch retains each of its two Shared values inside an independently guarded Arc.
Native subscriptions stay on the thread where they were created. Clipboard similarly
owns its private bounded receiver before accepting requests; the receiver and native
service anchor survive cancellation before the worker starts. Clipboard sessions stay
strictly thread-local. Controller Drop cancels a started clipboard pump even while
its result queue is full. Rejected captured/generated clipboard requests use one
serialized retirement lease and acknowledgement instead of a cancellable Tokio
cleanup closure. Preserve native-before-service destruction and existing output
backpressure. No per-resource thread, unbounded cleanup queue or implicit child kill.

## Parallel work and integration

1. Desktop/process owner: reproduce cancelled/unconsumed native return and service
   defects, implement guarded launch/reaper/editor boundaries, run scoped tests.
2. Watch owner: reproduce pre-start/unwind captured-scope destruction, add two guarded
   Shared owners and both levels of service anchors, run scoped tests.
3. Root: reproduce clipboard Drop/pump and idle service defects; guard inbox/rejection
   and cancel on Drop. Integrate startup ledger and Engine call sites.
4. Independent reviewer checks actual handoff order, native affinity, fixed capacity,
   cancellation, completed outputs and panicking trait methods. Inspect current CI.
5. Once all implementations are stable, run affected suites and strict Clippy once
   per platform, build fresh debug, validate native/software startup, document and
   commit/push. Keep remaining findings explicit and full goal active.

Windows and Linux Cargo target directories each have one assigned owner at a time.
Regression tests use owned fake providers/gates; native destructive checks, if needed,
may touch only explicitly created test directories. No user files are removed.

## Result

The three implementations, independent review, affected Windows/Linux gates, fresh
debug build and ordinary/software startup checks are complete. See the
[qualification record](../../verification/2026-11-12-native-service-retirement.md).
LIFE-01 and the broader application goal remain open for the separately listed
actor and UI/performance work.
