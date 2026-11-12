# Verified-copy concurrency on the development PC

Date: 10 November 2026. **Decision: no-go for implementation from this run;
confirmation is incomplete.** No production concurrency change has been made.
Completed measurements show substantial gains, but repeated abnormal audit delays
prevented completion of the predeclared qualification. This is not a finding that
parallel copying has no benefit. It implements the user's instruction to measure
this computer first and require reliable evidence before adding the feature.

## Decision rule

The [benchmark specification](../superpowers/specs/2026-11-10-ssd-transfer-scheduling.md)
sets the rule before measurement: at least 1.20 times full-SHA-256 throughput in
both large and mixed workloads, confirmed in seven interleaved rounds, with at
least five pairs above 1.10. An additional large-file shape must retain the gain;
the small-file workload must not slow by more than 5%. All contents, publication,
cancellation and cleanup checks must pass. A favorable single run or a faster
hash-only benchmark is insufficient.

Here, a 1.20 throughput ratio means 20% more bytes per second and 16.7% less elapsed
time. A positive component result permits planning an integrated scheduler; it
does not establish that the application already copies files concurrently.

## Machine and test scope

- Windows 11 Enterprise 10.0.26200, Intel Core i7-13800H, 14 cores / 20 logical
  processors, approximately 31.6 GiB RAM, Balanced power plan.
- Before confirmation, Windows reported AC power Online and battery at 100%.
  This was observed without changing the power plan.
- One SK hynix PC801 1 TB NVMe SSD, reported Healthy. C: and D: are NTFS partitions
  of **the same physical disk**, Disk 0. There is no second SSD to qualify.
- Owned source and destination fixtures are both under
  `_build/qa/ssd-copy-probe/fixtures/` on D:. No user files are copied or removed.
- The initial observation found approximately 104.6 GiB free on D:. Each trial
  checks space; the largest simultaneous source/output payload is 4 GiB. Owned
  datasets are audited and removed between workloads.
- Builds and other agent benchmarks were stopped before measurement. This is a
  normal running Windows desktop; unrelated system activity is not disabled.

Buffered I/O and normal caches remain enabled. Source generation, warmups and
independent audits warm the cache. These datasets fit RAM. Results describe this
cached, same-SSD copy/readback workload, **not** cold storage, sustained writes
beyond RAM, or copying between independent SSDs. The native method trace proves
overlap of calls for different files; it does not measure the device queue or
physical flash reads.

## What is timed

The standalone Rust harness uses the actual current Windows native source and
destination services and the current verified-copy state machine. It retains the
256 KiB buffer, streamed source SHA-256, destination sync, independent full target
readback, digest/length/revision checks, exclusive final publication and explicit
stage cleanup. Per-file opening, staging, copying, verifying, publishing and
closing are timed. Thread creation, source generation, extra independent audits
and deleting successful outputs between trials are outside the copy timer.

One baseline calls the unmodified production primitive with one worker. The
experimental primitive snapshots four common transfer source modules; its sole
semantic change is that each I/O call acquires one of the four shared permits
instead of all four. The original permit cost otherwise serializes the I/O even
when more threads exist. The experiment runs 1, 2 and 4 independent file workers,
without concurrent operations on any one file.

No shipped source, copy buffer, hash algorithm, application setting, dependency,
native Move policy or installed binary is changed. This harness does not include
the engine's recursive planner, UI, overwrite decisions or Move finalization.

Destination-root preparation is outside the timer; per-file preparation is inside.
Process CPU time covers the whole subprocess, so it must not be interpreted as
CPU time for the copy interval alone.

The binary uses an optimized release profile: `opt-level = 3`, fat LTO, one codegen
unit, symbols stripped, overflow checks enabled. Its SHA-256 is
`937BDB4A3C869F8AFBC1537612CF6F531F56AC2302578146FEB8D3A6C566B9CD`.
The recorded original and snapshot source hashes were independently rechecked
before measurement and all matched.

## Workloads and repetition

| Workload | Files | Payload |
| --- | --- | --- |
| Large | 32 x 64 MiB | 2 GiB |
| Mixed | 8 x 64 MiB, 512 x 1 MiB, 4,096 x 4 KiB | 1,040 MiB |
| Small | 8,192 x 4 KiB | 32 MiB |
| Larger individual files, if confirmation is justified | 8 x 256 MiB | 2 GiB |

Deterministically generated nonzero bytes have independent SHA-256 manifests.
Each variant has a warmup. Screening rotates 1/2/4-worker order across three
rounds; a promising result then receives seven confirmation rounds. Every
recorded run requires a complete source/output audit, matching file count and no
unpublished staging artifact. Failed runs cannot be silently discarded as timing
outliers. All raw trials are retained.

The historical CLI label `sustained` means the eight-file shape above. Its 2 GiB
payload does not qualify sustained media performance.

## Production control

The one-worker control passed its predeclared 5% agreement threshold:

| Round | Unmodified production, seconds | Experimental one worker, seconds |
| --- | ---: | ---: |
| 1 | 4.8590304 | 4.9198515 |
| 2 | 5.1535353 | 5.0658158 |
| 3 | 4.7493248 | 4.6623918 |
| Median | 4.8590304 | 4.9198515 |

The experimental median is 1.25% slower. No scaling correction is applied to the
subsequent results. All six measured controls passed the independent audits;
their maximum native-call overlap was one, with no write/readback overlap.

## Correctness before timing

The optimized harness ran three native test groups successfully:

1. Production and experimental 1/2/4-worker copies in full and sampled modes,
   followed by independent complete source/output audits.
2. Hold file A in destination readback: a second worker reaches file B's native
   write with the experimental permit cost; the unmodified exclusive budget
   prevents it until A is released.
3. Cancellation after actual write, sync and readback, using both verification
   modes and production/experimental execution. Unpublished partial files are
   removed, sources remain intact, and already published outputs remain complete.

These tests qualify the experiment's permit change. They do not replace the
production scheduler's future race, overwrite, dependency and Move tests.

After timing stopped, two additional native tests brought the suite to **5 passed,
0 failed**. One holds A inside destination readback, lets B finish, cancels
externally and verifies that the runner and partial file remain owned until A is
released and joined; both verification modes preserve completed outputs and remove
unpublished data. The other substitutes an owned test source directory with a
junction and requires disposal to reject it without removing retained bytes. That
guard test failed before the QA cleanup correction and passed afterward. Strict
release Clippy for the standalone harness and all its targets also passed.

The additional tests and cleanup guard were compiled only into a separate test
executable; the measured release executable's hash remains unchanged. Their
sources are distinct from the preserved `measured-source-v1/` snapshot. These are
QA harness changes, not application transfer changes.

## Results

Three-round screening produced these elapsed-time medians:

| Workload | 1 worker | 2 workers | 4 workers |
| --- | ---: | ---: | ---: |
| Large | 5.083 s | 3.112 s | 2.288 s |
| Mixed | 12.128 s | 6.826 s | 4.918 s |
| Small | 13.397 s | 10.124 s | 8.081 s |

Both candidates qualified for seven-round confirmation. All seven rounds finished
for the large and mixed workloads:

| Workload | 1 worker | 2 workers | 4 workers |
| --- | ---: | ---: | ---: |
| Large, 2 GiB | 4.859 s | 2.643 s | 1.969 s |
| Mixed, 1,040 MiB | 7.788 s | 5.655 s | 4.628 s |

Ratios of these time medians are 1.84x / 2.47x for the large workload and
1.38x / 1.68x for the mixed workload, for two / four workers respectively. These
describe about 46% / 59% less elapsed time for large files and 27% / 41% less time
for the mixed set. The gate uses the median of **per-round paired ratios**, a
different statistic:

| Workload | Workers | Median paired speedup | Range | Pairs at least 1.10 |
| --- | ---: | ---: | ---: | ---: |
| Large | 2 | 2.023x | 1.450-2.112x | 7/7 |
| Large | 4 | 2.672x | 1.977-3.459x | 7/7 |
| Mixed | 2 | 1.352x | 1.011-1.568x | 6/7 |
| Mixed | 4 | 1.730x | 1.599-2.111x | 7/7 |

These two workload groups pass their primary thresholds for both worker counts.
The complete gate does **not** pass: the small-file confirmation and final
eight-file shape are missing. The application remains serial.

### Interrupted confirmation

The screening small-file audits took 1.82-2.55 seconds. During confirmation, an
untimed `audit small` warmup was observed running for multiple minutes despite
continuing to read data. Its exact duration was not recorded; the gap between
observed process starts also includes intervening work. The first measured
small-file copy then took 21.604168 s
with one worker and completed its native full-SHA-256 copy state machine, but its
additional independent audit again lasted several minutes. This timing is
retained as **unaudited**, not included in any successful confirmation summary.

At 17:36:29 UTC the exact owned read-only `audit small` process was stopped after
159.84 seconds. Its recorded write count was zero. The runner then exited with
failure, appended a failed-audit record, and did not start further trials or
cleanup. The 8,192 source files and 8,192 final outputs remain as 64 MiB of owned
logical payload under `fixtures/small/`; there was no copying or source deletion
in progress when the audit process was stopped. No user process was terminated.

A contemporaneous system snapshot reported 100% total CPU and 2% disk time with
disk queue length one. This single snapshot does not identify the cause of the
delay. Process CPU totals in that snapshot are lifetime values, not current CPU
shares; they must not be used to blame a particular application. The experiment
does not establish a copy deadlock, content corruption or SSD failure.

The final counts are 6 audited controls, 27 audited screening runs and 42 audited
confirmation runs, plus one interrupted independent audit. The 42 confirmation
runs have matching full verification lengths and content audits, and every
multiworker run recorded distinct-file write/readback method overlap. The separate
review script reconciles the CSV and JSONL records and explicitly returns a
non-success result for the incomplete qualification. No anomaly is discarded or
replaced with a favorable rerun. There will be no production concurrency change
on this evidence alone; a future decision requires completing the agreed gate
under stable conditions, followed by integrated correctness and responsiveness
qualification.

The user suggested Microsoft Defender as the cause. A read-only check found
antivirus and real-time protection enabled, platform `4.18.26100.4`, engine
`1.1.26100.4`. Defender's Operational events in the affected interval contained no
event message matching the benchmark name or the two observed audit process IDs;
this does not exclude real-time scanning overhead. Cause remains unconfirmed.
Microsoft's [Defender performance analyzer](https://learn.microsoft.com/en-us/defender-endpoint/tune-performance-defender-antivirus)
can correlate scan costs with files and processes. Its recording command requires
administrator privileges, which the current Windows token does not have. No
security setting, exclusion or antivirus service was changed, and no Defender
performance recording has been collected.

Process CPU time is available in the trial records. The post-exit Windows process
query did not supply peak working set; blank values must not be presented as
measured RAM use. Copy buffers remain bounded by worker count in the inspected
implementation.

An independent source review confirmed the one-line permit change and found two
qualification gaps: source-subdirectory validation before fixture disposal and
cancellation while a native call remains held. Before confirmation, the runner
added untimed no-reparse/containment checks for both fixture children and their
ancestors. Its raw log now records pending, completed and failed-audit states.
The measured executable remains unchanged; the original runner and harness sources
are retained under `measured-source-v1/`. The post-timing tests described above
closed the held-call cancellation and native fixture-cleanup coverage gaps without
replacing that executable.

Confirmation records directory and representative first/last source-file flags
before warmup. This is not a measurement of allocated bytes for every file. The
runner uses deterministic rotation, not randomized trial order; the old screening
metadata's unused seed has no statistical meaning. A native call with an OS-level
hang has no harness subprocess deadline; no such result may be reported as a
completed successful trial.

## Local evidence

All generated evidence is beneath `_build/qa/ssd-copy-probe/`:

- `hardware-observed.json`: disk/partition topology and machine observations.
- `power-before-confirm.json`: read-only AC/battery observation.
- `source-manifest.json`, `snapshot.patch`: exact source hashes and permit change.
- `Cargo.toml`, `Cargo.lock`, `src/`, `snapshot/`: standalone benchmark sources.
- `native-final-tests.log`, `build.log`: optimized tests and executable build.
- `post-timing-guard-red.log`, `post-timing-tests-green.log`,
  `post-timing-clippy.log`: causal fixture-guard regression, five native test groups
  and strict release Clippy after measurements stopped.
- `run.ps1`, `analyze.ps1`: rotating trial runner and decision evaluator.
- `results-control-full/`: raw JSONL/CSV controls, metadata and decision.
- `results-screen-full/`: raw screening trials and workload manifests.
- `results-confirm-full/`: incomplete confirmation trials, representative file
  attributes, versioned runner hash and literal no-go decision.
- `confirmation-stop.json`, `untimed-audit-anomaly.json`: exact interrupted
  read-only command, elapsed time and contemporaneous system observation.
- `defender-observation.json`: read-only Defender status/log findings and the
  explicit limit on attributing causation.
- `independent-review.ps1`: separate raw-record, audit and paired-statistics check.
- `independent-summary.csv`, `independent-audit.json`: independently reconciled
  completed measurements and the incomplete-confirmation decision.

The benchmark files are ignored build artifacts. This report preserves the
method, final decision and summarized measurements in versioned documentation.
