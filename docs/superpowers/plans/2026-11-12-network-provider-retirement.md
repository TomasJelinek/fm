# Dynamic Network Neighborhood provider retirement

Approved task scope: LIFE-01 dynamic network provider ownership at b3fa92e.
Root owns shared EngineLifetime/lib startup integration. This task owns the
network controller, focused fixtures and evidence; no commit or push.

The controller performs one startup discovery with no retry/replacement path.
Reserve one payload slot before native creation, with no waiter. A successful
factory result must become a private cloneable guard inside the native closure
before it can enter a JoinHandle output or OnceLock. Every navigation clones that
same guard rather than cloning the raw provider Arc. Failed discovery returns the
unused slot after native cleanup; failure publication remains plain metadata.
Original locale/service references use TaskLifetime at both async/native layers.

1. Extract a narrow injectable discovery factory/worker submission seam without
   changing behavior. Add deterministic REDs for completed/unconsumed discovery,
   final OnceLock release and unpolled navigation after controller retirement.
   Factories create providers on the worker; no raw provider enters their capture.
2. Add `Retirement { provider: RetirementLease }`, capacity one, and the private
   guarded Provider type. Coordinate root constructor/startup integration only
   after baseline regressions compile and fail at the intended thread assertions.
3. Adopt successful discovery inside the native closure. Carry the same guarded
   provider through publication, retained state, requests and independently
   surviving native work. Preserve the two-request semaphore and release rules,
   generation checks, failure classification and native ABI quarantine behavior.
4. Gate provider Drop and prove startup capacity stays occupied until actual
   destruction. Cover failed creation and a native request surviving runtime
   shutdown; rerun existing network behavior tests. Keep generated listing and
   error metadata unwrapped. Run coordinated targeted Linux qualification and
   obtain independent ownership review before declaring this boundary complete.

Targeted and integrated Linux qualification completed: three observed baseline
failures became GREEN, all seven lifetime and eight existing network tests passed,
and the final affected filters/seven integrations plus strict engine all-target
Clippy passed. Independent review found no actionable issue. Exact commands,
counts, timings and artifacts are in
[the verification note](../../verification/2026-11-12-network-provider-retirement.md).
The root-owned Windows gate also passed 956 engine tests (seven ignored), 46 affected
integration cases and strict engine Clippy; all seven new lifetime cases passed.
Root owns deployment/commit qualification; source remains frozen.
