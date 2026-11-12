# Metadata job history implementation plan

> **For agentic workers:** Use superpowers:executing-plans for this focused regression.

**Goal:** Retain metadata details for every session job until explicit history clearing.

**Architecture:** Store bounded per-operation metadata results by immutable OperationId.
Jobs lookups and replacement use the same keyed collection; existing explicit clearing
and late-result fences remain authoritative. No persistence or new dependency.

**Tech Stack:** Rust, existing presenter and Slint job-row model.

**Spec:** docs/developer/OPERATION_PROGRESS.md, session history requirement.

## Constraints and review focus

- Keep backend native identities out of UI; only existing bounded result snapshots.
- Preserve the 32-outcome per-job sample limit; remove only the 16-job eviction.
- Cover result replacement, more than 16 jobs, live jobs, selective clearing,
  late cleared results, and rendered diagnostics/cache invalidation.

## Implementation

- [x] Add a presenter regression producing 20 metadata operations through events;
  verify first/last details, replacement without duplication and both clear modes.
- [x] Run the focused regression and retain its expected failure at the 16-job cap.
- [x] Replace Vec result storage with HashMap<OperationId, Arc<MetadataResult>> in
  presenter/operations.rs; adapt job_history.rs, job_presentation.rs and cache tests.
  Use insert/get/extract_if and deferred reclamation; ordering comes from progress
  history, never the result map.
- [x] Run targeted history and cache tests, then formatting and affected Clippy.
- [x] Document keyed session retention and record verification; integrate with the
  active tree-delete batch before the final affected-suite pass and commit.

## Targeted evidence

`_build/qa/job-history-20261112/red-direct.log` fails before the fix because the
first metadata receipt disappears after more than 16 operations. The initial
Cargo invocation selected zero tests and is not evidence. `final-green.log`
contains both successful retention and deferred-destruction regressions (0.01 s).
`cache-green.log` covers all four derived-row cache checks. The existing ten
operation presenter regressions also passed. Final integrated lint, build and full affected suites passed; see
[the integration record](../../verification/2026-11-12-tree-delete-integration.md).
