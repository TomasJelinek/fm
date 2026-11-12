# Operation and Copy actor retirement

This note covers the actor portion of
[runtime-independent retirement](../superpowers/plans/2026-11-12-runtime-independent-retirement.md).
It does not claim that every independent engine actor or the complete LIFE-01 goal
has been qualified. The coordinating task owns final engine/app validation.

## Ownership and bounds

Each actor receives an `ActorRetirement` capability created before provider state
is accepted. Its contribution to the shared slot ledger is 32 accepted jobs, one
serialized intake/rejection, and one service context: 34 per actor, 68 together.
Intake waits use one startup-reserved waiter behind a mutex. While it waits, the
incoming request remains owned by the enclosing guarded `EngineRun`; constructors
do not create fallback cleanup workers or change operation admission authority.

Operation inputs transfer their existing lease into blocking preparation and then
into `WorkerOwned<Job>` before the preparation output is published. The same guard
survives Preview delivery, confirmation storage, metadata preview, queueing,
predecessor/mutation waits, disposal and unconsumed worker results. Disposal futures
own the guard before their first poll. Native target fields precede the retained
provider capture; the mutation permit is the final Job field.

Copy uses one envelope for its current payload, completion receipt, mutation permit
and admission permit, in that destruction order. Ownership transitions retain the
same lease through input, retained preparation, retry and Job. One-shot queue
receivers already contain guarded envelopes, so consumer destruction only closes
and drops them; it no longer submits cleanup to Tokio. Preparation owners close on
the worker before Move can proceed. Verification, same-volume rename, confirmation
and source-removal execution are unchanged.

Worker helpers unpack their payload in a statement after binding the lease. This
makes payload locals unwind before the lease on errors/panics. The slot therefore
cannot become available while a remaining locale/provider destructor is blocked.
Normal retirement acknowledgements and aborted guards use the same domain.

## Evidence

All logs below are under `_build/qa/runtime-retirement-20261112/`.

- `actor-red.log`: two original actor tests failed because native pins were dropped
  on the executor. Boundaries were a prepared confirmation and a locally held Job
  blocked publishing preparation progress into a full output channel.
- `actor-green.log`: those cases, an unconsumed completed preparation result, and
  manually polled pending predecessor/mutation waits passed (4 tests, 0.02 s).
- `copy-scope-green.log`: with no Tokio runtime, a gated native destructor held the
  provider, both permits and the domain slot until release (1 test, 0.01 s).
- `copy-rejection-green.log`: the existing real rejected-send boundary retained
  capacity and withheld terminal delivery until source destruction (1 test, 0.04 s).
- Independent review identified a remaining locale unwind edge. In
  `locale-order-red.log`, both ordinary failure and injected provider panic returned
  the slot while the final locale owner was still blocked: `[(false, true),
  (true, true)]`. Two-stage destructuring corrected that order.
- `actor-final-green.log`: **209/209** tests passed in **6.11 s** with filter
  `operations::`, including all five actor retirement tests, Copy envelope coverage,
  native/captured Delete, Copy/Move verification, retry, metadata and cancellation.
  These totals overlap the earlier focused runs; they are not additive.

The Windows actor-regression executable was
`_build/debug/deps/midnight_engine-ebcdb496ad5aeb66.exe`, SHA-256
`b4d7d43e97abdf01354f418f561f12de6a74beaf645be33dd1657d28b48a6d35`.
The command used the existing Windows toolchain/SDK configuration with locked,
offline dependencies:

```text
cargo --config <existing worker SDK registry.toml> test --target-dir _build
  --locked --offline -p midnight-engine --lib operations:: -- --nocapture
```

Tests used the same-user private TEMP path recorded in
`_build/qa/tree-delete-20261112/validation-temp.txt`; no existing private-directory
ACL was changed. Rustfmt and scoped diff-whitespace checks passed. Independent
actor review found no further last-owner or premature-admission issue after the
locale finding; final combined lint/platform qualification remains with root.
No independent commit was made.


## Final Linux qualification

Logs and the exact `linux-final.sh` driver are under
`_build/qa/life01-engine-20261112/`. Ubuntu 24.04 used the existing native Linux
verification environment, the existing worker SDK registry configuration, two
Cargo jobs, and the independent `_build/linux` target. `TMPDIR` was a fresh native
`/tmp/fm-life01.*/_build/tmp` fixture recorded in `linux-fixture-root.txt`.

The first lifecycle attempt passed seven cases and blocked in a fixture that
synchronously dropped Tokio while the still-owned EngineRun retained blocking tree
lanes. The original log is preserved as
`linux-engine_lifecycle-before-bounded-shutdown.log`. Root changed only the
already-polled shutdown fixture to the application's bounded runtime shutdown
path; the unpolled synchronous-drop regression remains unchanged.

Strict all-target Clippy then found the operation constructor's eighth argument
and a complex test helper return type. The constructor now takes named
`operations::Services { mutation, locale }` with the same guarded field destruction
order. The helper uses a named `PreparationTask` alias. No lint suppression was
added. The original diagnostics are in `linux-clippy-before-service-group.log`.

The final source passed:

- `linux-clippy-final.log`: strict core and engine **all-target** Clippy, 2 m 21 s.
- `linux-archive-size-sort.log`: **4/4**, 0.12 s, including explicit retirement
  acknowledgement before checking the released archive ownership.
- `linux-final-engine_lifecycle.log`: **8/8**, 0.07 s.
- `linux-final-retirement.log`: **37/37**, 0.36 s.
- `linux-final-operations--.log`: **200/200**, 7.21 s.

The test filters overlap and must not be added together. Platform-specific cases
account for the lower operation count than Windows. Clippy used:

```text
cargo clippy --config _build/qa/elevation-linux-20261111/sdk.toml --locked --offline
  -p midnight-core -p midnight-engine --all-targets -- -D warnings
```

The archive Cargo test rebuilt the final linked executable; the other three
filters ran directly against that same fresh binary. Its SHA-256 is
`c9025fa560f97f74735d1ee019ce7b6f1d5c7183434e6c9c6cb989481b229c8e`, recorded in
`linux-final-test-binary.sha256`, for
`_build/linux/debug/deps/midnight_engine-0ffc535384c44315`.
`linux-final-exit-status.txt` records zero. The Linux slot was released to the
separate compact-PDF qualification after this driver finished; no extra Linux
build or native timing run was left active.
