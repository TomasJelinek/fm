# Bounded Cargo test batching implementation plan

> **For agentic workers:** Use superpowers:executing-plans to implement this approved plan.

**Goal:** Reduce repeated Cargo startup without reducing CI test coverage or losing bounded cleanup.

**Architecture:** Keep Cargo as the test executor. Combine consecutive integration selections with
repeated `--test` arguments, bounded by selected package/target executables. Keep same-name groups
atomic because Cargo selects those across the workspace. Retire only the complete, validated artifact
set after the whole batch succeeds. Library, doctest, binary and example gates remain unchanged.

**Tech Stack:** Python standard library, Cargo JSON messages, existing workspace metadata/features.

**Spec:** `docs/verification/2026-11-12-ci-windows-test-runtime.md` and parent-approved batching design.

## Constraints and review focus

- Keep the exact active feature union, eligibility, working directories, runners, runtime library
  paths, test environments, custom harnesses, ignored tests and Cargo fail-fast behavior.
- Keep all default library/doctest/example coverage; no workflow or timeout weakening.
- Preserve failed batch artifacts; stop immediately and never execute later batches.
- Validate exact `(package_id, target_name)` coverage and every generated path before any deletion.
- Same-name groups cannot split: the artifact bound is the larger of the requested batch size and
  largest existing name group. This preserves the serial runner's unavoidable duplicate-name group.
- Keep the default at one until actual same-target 1/4/8 measurements justify a bounded alternative.
- Output stays in `_build`; coordinate Cargo and quiet measurements. No dependencies or commits.

## Tasks

- [x] Add failing Python fixtures for batching, unchanged coverage, duplicate-name grouping, exact
  artifact identity, Cargo JSON framing, complete-set cleanup and failed-batch retention.
- [x] Implement the small planner and exact artifact validation; expose bounded sizes 1/4/8.
- [x] Run targeted Python fixtures, inspect the diff and request independent review.
- [x] After a Cargo lease, measure 1/4/8 on the same actual representative integration targets,
  including Cargo command count, build/run wall time, cleanup time and peak selected artifact bytes.
- [x] Choose the default only from measured evidence; document current graph counts and the limits
  of local warm-cache measurements versus hosted full-suite runtime.

Cargo remains the execution authority; repeated `--test` selections are documented in the
[Cargo test reference](https://doc.rust-lang.org/cargo/commands/cargo-test.html). Within a batch Cargo
chooses target order, so it can differ from the old global alphabetical target-name order. Execution
remains serial and failure stops the batch. This is not an assertion that the 50 m 32 s CI baseline
will improve by the ratio of command counts.

## Completion evidence

Default eight approved after paired 19-test/8-target samples measured 13.8% lower mean local elapsed
than serial, with selected artifacts bounded at 73,445,888 bytes. Final Python suite: 31 tests,
2 existing Windows symlink privilege skips. See
`docs/verification/2026-11-12-ci-test-batching.md` for raw paths, actual graph counts and hosted-run
limitations. No workflow edits, dependencies, broad Rust tests or commits were needed.
