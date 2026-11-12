# Checksum actor retirement

Approved scope: LIFE-01 checksum adapter/backend ownership. Preserve digest algorithms,
source revalidation, limits, cancellation, generation identity and manifest authority.
Root owns shared EngineLifetime/lib startup integration; this task owns checksum files
and their narrow integration fixtures. No commit or push from this task.

1. Preserve RED evidence for a dequeued adapter capture blocked on the outbox, a
   second queued capture, an unpolled actor service at runtime shutdown, and backend
   Work waiting on the process-wide worker permit.
2. Add startup checksum capabilities: 36 payload slots (32 admitted jobs, one serial
   captured intake/rejection, two manifest saves, one shared guarded service) and one
   fixed waiter. Acquire before provider adoption; pre-acquisition captured input
   remains within the guarded EngineRun. No ad hoc cleanup domains in production.
3. Carry one job lease through captured admission, preparation output, backend queue,
   native execution and disposal. Retain native pins before capture/provider release,
   and captured source before quota/lease release, including error and unwind.
4. Carry the guarded service anchor through backend and manifest closures. Retain
   save ownership and mutation permits until native cleanup; release save capacity
   after retirement acknowledgement. Plain digest result/event data stays unchanged.
5. Add completed/unconsumed preparation and queued blocking cancellation coverage,
   plus gated destructor/quota ordering. Run focused RED/GREEN and existing checksum
   boundary/manifest/native integration tests on Windows and Linux with coordinated
   Cargo leases. Record exact evidence and remaining LIFE-01 limits separately.

Qualification complete: Windows full engine/affected integrations and Linux scoped
shared gates passed, including strict engine all-target Clippy on both platforms.
See [the checksum verification note](../../verification/2026-11-12-checksum-retirement.md)
for the preserved REDs, the explicit queued-cancellation fixture correction, final
counts, artifact identities and scope limits. Root retains integration/commit ownership.
