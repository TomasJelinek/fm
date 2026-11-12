# Report arena retirement implementation

This batch implements the native ownership correction described in
[the original reproduction](2026-11-12-report-storage-retirement.md). The original Linux RED remains
preserved: the last `FolderSize` warning destroyed its actual `AppendSpool` on the caller, and the
last `TreeDiagnostic` text prefix closed its anonymous file on the caller. Neither original-service
anchors nor a particular output consumer's retirement queue covered every final-owner path.

## Ownership and resource bounds

`EngineLifetime` starts one fallible report cleanup worker before native services are accepted and
exports the explicit `report_arena()` factory. One additional startup retirement lease protects the
shared arena registry. There is no per-report retirement-domain slot or arbitrary report-count cap.
Producer constructors receive a cloned factory explicitly; the public directory-tree constructor
also requires one. No report producer silently starts a cleanup thread.

Before the first native file is opened, `ReportLog` reserves a reusable arena node. The registry
retains the actual storage, while writer and immutable text handles share one final-owner token.
Raw storage and `SpoolPrefix` fields precede that token in destruction order. Final token Drop for accepted native storage only
links its already-admitted index into an intrusive queue and wakes the startup worker: no filesystem
work, allocation, runtime spawn or thread join occurs there. There is at most one pending entry per
accepted file. Node storage is reused after actual destruction; metadata capacity follows the
high-water mark of admitted files rather than a growing list of completed payloads. An unused
reservation returns its empty metadata node immediately, so cancelled/failed creation cannot build
an empty-node backlog behind blocked native cleanup.

The worker removes an owner under the metadata mutex, releases the mutex, then destroys the native
resource. A blocked close therefore does not hold the registry lock or make its node reusable.
Each destructor unwind is contained. Normal final-factory Drop wakes the worker without joining it;
existing report/prefix owners remain valid independently of the engine lifetime.

Unexpected worker exit marks admission unavailable. A reservation obtained before that exit can
still adopt its native result safely; later reservation attempts fail before file creation. Existing
tokens keep the startup-guarded registry alive, so its eventual fallback destruction occurs on the
generic retirement worker. This exceptional path may delay cleanup until remaining owners vanish;
it does not accept more files or move final native destruction onto the caller. The fallback also
contains individual destructor panics while releasing other files.

Every report retains its own append file, complete records, immutable byte boundaries and independent
storage failure. The fix does not introduce shared-file indexing, session-long retention of normal
completed reports, warning truncation or relaxed private-spool validation. Report construction,
recording and reads still belong on backend workers. Existing native rejected-send protection and
UI retirement queues remain in place.

## Qualification

The permanent tests include the two actual-file RED regressions, public startup-factory lifetime,
completed-but-unconsumed native output after runtime shutdown, and unpolled work destruction.
Ten focused arena cases cover startup failure, unused reservation reuse, raw/prefix token retention,
blocked cleanup, final-factory nonjoining Drop, destructor panic, worker exit/unwind, adoption after
exit, fallback cleanup, and failed private-file creation. Existing direct warning tests preserve
complete Unicode text, partial-write honesty, immutable ranges and cancellation; an added case
checks independent file failures within one arena.

Qualification is in progress. Logs, exact native Linux fixture root and artifact hashes are under
`_build/qa/report-arena-20261112`. The first shared compile found one stale archive-size fixture
signature; it was corrected by its owner, and the original compiler evidence is retained separately
as `linux-first-compile.*`. No report-arena compiler defect was reported in that attempt.

### First targeted results and bounded-node correction

The first coherent Linux build completed in 1m12s. Its report arena 10 and size-warning 18 tests
passed, including both original actual-file RED cases. Direct producer filters passed:
`tree_diagnostics` 2, `folder_presence` 6, `folder_sizes` 36, `archive_sizes` 32,
`providers::tree` 1, `providers::tree_tests` 17, `tree_controller` 8,
`tree_actor::size_capture_tests` 8, `tree_actor::size_runtime_tests` 8 and tree refresh 22.
The provider and sizing filter names were verified against `--list`; an earlier `tree_sizes`
selection matched zero tests and is not evidence. Five integration executables passed 57 tests
with one existing ignored case: folder sizes/warnings, selected sizes, tree controller and filters.
Windows first report-filter qualification passed 58 tests in 0.80s after a 32.68s compile.

Self-review found that unused reservations initially queued empty nodes behind a blocked native
close. The strengthened gated regression reproduced growth to three nodes where two suffice.
An exact-source `rustc` harness, including production `report_arena.rs` and `retirement.rs` and
linking the already-built platform dependency, recorded 9 passing and 1 failing arena tests.
Returning an empty reservation directly to the free list made all 10 pass; accepted native files
still wait for actual destruction before reuse. A first harness launch used a former WSL temporary
root that no longer existed; that separate fixture failure is retained as `arena-red-stale-tmp.*`.
Both conclusive runs created fresh native temporary roots. Logs are `arena-red.log` and
`arena-green.log`; no engine source was duplicated into the harness.

At that intermediate checkpoint, combined-source engine qualification was pending. Initial Linux Clippy
encountered the intentionally incomplete icon/desktop protocol migration and was stopped only after
all native tests had completed. Its interrupted output is not a successful lint result. A fresh
strict gate is required once that separate source work is coherent.

### Final combined test gate

After the bounded-node correction and producer/icon migrations were frozen, the combined Linux
engine/platform build completed in **1m56s**. All report and producer filters listed above passed
again, including both original native file regressions. The five integration targets again passed
**57 tests, one existing ignored case**. Linux platform icon tests passed **19/19 in 0.07s**.
The fourth command-icon admission test landed after that compilation; a final narrow pickup rebuilt
the engine in **1m17s** and passed **4/4 in 0.08s**. Its exact executable hash is recorded in
`linux-final-command-icons.sha256`; the combined artifacts have their own hash manifest.

Strict lint first reported a derivable `Default` in the new command-icon actor; its owner corrected
that declaration. The retry reached one existing Linux discovery fixture using numeric `0` for Unix
permissions, which the current toolchain requires to be written as `0o0`. Both original lint outputs
are preserved. No report-arena lint issue was reported. The Linux target was handed to the platform
owner for that semantics-preserving fixture correction and final strict-lint retry before the
separate settings CI reproduction. Final Linux engine/platform all-target Clippy subsequently passed in 1m20s; see `linux-final-clippy.log`.

Two independent source reviews found no further issue in arena reservation/adoption races,
worker-exit fallback, raw-prefix/token ordering or cleanup lock boundaries. The report implementation
is frozen; unrelated icon/settings work does not change the report ownership contract.

Final Windows combined qualification passed 988 engine unit tests (7 existing ignored) and 57
integration tests (1 existing ignored). Native executable-artwork tests passed 2/2 in 0.09s.
Strict Windows core/engine/UI/platform/app all-target Clippy passed in 1m11s. Logs are
`windows-final-{tests,icons,clippy}.log` under the same QA directory. A fresh independent
read-only ownership review found no actionable issue; it did not run additional tests.
