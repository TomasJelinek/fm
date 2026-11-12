# Transfer ETA work accounting

Status: implemented; focused and full affected Windows unit suites passed on
2026-11-12 (local time).

The authorized follow-up adds estimates for sampled Copy and cross-volume Move.
It changes presentation accounting only: no new content reads, weaker verification,
source-removal permission, automatic retries or changes to native rename.

## Design and evidence

For one file of length L, the readback budget V is L for FullSha256 and
min(L, 3 * 64 KiB) for SizeAndSamples. `transfer_edges.rs::ranges` uses disjoint
first/middle/last ranges; files up to 192 KiB are compared completely. Samples are
captured during the ordinary source stream, with no additional source read.

`VerifiedFileCopy` writes L bytes then independently reads V destination bytes.
`move_worker.rs::validate` calls `VerifiedFileProof::begin_validation` before source
capture and again afterward. Each proof pass reads V source bytes and V destination
bytes (`transfer_proof.rs::advance`). Thus Copy has L+V byte-work and cross-volume
Move L+5V. Metadata checks, sync, namespace changes and cleanup have no byte budget;
the estimate is approximate and can return to Calculating while those remain.

Aggregate file length cannot recover a sampled budget: many small files and one
large file can have equal total bytes but different sum(V). Discovery therefore
accumulates V per regular file using the existing metadata visit, without retaining
a per-file list or adding I/O. A private worker accumulator records published V and
V whose source was successfully removed. The public progress snapshot exposes only
optional remaining write/read byte counts, never execution authority.

Active Copy work comes from the retained VerifiedFileCopy. Active Move proof credit
comes from the actual phase and current proof's validation_bytes, not cumulative
job I/O. Before credits its current pass; Capture credits one complete source/target
pass; After credits that first pass plus the new current pass; Finalize credits both.
When source removal is retried, After starts with reading=false, so the repeated
pass is outstanding again even before begin_validation resets the proof counter.
Completed publication/removal credit is recorded exactly once at those boundaries.

Unknown or invalidated discovery, overflow, underflow, skipped/failed work, or an
unsupported proof state yields no byte ETA. Same-volume/mixed native rename retains
SelectedRoots discovery and the existing measured item ETA, without recursive work.
Paused, waiting and terminal operations suppress ETA. Current transfer rate measures
writes and current verification rate measures initial readback plus Move proofs,
matching the two remaining-work categories. Their sum is the observed byte-work
rate used for this approximate estimate; future I/O speeds are not guaranteed.
Cumulative retry I/O contributes to observed activity, never completion credit.

## Implementation plan

- [x] Add a core policy readback-length helper and optional remaining-work snapshot;
  add a private worker accumulator and discovery/publication/removal hooks.
- [x] Derive active proof credit by phase, including retry reset, and project ETA
  from matching measured activity. Keep same-volume item ETA unchanged.
- [x] Add targeted budgets, real Copy/Move/retry boundary and meter regressions;
  cover sample boundaries, multiple files, overflow, changed totals, pause and errors.
- [x] Run focused tests and strict lint under the coordinated Cargo lease; retain
  evidence here. No commits and no native performance workload during compilation.

The native Move regression runs on one owned test volume: existing CrossDeviceService
injects rename refusal while retaining real OS identities. A cfg(test)-only discovery
entry point selects recursive scope for this fixture; production native eligibility
and SelectedRoots selection are unchanged. Content reads, publication, proof,
source capture/removal and cleanup use the real native implementation.

## Qualification

Evidence is retained under `_build/qa/transfer-eta-20261112/`:

| Scope | Result | Evidence |
| --- | --- | --- |
| Core policy boundary test | 1 passed | `core-tests.log` |
| Real Copy/Move, discovery, budgets and removal retry/restore | 14 passed | `engine-tests.log` |
| ETA and existing progress meter tests | 8 passed | `ui-meter-executed.log` |
| Selected production libraries, strict Clippy | Passed with `-D warnings` | `clippy.log` |

The core test covers empty/small files, the 192 KiB boundary and u64::MAX. The
native Copy test covers empty, small and large files together. Both Move policies
are checked after every step against actual initial readback and both proof-pass
byte counts. The removal-retry fixture verifies the entire After pass becomes
outstanding before its old counter is reset, and retains the actual source-removal
and untouched-future-file assertions. Native same-volume zero-content-I/O tests
remain passing. Meter tests cover decreasing sample/proof work, retry growth,
unknown work, invalid rates, pause, waiting, errors and native item estimates.

Exact executables:

- Core SHA-256: `A83A0C285F29F737FAE45F50E8B7075F9CC77626FBC50F4D4A79FD2821646677`.
- Engine SHA-256: `400B31FB314496EBBBD369B2DE062B6BC9D912CD7941BF264C8CA3DEB014FD96`.
- UI SHA-256: `650BC53F3790AACB2D7CA3D65C9EB2CE86CFD1F93EEA9C539949ED820D58B3E1`.

`engine-artifact.json` and `ui-artifact.json` record paths, timestamps and private
TEMP directories. The first UI Cargo invocation used the source filename as its
filter and executed zero tests; its build log is retained as `ui-meter-tests.log`.
The executable's actual module was verified with `--list`, then
`job_presentation::meter::` executed all eight tests in `ui-meter-executed.log`.
No zero-test invocation is counted as test qualification.

Clippy selected the stable application/UI/viewer/platform/elevation package union
plus core and engine with `--lib -- -D warnings`; all selected libraries passed.
Two pre-existing warnings in the vendored femtovg dependency remain outside these
selected-package lint results. No Slint, dependency or transfer security-policy
changes were made. Independent reviews checked the sample formula, phase credit,
retry reset, publication/removal boundaries and same-volume scope with no findings.

After implementation and reviews were frozen, the already-built affected unit
executables also ran once without filters or ignored tests enabled:

| Full unit suite | Passed | Failed | Ignored | Test-body time |
| --- | ---: | ---: | ---: | ---: |
| midnight-core | 61 | 0 | 0 | 0.13 s |
| midnight-engine | 833 | 0 | 7 | 34.59 s |
| midnight-ui | 893 | 0 | 4 | 9.10 s |

These totals include the focused cases above; they are not additional independent
test counts. Logs are `full-core.log`, `full-engine.log`, and `full-ui.log`.
`full-unit-results.json` records exact hashes, timestamps, fresh per-suite repo TEMP
roots, pinned RUSTC, exit codes, summaries and validated total counts. Each suite
reported zero filtered tests. No Cargo or native performance measurement was run
for this full-suite check; the durations are evidence only, not performance claims.
The old logs, including the initial zero-test UI filter, remain preserved.
