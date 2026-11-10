# SSD Transfer Overlap Benchmark Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Measure real 1/2/4-stream native verified-copy performance on this PC and
reject production concurrency work unless the substantial-gain gate passes.

**Architecture:** A separate QA workspace snapshots the current common transfer
primitive and changes only the copied I/O permit cost. Real Windows source/stage
handles, existing verification and publication execute under bounded workers;
unmodified production code supplies a one-worker control.

**Tech Stack:** Current Rust toolchain, current native Windows platform crates,
existing `midnight-work-budget`, pinned `sha2 = 0.10.9`, PowerShell. No new
third-party dependency, production source change or shipped feature flag.

**Spec:** [SSD transfer overlap: benchmark gate](../specs/2026-11-10-ssd-transfer-scheduling.md).

## Execution record: 2026-11-10

The experiment ran on the single positively identified NVMe SSD in this PC.
C: and D: are partitions of that same physical disk. All fixtures used D:;
there is no evidence about two independent SSDs. The primary policy was full
SHA-256. Sample-mode correctness was tested, but sample-mode performance trials
were not needed or run.

- [x] Preserve the measured executable and source provenance. Its SHA-256 is
  `937BDB4A3C869F8AFBC1537612CF6F531F56AC2302578146FEB8D3A6C566B9CD`.
  `measured-source-v1/` archives the original Rust/runner files; all four
  production/snapshot hashes still match `source-manifest.json` after timing.
- [x] Run the six-trial control. Production/1 median was 4.8590304 s and
  snapshot/1 median 4.9198515 s, within the 5% control tolerance. All audits passed.
- [x] Run the 27-trial screening. Both 2 and 4 workers passed screening;
  `results-screen-full/decision.json` nominated them for confirmation only.
- [ ] Complete the 84-trial confirmation. **Stopped as inconclusive/no-go**:
  42 fully audited large/mixed trials completed, then small-file integrity audits
  became abnormally slow. The first small timed result, 21.604168 s, is retained
  with pending and failed-audit records and is not counted as qualified.
  The second slow read-only audit was stopped after 159.8402575 s, with zero
  writes, following repeated stalls. No small-file guard or larger-file
  confirmation was completed. No trials were silently removed or restarted.
- [x] Preserve the stop evidence and fixtures. `confirmation-stop.json` records
  the exact owned audit command/PID/duration; `untimed-audit-anomaly.json` records
  a read-only environment sample. The cause remains unproven. Both source and
  output files for the small workload remain under the owned fixture directory.
  `results-confirm-full/decision.json` is the evaluator's literal `no-go`.
- [x] Run post-timing native safety tests using the separate unit-test executable.
  `post-timing-guard-red.log` demonstrates unsafe source-junction disposal;
  `post-timing-tests-green.log` records five passing tests after the QA-only guard
  correction. The added held-readback cancellation test proves join-before-return,
  retained partial ownership until release, source preservation and preservation
  of a file completed by the other worker, under both verification modes.
- [x] Complete strict QA Clippy. `post-timing-clippy.log` records the successful
  release-profile, all-targets check for the isolated harness with warnings denied.
- [x] Complete independent final reconciliation. The read-only reviewer checked
  six control, 27 screening and 42 completed confirmation rows plus the retained
  failed small-file audit, unique trial keys, full verification lengths, medians
  and immutable executable hash. No remaining scoped blocker was found; this
  review confirms the incomplete/no-go result, not a production scheduler gate.
- [x] Leave production concurrency unchanged. No production scheduler, app
  setting, dependency, release binary or deployment was changed for this probe.

The independent reviewer and parent own final reconciliation and the verification
report. These observations do not establish foreground navigation latency or
that four concurrent workers are a safe application default. The machine stayed
on AC with a full battery, and sampled fixture paths were ordinary uncompressed,
non-sparse files/directories. Buffered cached I/O, background-system activity,
the small data footprint and the incomplete confirmation limit interpretation.
No further performance trials are authorized for this experiment today.

## Global Constraints

- The latest user instruction is to benchmark this PC first and **not implement
  SSD concurrency unless copying becomes substantially faster**.
- No production scheduler is authorized by this document.
- HDD, network, unknown, mixed or failed classification remains serial.
- All executable harness code, lockfiles, generated output, data and logs live
  below `_build/qa/ssd-copy-probe/`.
- The buffer is 256 KiB. Full SHA-256 is the primary gate; sample verification is
  first/middle/last 64 KiB and all bytes for files of at most 192 KiB.
- Do not purge global caches, disable security software, change system power
  settings or run privileged disk configuration commands.
- Preserve unrelated working-tree changes. No commits, production edits or Cargo
  until the parent releases the current application-build artifact freeze.
- The coordinating review approved the repeatable 20% full-mode gate and QA-only
  method; workload sizes are bounded experiment choices, not app requirements.

## Review Focus

1. Extra workers behind the old four-slot exclusive request must not be reported
   as overlap; the native A-readback/B-write test owns this boundary in Task 2.
2. Different drive letters on the same SSD must share one reported physical
   identity; classification uncertainty stops qualification in Task 1.
3. A modified primitive must not quietly change verification or cheapen its
   baseline; exact source diff and production/1 control are required in Tasks 1/3.
4. Disk-full, substituted cleanup directories and cancellation with a native call
   still running must preserve source/data and report residuals in Task 2.
5. Cache warmth/order/noisy best runs must not manufacture the go decision;
   interleaving, all raw trials, sustained confirmation and literal thresholds
   are owned by Task 3.

---

## Scope, ownership and files

This plan is an isolated experiment, not the earlier speculative production
rollout. Existing production behavior remains unchanged throughout. The owning
agent coordinates Cargo with the parent; a read-only reviewer checks the harness
diff and methodology before timing. No additional agent is required by this plan.

| File | Responsibility / owner |
| --- | --- |
| `_build/qa/ssd-copy-probe/prepare.ps1` | Benchmark owner: topology/free-space evidence, source snapshot, exact diff, isolated workspace. |
| `_build/qa/ssd-copy-probe/Cargo.toml`, `Cargo.lock` | Benchmark owner: existing pinned local dependencies and separate target directory. |
| `_build/qa/ssd-copy-probe/snapshot/src/` | Generated exact common module copies in QA dependency crate; only declared permit-cost delta. |
| `_build/qa/ssd-copy-probe/src/fixture.rs` | Benchmark owner: owned deterministic data, manifest, space bounds and safe cleanup. |
| `_build/qa/ssd-copy-probe/src/runner.rs` | Benchmark owner: production/probe adapters, fixed workers, timing and full native lifecycle. |
| `_build/qa/ssd-copy-probe/src/trace.rs` | Benchmark owner: bounded distinct-file native overlap observations. |
| `_build/qa/ssd-copy-probe/src/main.rs` | Benchmark owner: validated CLI and JSON trial output. |
| `_build/qa/ssd-copy-probe/run.ps1`, `analyze.ps1` | Benchmark owner: deterministic interleaved rotation, paired ratios and go/no-go. |
| `_build/qa/ssd-copy-probe/results-{control,screen,confirm}-full/` | Generated JSONL/CSV evidence, fixture manifests and decisions, never production inputs. |
| `docs/verification/2026-11-10-ssd-copy-probe.md` | Coordinating parent: exact results and limitations; independent reviewer verifies claims. |

Read-only source dependencies: `midnight-platform-common/src/transfer{,_proof,
_retry,_edges}.rs`, core transfer contracts, Windows platform/native stage code,
and `midnight-work-budget/src/lib.rs`. Production platform-specific implementations
remain separate; the QA executable is explicitly Windows-only for this PC.

## Task 1: Provenance, topology and a comparable isolated primitive

**Files:** Create `prepare.ps1`, `Cargo.toml`, `Cargo.lock`, generated
`snapshot/src/{transfer,transfer_proof,transfer_retry,transfer_edges}.rs` and
`src/provenance_tests.rs` below the QA root. Read the source dependencies above.

**Interfaces:**

- Implemented as `runner::Engine::{Production, Probe}` with validated CLI
  arguments passed to `runner::run`. The public timed CLI accepts streams 1/2/4;
  `Production` accepts only 1. Paths refer to the fixed owned QA root.
- Produces `source-manifest.json`: original/copied paths and hash pairs.
  `snapshot.patch` records the single permit change. `hardware-observed.json`,
  native `topology` output and `power-before-confirm.json` record the actual
  machine evidence. Cargo/profile and immutable binary identities are retained
  with the QA workspace and run metadata.
- Generated `qa_transfer::transfer::VerifiedFileCopy` retains the existing public signatures
  of the snapshotted primitive. No production public API is extended.

- [x] **Step 1: Review the proposed method and gate with the parent.** Confirm the
  current build has released Cargo and the experiment may execute. Record any
  revised threshold or fixture bound in the spec before collecting timings.
- [ ] **Step 2: Add failing provenance/configuration checks.**
  `snapshot_rejects_zero_or_multiple_permit_matches` must refuse an unexpected
  source body; `only_declared_permit_cost_changes` compares every copied module;
  `production_cannot_run_multiple_streams` rejects 2/4. Use immutable source-copy
  fixtures for these checks; never mutate the repository to make them fail.
- [ ] **Step 3: Run those checks against the intentionally absent transformer.**
  Save the meaningful RED output under the QA root.
- [x] **Step 4: Implement `prepare.ps1`.** Copy the four exact modules, replace only
  the one acquire cost, save the diff and verify byte-for-byte preservation of
  all other content. Query topology read-only; reuse the native positive SSD
  classifier as evidence and correlate source/destination physical disk mapping.
  Uncertain/unsupported classification is a stopped experiment, not an override.
- [x] **Step 5: Create the separate Cargo workspace with existing pinned inputs.**
  Use local core/common/Windows/work-budget path dependencies and existing SHA-256.
  Generate the QA lockfile offline, then build/test with `--locked --offline`.
  No root manifest or root lockfile changes are allowed.
- [x] **Step 6: Run snapshot checks and review the source diff independently.**
  Verify the only algorithm-source delta is the permit cost. Record exact source
  hashes before subsequent runs; any source change invalidates mixed-version data.

**Executed verification:** The transformer requires exactly one matching permit
request and checks the inverse replacement. The parent and reviewer independently
compared all four original/copy hashes and the one-line patch. The originally
proposed dedicated `provenance` Rust test target was not implemented; Steps 2/3
above remain unchecked. `source-manifest.json`, `snapshot.patch`, native
`topology` output and `hardware-observed.json` contain the actual evidence.

## Task 2: Real native execution, owned fixtures and overlap evidence

**Files:** Create `src/{fixture,runner,trace,main}.rs` under the QA root. Native
tests are in the binary's `runner::tests` module, not a separate integration target.

**Interfaces:**

- Validated workload names `large`, `mixed`, `small`, `sustained` use the spec
  file counts/sizes. `sustained` is only the historical CLI name for eight larger
  files totaling 2 GiB; it does not establish sustained NAND or beyond-RAM speed.
- `Fixture::create(workload: &str) -> io::Result<Fixture>` returns
  an immutable ordered file manifest and destination path. Free-space/containment
  checks precede writes. Data generation and source synchronization are untimed.
- `runner::run(&Fixture, Engine, usize, TransferVerification, u32) -> io::Result<Trial>`
  joins every worker, including failures/cancellation, before returning.
- `Fixture::audit(destination: bool) -> io::Result<()>` checks
  all full file contents/counts and source preservation outside the copy timer.
- `Fixture::clean_outputs() -> io::Result<()>` removes only
  recorded owned outputs after identity/containment checks; failure retains paths.
- `Trace::enter(file: usize, phase: Phase) -> Guard` observes the real inner
  native call; `Phase::{SourceRead, Write, Sync, Readback}`. Retain at most four
  active intervals plus aggregate counters; do not claim kernel queue depth.
- `Trial` retains elapsed time, bytes/files, verification bytes, observed maximum
  native method concurrency and distinct-file write/readback overlap. The runner
  adds process CPU seconds and audit/cleanup durations to JSONL/CSV. Peak working
  set could not be queried after process exit and is unavailable, not zero.
  Ordering is deterministic rotation; no randomized seed is used.

- [x] **Step 1: Add causal native contract tests.**
  `blocked_readback_allows_another_native_write_only_in_probe_mode` holds A in
  readback and requires B-write entry before release for probe/2; production's
  exclusive cost must prevent that entry. Ensure the guard releases on assertion
  failure so test teardown cannot leave a blocked thread.
- [ ] **Step 2: Add lifecycle/ownership tests.**
  `cancel_waits_for_native_return_then_removes_only_partial` covers write/readback,
  both policies, preserved completed outputs and unchanged sources;
  `cleanup_refuses_substituted_or_external_root` proves containment;
  `native_trials_publish_exact_audited_outputs` covers all permitted stream counts.
- [x] **Step 3: Run RED before implementing the runner.** Save the actual observed
  failure, distinguishing missing behavior from toolchain/configuration problems.
- [x] **Step 4: Implement `runner::run` using fixed joined workers and the real native
  transfer service.** Prepare the held destination once, then include per-file
  source open, stage creation, copy, flush/sync/readback, no-replace publication,
  explicit cleanup and final handle drops in timing. Retain the 256 KiB primitive
  buffer and selected policy. Stop new file claims on the first failure, join
  running calls, clean owned partials and report any residual before return.
- [x] **Step 5: Implement trace, fixture/audit and cleanup interfaces.** Native
  intervals start inside the acquired permit immediately before the wrapped call.
  Different files, not merely threads, must overlap. The manifest and full audit
  supply expected bytes independently of the transfer's own verifier.
- [x] **Step 6: Run native tests GREEN.** Verify no stage artifacts remain after
  successful/cancelled cleanup. A real cleanup failure is a failed experiment and
  retains the actionable path. Check that every absolute deletion target stays
  below the checked QA fixture root.

**Executed verification:** `cargo test --release --manifest-path
_build/qa/ssd-copy-probe/Cargo.toml --config $sdkConfig --target-dir
_build/qa/ssd-copy-probe/target --locked --offline -- --test-threads=1` ran the
binary unit tests. `native-red.log` records the absent runner behavior;
`native-final-tests.log` records three passing native tests before timing.
Those three pre-timing tests cover exact outputs, A-readback/B-write overlap,
and cancellation after actual write/sync/readback returns. The two post-timing
tests add external cancellation while readback is held and real source-junction
substitution. The five-test run is recorded separately in
`post-timing-tests-green.log`. No measurement executable was rebuilt for these
checks. A synthetic disk-full injection and the originally proposed broader
provenance test suite were not executed by this QA harness.

## Task 3: Controlled measurement and an explicit no-go/go report

**Files:** Create `run.ps1`, `analyze.ps1`, generated evidence files below the QA
root, and `docs/verification/2026-11-10-ssd-copy-probe.md`.

**Interfaces:**

- CLI: `ssd-copy-probe run production|probe 1|2|4 full|samples
  large|mixed|small|sustained <trial-u32>`. The owned QA root is fixed in the harness.
  `prepare`, `audit`, `clean` and `dispose` take a workload name; `topology` queries
  the native classifier. Fixture data stays below approximately 6 GiB live.
- `run.ps1` produces append-only `trials.jsonl`, completed `trials.csv`,
  `run.json` and independent fixture manifests. Confirmation records pending
  timings before audit and then a complete/failed record. It refuses concurrent
  builds/another probe and rechecks free
  space before every run; it never kills unrelated processes.
- `analyze.ps1` produces `summary.csv` and `decision.json`, including per-round
  `T1/Tn` ratios, medians, ranges, every failed condition and the literal decision.
  A trial correctness error invalidates qualification rather than being discarded.

- [ ] **Step 1: Test the gate evaluator with synthetic rows.**
  `below_twenty_percent_is_no_go`, `five_of_seven_pairs_required`,
  `quick_cannot_rescue_full`, `sustained_regression_is_no_go`,
  `small_file_slowdown_over_five_percent_is_no_go`, and
  `failed_audit_or_absent_overlap_is_no_go` assert the literal spec thresholds.
  Test exactly 1.20/1.10 boundary ratios and missing/incomplete trial groups.
- [ ] **Step 2: Run evaluator RED, implement paired-group analysis, then GREEN.**
  Group by immutable source manifest, workload, policy and worker count. Never
  combine runs from different binaries/source snapshots or silently omit failures.
- [x] **Step 3: Build one optimized isolated binary after artifact freeze release.**
  Use `cargo build --release` with the same QA manifest/config/target/locked/offline
  arguments. Record compiler/profile and binary hash. Stop all builds before timing.
- [x] **Step 4: Validate production/1 versus probe/1.** Full audits must agree and
  median elapsed difference must be at most 5%. Investigate a failure and stop;
  do not compensate with a scaling factor.
- [x] **Step 5: Run warmups and three deterministically rotated screening rounds for
  1/2/4 streams across the three screening workloads.** Full mode runs first.
  Record all data, native overlap and cache/environment limitations. Stop with
  no-go if no candidate approaches the substantial-gain gate.
- [ ] **Step 6: If screening warrants it, run seven interleaved measured rounds
  and sustained confirmation.** Apply the same candidate count to large/mixed;
  sample mode is secondary. Do not increase fixture size beyond the explicit
  free-space bound or change system settings to obtain favorable timing.
- [ ] **Step 7: Write the verification report and obtain read-only review.** Report
  production control, all stream counts/modes, topology, medians/paired ratios,
  integrity, overlap and cleanup evidence. Distinguish component throughput from
  application scheduling and cached readback from physical-media reads.
- [x] **Step 8: Apply the result exactly.** On no-go/ambiguous evidence, leave
  production serial and close only this experiment. On go, return the measured
  evidence for a separate complete scheduler plan; do not silently start production
  work, change defaults, mark IO-01 implemented, commit or deploy from this harness.

## Deferred production design checklist

This is not an implementation task in the current plan. If the measured gate
passes, the next plan must assign core/work-budget, Windows, Linux, plugin API/SDK
and native host, consumer migration, engine, UI and benchmark ownership explicitly.
It must define retained physical tokens and preserve SSD folder-scan/thumbnail
parallelism; one recursive root must demonstrate real A-readback/B-write overlap.
Independent files continue while another waits for overwrite/error consent, with
bounded state and a serialized dependency-aware commit lane. Per-file Move proof,
source removal, postorder directory cleanup, exact root receipts and all current
cancellation guarantees remain mandatory.

Foreground tests must wait for actual fresh directory results during long copy,
not only responsive chrome. Preserve the current distinction between explicit
foreground branch work and mutation-gated watch/RefreshLoaded maintenance until
a reviewed namespace-observation design safely changes it. Neither throughput
measurement nor a GUI render test proves this boundary.

## Self-review and handoff

The plan covers the latest conditional user request, rather than the superseded
direct implementation rollout. The evaluator's four executed boundary checks
cover below-20%, exact-20%, fewer-than-five qualifying pairs and incomplete
groups; they are not the full synthetic matrix originally listed in Task 3.
Native overlap has a barrier test; fixture deletion and partial cleanup have
ownership tests.
No production interface, plugin ABI, dependency inventory or app setting changes
are required for this experiment. Source and runtime limits are bounded; all
the recorded execution status above distinguishes completed and incomplete gates.

Review the method/thresholds before execution. The parent retains the current
build/deploy lease; no Cargo or fixture generation is authorized while that lease
is active. The final handoff is an evidence-backed no-go/go decision, not a claim
that the broad implementation goal is complete.
