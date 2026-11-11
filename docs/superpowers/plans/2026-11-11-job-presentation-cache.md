# Job Presentation Cache Implementation Plan

> **For agentic workers:** Use the approved inline execution flow; no commits in this task.

**Goal:** Remove repeated full-history detail formatting without discarding job receipts.

**Architecture:** Cache derived OperationRow values per Presenter and typed job identity.
Reuse only an exact progress Arc, NumberLocale, and optional CopyFailure/MetadataResult
Arc dependency. Retain Weak identity tokens so cached UI data does not own retired
core payloads. Always refresh asynchronous icons and attention state. Prune derived
cache entries when their retained job receipts disappear.

**Tech Stack:** Existing Rust/Slint presentation types; no new dependencies or Slint changes.

**Spec:** docs/verification/2026-11-11-job-history-audit.md and the approved measured-cache task.

## Constraints and review focus

Preserve all receipts and the current metadata result retention policy. Do not cache
filesystem or mutation authority. Verify locale changes, optimistic Pause Arc mutation,
late failures/metadata results, asynchronously loaded icons and explicit Clear. Theme
remains a Slint binding; image handles are refreshed from the current desktop cache.

## Implementation

- [x] Add `job_row_cache.rs` with typed snapshots, exact Arc dependency comparison,
  locale comparison and retained-ID pruning; register one Presenter field/init.
- [x] Add focused reuse/invalidation/clear tests, then wire cached Copy/mutation row
  construction in `job_presentation.rs` with shared live icon/attention refresh logic.
- [x] Run the full ordinary UI unit suite and the unchanged ignored 1k/10k
  benchmark under coordinated qualification slots. Record exact artifact hashes,
  timings and remaining limitations.
- [x] Parent qualification: affected rendered job integration tests and final app build.

Baseline: 10k warm rows take 41.666–41.757 ms for ordinary details and
241.949–259.228 ms with 16 retained errors and 240 cached samples per job. Baseline
unit suite passes 885 tests; the benchmark is opt-in. Qualification compilers and
native timing workloads are serialized; no Cargo until the parent grants a lease.

The unchanged quiet-slot benchmark now passes all eight cases. Cached 10k warm
rows take 13.524-13.708 ms for ordinary details and 15.355-15.899 ms for detailed
rows. See the audit for exact artifact hashes, scope and retained-memory costs.
After correcting the invalid whole-row Image equality oracle, the final rebuilt
UI unit artifact passed 889 tests with 0 failures and 4 ignored. All four cache
reuse/invalidation tests passed. The audit records the exact final artifact hash
and isolated-TEMP qualification log.
