# Transfer stage retry and cleanup retention, 11 November 2026

This change fixes two stage-ownership gaps and one bounded-history gap in the
existing Copy/Move implementation. It adds no metadata or link capability and
changes neither verification policy nor native source-removal authority.

## Reproduced defects

1. A cross-device Move stage could fail its initial validation probe and then
   refuse cleanup. The worker dropped that stage, but still offered Retry because
   it retained the source. Eight retries created eight separate stage files; a
   later successful probe forgot the first residual. Cancel could not clean that
   abandoned stage even after the injected cleanup refusal cleared.
2. Required-modified-time preparation similarly discarded a failed stage after
   cleanup refusal, then offered a fresh preparation Retry. A successful later
   attempt could erase the earlier cleanup diagnostic.
3. After 17 earlier skipped probe failures filled the 16-detail error history,
   Skip treated the current stage cleanup refusal as an ordinary error. The
   actionable residual path was omitted from terminal history.

The first gate observed four expected failures and two passing controls. After
fixing stage ownership and preparation admission, those six passed and a seventh
regression reproduced the bounded-history failure before its fix.

## Implemented contract

An active Move keeps the exact failed-probe stage in its existing `MoveWork.stages`
map. Retry revalidates that same stage; it does not allocate another one. Source
revalidation still precedes retry work. Skip and Cancel invoke the existing owned
stage cleanup operation. No pathname-derived cleanup authority was introduced.
A Skip cleanup refusal uses the existing critical diagnostic path, retaining the
current residual even when older error details fill the history.

Metadata preparation cannot retain its abandoned Job across attempts. Its private
`StageFailure` therefore distinguishes a cleanup refusal from an ordinary
recoverable probe error. A cleanup refusal records the consolidated native error
and exact stage path before the Job is dropped, and ends preparation without a
fresh Retry. Sources remain intact. Cancellation still yields Cancelled and keeps
the diagnostic; otherwise the outcome is Failed. Ordinary probe failures whose
stages were cleaned still offer Retry. Terminal reporting preserves the already
recorded cleanup error once, including its original native classification and
other residual paths included in its diagnostic.

An OS refusal is not successful cleanup: the stage can remain on disk and needs
attention at the reported location. These changes preserve ownership/reporting;
they do not claim that the application can force the OS to remove a refused
object. Published destinations and source-finalization proofs remain unchanged.

Implementation is confined to `copy_worker.rs`, `copy_metadata_worker.rs`,
`copy_decisions.rs`, and `copy_operations.rs` in `crates/midnight-engine/src`.
`copy_stage_retry_tests.rs` contains seven new regressions; `copy_tests.rs`
registers them, and the existing cleanup-contract test reads the private typed
failure's native error.

## Verification

The pinned environment was loaded from
`_build/qa/navigation-ordering/windows-env.ps1`, using the repository `_build`
target, offline locked dependencies, debug information disabled, incremental
compilation disabled, and a fresh TEMP/TMP directory below this QA run. No obsolete
SDK registry override was used.

| Gate | Observed result | Evidence |
| --- | --- | --- |
| Original six regressions on unchanged production | 4 failed as expected, 2 passed; build 23.14 s, tests 0.07 s | `_build/qa/transfer-stage-retry/red.log` |
| First fixes plus full-history regression | Original 6 passed; full-history Skip failed as expected; build 21.30 s, tests 0.04 s | `_build/qa/transfer-stage-retry/history-red.log` |
| Final affected Copy/Move suite | **142 passed, 0 failed, 0 ignored**, including all 7 new regressions; build 22.17 s, tests 10.15 s | `_build/qa/transfer-stage-retry/final-copy-green.log` |

Final command:

```powershell
cargo test -p midnight-engine -p midnight-explorer --lib copy_operations --no-fail-fast --locked --offline --target-dir _build -- --test-threads=2
```

The app package preserves the normal application dependency feature graph. The
fresh final TEMP path is recorded in
`_build/qa/transfer-stage-retry/final-temp.txt`. A launch that only waited on another
agent's Cargo lock was interrupted before compilation; it is not counted as a
verification result.

The recorded Windows runs created only uniquely owned files below
`_build/qa/transfer-stage-retry`. The fixture subsequently switched to honoring
QA-owned TEMP/TMPDIR so Linux qualification can use ext4 even when repository
sources live on DrvFs; combined platform gates reverify that fixture-only
portability correction. The tests use real native source pins and file I/O,
with narrow injected validation/cleanup refusals. Native cross-device eligibility
is injected through the existing fixture; no real second volume is claimed. No
real disk exhaustion, device removal, network outage, user files, or ACL changes
were used by these new cases. The broader existing recursive-Move suite retains
its established private-fixture ACL setup under the fresh QA TEMP; it does not
change user directory permissions.

The inspected Salamander reference remains
`D:/experiments/salamander-main/src/worker.cpp`: pending-I/O draining at
3726–3808, unfinished-output cleanup at 4733–4767, and separate Move removal
retry at 6069–6112 informed the boundaries. Its pathname deletion and ignored
cleanup failures were not adopted. See the existing
[transfer error reference audit](2026-11-08-transfer-error-reference-audit.md)
for provenance and the wider matrix.

This is affected Windows backend evidence. Combined application tests, strict
Clippy, and Linux parity are coordinated separately; this record does not claim
physical cross-volume or adverse-device qualification, or completion of the
remaining metadata/link requirements.

## Integrated qualification

The [combined batch record](2026-11-11-queue-batch.md) records final Windows
and Linux gates, strict checks, the matching debug build/runtime, and native/software
startup verification. It supersedes pending integration notes above.
