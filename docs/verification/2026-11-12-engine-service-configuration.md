# One-time engine service configuration

Baseline: `f0cf369`. This is the separate builder-replacement follow-up after report-arena and executable-artwork qualification.

## Reproduced caller-side cleanup

The original `Engine::with_*` methods assigned directly through `WorkerOwned<EngineServices>::DerefMut`. The outer guard owns final destruction of the bundle but does not intercept replacement of an individual field. All 17 optional service setters had this behavior; `with_network(None)` could release an existing last owner without adding another service.

The focused baseline integration compiled successfully and failed its last-owner assertion: Settings destruction and the calling test both ran on `ThreadId(2)`. No Tokio runtime was involved. Build/run took 5.93 seconds (Cargo compilation 4.01 seconds), exit 101, one expected failed test. Exact evidence:

- `_build/qa/engine-configuration-20261112/windows-red.log`
- `_build/qa/engine-configuration-20261112/baseline-red.rs`

The native closure boundary is concrete: `native_content::archive` retains an InstalledProvider. An independently constructed/loaded provider can be its sole owner, whose final FFI instance Drop calls plugin destruction and releases its library. The application's singleton and once-only composition often hide this path; they do not establish a safe public replacement contract.

## Bounded correction

Each `configure_*` call now requests an opaque token for a vacant service field before accepting an owner. Duplicate admission returns metadata-only `ServiceAlreadyConfigured`. The exclusive token's consuming `install` operation cannot fail, be reused or replace another owner. Abandoning a token leaves the field unconfigured. There is no callback that could already own a rejected replacement.

All existing optional services, including locale, remain absent by default. Unavailable network services simply skip configuration. Required filesystem and wakeup construction retain their existing API. Accepted service owners remain under the existing startup-reserved service guard, with no extra retirement slots, workers or accumulated displaced configurations. This change alone preserves the baseline 304-slot/seven-waiter ledger. The separate native tree Rename integration adds four guarded capture slots; the combined current ledger is 308 slots and seven waiters.

The internal Rust composition API migrates from setters to explicit admission/install; the plugin ABI and operation authority are unchanged. Arbitrary native destructor blocking remains possible on the existing worker and never becomes a synchronous caller fallback. Independently retained external service Arcs remain their holders' responsibility.

## Qualification status

The focused Linux configuration integration passed **5/5 tests in 0.06s**, after a 1m07s build. They cover rejection before a second owner is constructed, original-owner retention, unused-token reuse/default locale, optional network admission, a gated owner containing a real temporary file and 2 MiB allocation, and configured ownership surviving an unpolled run plus runtime destruction. Field order acknowledges completion only after the real file/allocation fields have dropped. The blocked cleanup regression uses a separate dropper thread, bounded observations and an RAII release on test unwind, so a regression does not strand the test.

The first Linux GREEN attempt stopped before tests because a call-site migration had removed the crate qualifier from desktop.rs imports. The integrator restored the qualifier without changing runtime behavior. The failed compiler output is preserved at `_build/qa/engine-configuration-20261112/linux-first-compile.log`; the identical focused gate then passed.

The existing Linux `engine_lifecycle_tests::` filter passed **8/8 in 0.09s**, after a 1m09s library build (978 tests filtered). This covers unstarted/unpolled execution, full-outbox dispatch, interrupted tree shutdown, late command permits, normal shutdown acknowledgement and runtime shutdown. The combined script exited 0. No zero-match configuration library filter was counted.

Evidence:

- `_build/qa/engine-configuration-20261112/linux-configuration-green.log`
- `_build/qa/engine-configuration-20261112/linux-lifecycle-green.log`
- `_build/qa/engine-configuration-20261112/linux-focused-status.txt`
- `_build/qa/engine-configuration-20261112/linux-fixture-root.txt`

The Linux run used the existing offline SDK, existing `_build/linux` target and a fresh `/tmp/.../_build/tmp` fixture root. Windows configuration integration also passed all 5 tests in 0.00 s. The combined full engine gates passed 1,331 Windows and 1,308 Linux tests; affected UI gates, strict all-target Clippy and independent ownership review passed. The [combined checkpoint](2026-11-12-config-rename-cmd-checkpoint.md) records exact scope and the fresh debug runtime. This does not close every LIFE-01 follow-up.
