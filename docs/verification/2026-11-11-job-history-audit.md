# Job history projection audit

Status: measured baseline and cached projection, 2026-11-11.
Qualification preserves session history and does not change the metadata result
storage policy. The final ordinary UI unit suite passes 889 tests with 4 ignored.

## Baseline repeated projection work

`operation_presentation::refresh` calls `job_presentation::rows` on every refresh,
including when the Jobs ledger is closed. The projection formats all retained
Copy/Move/mutation rows, including terminal details, diagnostics and recovery paths,
and sorts them by operation identity. `update_model` checks equality only after
that work has happened. Chart geometry has an existing snapshot/locale cache, but
this does not cache the rest of the row or its detail text.

Relevant source locations at review time:

- `crates/midnight-ui/src/operation_presentation.rs:16`: refresh and row projection.
- `crates/midnight-ui/src/job_presentation.rs:29`: all-history row construction.
- `crates/midnight-ui/src/job_presentation.rs:52`: chronological sort.
- `crates/midnight-ui/src/job_presentation.rs:59`: mutation details and metadata lookup.
- `crates/midnight-ui/src/job_presentation.rs:163`: Copy/Move details and diagnostics.
- `crates/midnight-ui/src/lib.rs:836`: model equality/replacement after projection.
- `crates/midnight-ui/src/job_speed.rs:21`: all-live-ID set for chart cache retention.
- `crates/midnight-ui/src/job_speed.rs:26`: chart reuse by snapshot pointer and locale.
- `crates/midnight-ui/src/job_progress_windows.rs:86`: window synchronization.
- `crates/midnight-ui/src/job_progress_windows.rs:97`: clone all rows and construct
  another live-identity set, even when no historical progress windows are open.

For H retained jobs and B bytes of formatted diagnostic/path text, the baseline row
construction performs O(H log H + B) work, followed by model comparison and linear
window/cache bookkeeping. Active backend progress is sampled every 250 ms
(`crates/midnight-engine/src/operation_throughput.rs:11`); ordinary presentation
refreshes also run this path. This establishes growth with session history, but
does not establish a user-visible latency threshold. Chart sample storage and
cached path strings also grow with retained history. No history cap should be
reintroduced merely to hide repeated projection work.

## Opt-in measurement

`crates/midnight-ui/src/job_presentation_tests.rs:337` contains the ignored test
`job_presentation::tests::benchmark_warm_job_history_projection`. It measures eight
warm `rows()` projections per case with 1,000 and 10,000 terminal Copy jobs:

- Ordinary details, and 16 retained errors plus 240 throughput samples per job.
- Unchanged history, and one job receiving current-file progress updates.

The chart cache is populated before each case. Fixtures share sample storage; this
is a projection-time benchmark, not a retained-memory benchmark. It does not
measure Slint rendering, model equality, mutation metadata details or native window
synchronization. Run the newly linked unit executable alone, without competing
compilers or native performance workloads, and retain its executable hash and log:

```powershell
& $unitExe --exact job_presentation::tests::benchmark_warm_job_history_projection `
    --ignored --nocapture --test-threads=1
```

The baseline run completed in a coordinated quiet slot at 2026-11-11 22:17 UTC.
The UI unit executable used opt-level 1 and SHA-256
`686045C5E6CBD29D00CCAB1D4E17DDFAE276CCB4110A5FA838C1AB089D3FC29E`.
The same executable passed 885 ordinary UI unit tests. Mean projection times:

| Jobs | Details | Unchanged history | One active job |
| --- | --- | ---: | ---: |
| 1,000 | Ordinary | 3.836 ms | 4.079 ms |
| 1,000 | 16 errors and 240 cached samples | 26.493 ms | 26.503 ms |
| 10,000 | Ordinary | 41.757 ms | 41.666 ms |
| 10,000 | 16 errors and 240 cached samples | 259.228 ms | 241.949 ms |

These warm projection costs establish a material long-session UI stall. They do
not measure rendering or end-to-end input latency. Exact artifact metadata and all
eight baseline results are retained in
`_build/qa/viewer-settings-20261111/job-history-benchmark-baseline.json` and `.log`.

The approved cache reuses derived Copy/mutation rows only when their exact progress
Arc identity, NumberLocale and optional CopyFailure/MetadataResult identity match.
Weak tokens retain allocation identity against address reuse, but do not retain
retired core payloads or take over backend reclamation ownership. Arc::make_mut
invalidates those tokens during optimistic Pause updates. Attention and image
handles are refreshed from current state on every projection; theme styling stays
in Slint. Cache entries are pruned with the retained job identities on Clear.

The cache adds one derived row plus locale and weak identity tokens per retained
Copy/mutation receipt. Slint strings/images in projected rows are shared handles;
rows returned to the model do not copy the full text. It does not cap receipts,
extend metadata result retention, cache native authority or change user-command
projection. Sorting, model equality and window synchronization remain separate
costs. The cache remains proportional to retained history; this change removes
repeated detail formatting without changing receipt retention.

## Cached projection measurement

The unchanged benchmark ran in a coordinated quiet slot at 2026-11-11 22:48 UTC.
The cached UI unit executable used opt-level 1 and SHA-256
`12511BBD2F81FA9AD8679975FF2DAC03E2B7ADFC9D405CA0C5C1023F5D1B14E9`.
All eight cases passed, with the same fixtures and eight warm iterations per case:

| Jobs | Details | Baseline unchanged | Cached unchanged | Baseline active | Cached active |
| --- | --- | ---: | ---: | ---: | ---: |
| 1,000 | Ordinary | 3.836 ms | 0.838 ms | 4.079 ms | 0.860 ms |
| 1,000 | 16 errors and 240 samples | 26.493 ms | 0.773 ms | 26.503 ms | 0.951 ms |
| 10,000 | Ordinary | 41.757 ms | 13.524 ms | 41.666 ms | 13.708 ms |
| 10,000 | 16 errors and 240 samples | 259.228 ms | 15.355 ms | 241.949 ms | 15.899 ms |

Exact artifact metadata and results are retained in
`_build/qa/viewer-settings-20261111/job-history-benchmark-cached.json` and `.log`.
The 10,000-row detailed cases remove approximately 226-244 ms of repeated projection
work. These timings do not establish end-to-end input latency or memory usage.
Sorting, live-ID sets, model comparison and window synchronization still scale with
history. In particular, Slint 1.18.1 treats two empty Image values as unequal, so
model equality may still replace otherwise unchanged rows while artwork is absent.
That pre-existing behavior is outside this cache change.

The benchmark artifact's ordinary suite initially found one test-oracle failure:
whole-row equality compared empty Slint Images. Local Slint source
(`graphics/image.rs`, `impl PartialEq for Image`) confirms that empty images fall
through to false. The corrected fixture checks shared text allocation and build
count while preserving snapshot/locale invalidation checks. Production cache code
and the timed benchmark were unchanged by this test-only correction.

The final rebuilt UI unit executable, linked at 2026-11-11 22:51:58 UTC, has SHA-256
`6B9F51E1F6AE29A5BB8CD91330F2D018E2668E8C424462D577AB9E7A20807726`.
Its full ordinary suite passed **889 tests, 0 failed, 4 ignored**, with an isolated
TEMP directory and `--test-threads=1`; reported test-body time was 8.81 seconds.
All four cache tests passed, including exact reuse, optimistic pause/locale
invalidation, late failure/metadata/artwork updates, weak ownership and Clear.
Evidence is retained in
`_build/qa/viewer-settings-20261111/final-corrections-ui-unit.log` and
`final-corrections-ui-unit-executable.json`. This unit-suite timing ran alongside
the application build and is not used as a performance measurement.

## Older metadata result detail

`crates/midnight-ui/src/presenter/operations.rs:296` still removes the oldest
`MetadataResult` after more than 16 results arrive. The job progress receipt remains
in session history, but its detailed metadata outcome text can disappear after
newer metadata jobs finish. `job_presentation::mutation` looks up that optional
result when building Details. This global result limit also keeps its linear
lookup bounded today; it is not currently an unbounded quadratic lookup.

Changing this limit requires a separate retention/memory review. Metadata result
payloads and progress receipts have different content and lifetimes. A follow-up
should specify what historical detail remains available, account for retained
memory, and test at least 17 completed metadata jobs plus Clear, late result events
and job identity reuse protection. Current qualification intentionally leaves this
storage behavior unchanged.
