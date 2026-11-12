# Dynamically discovered network provider ownership

Base: b3fa92e. Scope: LIFE-01 Network Neighborhood discovery and request owners.
Root owns startup integration and the final engine batch. This note does not
claim closure of the remaining actor inventory.

The discovered provider is created after EngineServices. The concrete native
client owns an FFI Instance whose destructor calls the plugin destroy callback,
then releases or quarantines its library according to the unchanged ABI result.
A TaskLifetime retaining only original services could not protect that new owner.

The controller performs exactly one discovery and writes one OnceLock; there is
no retry or replacement path. One startup payload reservation therefore covers
its sole native instance. No waiter, per-request retirement slot, new cleanup
thread or implicit process termination is added. The two-request semaphore stays
independent and continues to cover the native call, including a detached call
whose runtime has stopped. A future discovery-retry feature must revisit this
single-instance admission argument.

Successful creation now constructs the private cloneable
`Provider(Arc<WorkerOwned<Arc<dyn NetworkNeighborhood>>>)` inside the native
closure before its result can enter a JoinHandle or OnceLock. Requests clone
only that guard, and Provider exposes only a borrowed trait reference. The same
reservation remains occupied until actual final native destruction completes.
Failed creation returns the unused reservation after its native cleanup. Pure
availability, error, address and listing metadata remains unwrapped.

Original locale/service references are retained by TaskLifetime in the controller
and in both async/native navigation layers. The production discovery factory
captures no original service; it carries its own startup reservation. Generation
checks, cancellation, partial listings, native path handoff and ABI failure/library
quarantine behavior are unchanged. No native-host or plugin source changed.

## Reproduced regression

A behavior-preserving private discovery factory/submission seam allows a test
provider to be constructed on the real blocking worker and then pass through the
actual controller pipeline. Its destructor records its thread and blocks behind
a gate. The factory captures only fixture metadata, never an already-created
native provider.

Before the ownership fix, all three baseline cases failed at the intended thread
assertion: **0 passed, 3 failed, 0 ignored in 0.14s**, compile 2m33s. Final provider
Drop ran on the caller for a completed/unconsumed discovery result, final
controller/OnceLock release, and an unpolled navigation after controller Drop.
Evidence: `_build/qa/network-provider-lifetime-20261112/linux-red.log` and its
`linux-red.sh` reproduction command.

The final fixtures additionally assert that the one-slot domain cannot reserve
again while gated native destruction is active. Additional cases cover native
creation cleanup on error and unwind, plain failure availability/classification,
a request holding its provider and one call permit after runtime shutdown, and
an explicitly aborted queued factory that never creates a provider.

Independent read-only review found no actionable issue in the one-instance bound,
native-result adoption, private guard propagation, original-service anchors or
seven test seams.

## Focused Linux qualification

The final `network_neighborhood::` filter passed **15 tests: seven new lifetime
regressions and eight existing behavior tests**, zero failed/ignored in 0.15s.
Compilation finished in 1m09s; the driver exited zero. The deliberate creation
unwind is caught by Tokio and asserted by its test; it is not a failed test.

Reproduction/evidence: `_build/qa/network-provider-lifetime-20261112/linux-green.sh`,
`linux-green.log` and `linux-green-status.txt`. The run used the existing native
Linux verification environment and SDK, locked/offline Cargo, two build jobs and
two test threads, with private TMPDIR
`/tmp/fm-network-provider-lifetime.kxRMl8FX/_build/tmp`.
The qualified engine test binary SHA-256 was
`c48054e5a020e0734846b6f326330a5c6173084ec016ca1419af3374227634ef`.

Owned-source formatting and diff whitespace checks passed. No native-host/plugin
source, dependency or ABI policy changed.

## Final affected Linux gate

After the File List owner explicitly froze its final source and fixtures, one
fresh build selected the engine library and seven directly affected integration
targets. The resulting binaries ran directly with the same native SDK/environment,
locked/offline Cargo, two build/test jobs and isolated TMPDIR
`/tmp/fm-network-final.1CQzNYyF/_build/tmp`. Every execution had a 180-second bound;
no full Linux library or unrelated native timing suite was run.

| Library filter | Passed | Time |
| --- | ---: | ---: |
| file_list | 30 | 0.19s |
| network_neighborhood | 15 | 0.16s |
| path_picker | 6 | 0.10s |
| shell_menu | 3 | 0.20s |
| volume_information | 3 | 0.08s |
| folder_presence | 6 | 0.09s |
| task_service_tests | 14 | 0.10s |
| engine_lifecycle | 8 | 0.11s |

| Integration | Passed | Time |
| --- | ---: | ---: |
| folder_size_warnings | 2 | 0.08s |
| folder_sizes | 20 | 0.12s |
| navigation | 7 | 0.22s |
| navigation_fallback | 2 | 0.18s |
| navigation_ordering | 5 | 0.08s |
| selected_folder_sizes | 5 | 1.35s |
| volumes | 5 | 0.13s |

All listed filters/targets had zero failed and zero ignored cases. The single
artifact build finished in 1m34s; strict engine all-target Clippy with `-D warnings`
finished in 1m32s. The final driver exited zero and recorded DONE. There are exactly
seven new network lifetime tests within the 15-case network filter, unchanged
from the focused GREEN run.

Reproduction/evidence is under `_build/qa/network-provider-lifetime-20261112/`:
`linux-final.sh`, `linux-final-progress.log`, `linux-final-build.log`,
`linux-final-lib-*.log`, `linux-final-integration-*.log`, `linux-final-clippy.log`,
`linux-final-status.txt` and `linux-final-executables.sha256`. The hash file records
all eight selected executables. Final Linux engine library test SHA-256:
`6c9522d8a4bc91a5819d6f44372c45245e5f30198d9ba4560a4f959d8bd03722`.

## Windows qualification

The root-owned final Windows gate passed the full engine library: **956 passed,
0 failed, 7 ignored in 6.55s** (test build 28.49s). All seven new network lifetime
cases appear as passing in
`_build/qa/remaining-native-results-20261112/windows-final-tests.log`.
The same seven integration targets passed **46 tests**, with no failures/ignored
cases. This is broader Windows engine evidence; the Linux run above deliberately
used scoped library filters.

Strict engine all-target Clippy passed in 19.77s
(`remaining-native-results-20261112/windows-clippy.log`). The debug application
and elevation-helper build completed in 1m20s
(`remaining-native-results-20261112/debug-build.log`). These paths are under
`_build/qa/`. Root separately owns deployment, native/software smokes and commit;
this task does not infer those outcomes from build success.

No commit or push was performed by this task; no Linux build/test process remains.
