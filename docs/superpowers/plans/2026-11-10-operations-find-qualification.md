# Operation qualification and advanced Find implementation plan

> **For agentic workers:** use the existing parallel task ownership, test-driven
> implementation and final independent review; root schedules Cargo serially.

**Goal:** exercise real copy/move/delete paths and make Find correct, responsive
and convenient with recent-modification and verified duplicate searches.

**Architecture:** reuse production native services, Engine queues and Slint
presenters; extend the bounded Find worker. Independent operation QA, backend
Find and UI tasks share explicitly named request/result contracts.

**Tech stack:** existing Rust/Tokio/Slint, SHA-256 and Unicode libraries.

**Spec:** [qualification design](../specs/2026-11-10-operations-find-qualification.md).

## Global constraints

- Fixtures/output under `_build`, caches `.tools`, no unrelated worktree changes.
- Only owned fixtures may be deleted; no real disk exhaustion or global recycle
  cleanup. Describe native and injected failures separately.
- Backend-only filesystem/content work; separate platform files; no new fees or
  dependencies; handwritten files at most 100,000 bytes, rustfmt width 100.
- Preserve native path identities, checksums, cancellation and source safety.

## Review focus

1. Cancellation after publication versus before verification: no source loss or
   surviving partial temporary file; test both deterministic phases.
2. Hard-link aliases, overlapping roots and source mutation cannot create false
   duplicate groups; test real identities and controlled replacement.
3. Date cutoffs do not move during queue waits or exclude traversal of old parents.
4. Unicode text and whole-word boundaries split at every possible read edge are
   decoded/matched without whole-file allocations or silent false results.
5. Sorting/progress/resize cannot change the targeted result or steal input focus;
   test real pointer/key events and stable IDs under queued updates.

### Task 1: Real native operation matrix (parallel agent)

Files: new `crates/midnight-engine/tests/operations_real_windows.rs` and isolated
support modules; own only backend-transfer fixes agreed with root.

- [ ] Build and run owned fixtures through production Engine and native services.
- [ ] Cover checksum modes, fast rename, overwrite actions, queue/cancel/retry,
  source changes, same-object safety, locked/read-only files and deletion.
- [ ] Run opt-in recycle fixture separately; verify all other sentinels untouched.
- [ ] Record counts, durations, stage cleanup and exact unsupported/injected cases.

### Task 2: Recent/duplicate Find backend (parallel agent)

Files: `midnight-core/src/find.rs`, `midnight-engine/src/find*.rs` excluding
`find_content*`, new duplicate worker/tests and relevant fixture adaptations.

Interfaces: `FindRequest.modified_within: Option<Duration>`, `duplicates: bool`;
`FindResult.duplicate_group: Option<u64>`, `duplicate_group_size: usize`;
`FindProgress.phase` plus duplicate candidates/groups/bytes; independent bounded
candidate count. `FindContentQuery.whole_words: bool` defaults false.

- [ ] Add contract and failing date/duplicate tests, observe failures.
- [ ] Freeze acceptance time, apply predicate without pruning traversal.
- [ ] Add size/hash/exact comparison pipeline and complete-group publication.
- [ ] Test cancellation, budgets, hard links, mutations, empty files and sorting.

### Task 3: Content matching and application pathways (root)

Files: `midnight-engine/src/find_content.rs`, extracted decoder/matcher modules,
`tests/find_content_streaming.rs`; new UI/native operation integration tests.

- [ ] Reproduce missing UTF-16/whole-word behavior and benchmark current matcher.
- [ ] Add bounded decoding/matching with query/buffer reuse; preserve revisions.
- [ ] Test actual UI-to-engine copy/move/delete, clipboard and drop admission;
  run actual native desktop workflow with owned fixtures where feasible.
- [ ] Record performance and actual versus simulated input/native services.

### Task 4: Find UI sweep (parallel agent)

Files: `midnight-ui/ui/find-window.slint`, `find-results.slint`, Find-only controls,
`src/find_windows.rs`, `presenter/find.rs` and Find UI tests.

- [ ] Observe RED for missing page navigation and query functionality.
- [ ] Add compact advanced controls, visible active state and duplicate group UI.
- [ ] Preserve native host/focus/drafts and 320×200 footer geometry.
- [ ] Test keyboard/pointer, sorting/selection, Stop/restart, multiple windows and
  lazy large models; render and inspect normal/small/light/dark forms.

### Task 5: Integration and qualification (root plus independent review)

- [ ] Run scoped backend/platform/UI suites and native owned-fixture probes.
- [ ] Have an independent reviewer examine changes and identified risk classes.
- [ ] Fix demonstrated bugs, rerun affected checks, strict Clippy/format/source size.
- [ ] Build debug app, deploy matching runtime and smoke-test actual Windows UI.
- [ ] Update design/developer/status and verification docs with measured results.

No unrelated changes are staged or committed as part of this plan. Any remaining
native migration and broader application feature goals remain explicitly open.
