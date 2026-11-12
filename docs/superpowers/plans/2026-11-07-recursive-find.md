# Recursive Find Backend Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans inline. No subagents or commits are authorized for this delegated task.

**Goal:** Implement the bounded native FIND-01 backend without claiming the modeless UI complete.

**Architecture:** A portable worker consumes identity-pinned native sources. Fixed controller slots serialize each session's latest generation; bounded native workers and streamed output preserve input and memory limits.

**Tech Stack:** Rust 1.99.1, existing Tokio/mask compiler/Unicode folding, existing Windows/rustix APIs. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-11-07-recursive-find-design.md`

## Global constraints

- Native paths remain raw; no UI filesystem work or shared OS-specific bodies.
- Outputs `_build/`; caches `.tools/`; files at most 100,000 UTF-8 bytes.
- Native only; no symlink/mount recursion. Unsupported capabilities stay explicit.
- Never touch concurrent copy/transfer implementations, Slint or keyboard code.
- Coordinate core exports/protocol and engine hooks with root before integration.

## Review focus

- A changed result must fail validation rather than activate a replacement.
- A full event queue must not strand cancellation or publish superseded results.
- UTF-8 and matches spanning reads retain exact text; malformed content is incomplete.
- Deep/wide trees and enormous native names consume explicit resource budgets.
- Root aliases, raw names and links do not cause loops, loss or implicit traversal.

### Task 1: Core contracts and native sources

Files: new `midnight-core/src/find.rs`, core exports/protocol Find-only additions;
new platform `{windows,linux}/src/find.rs` and native integration tests; module hooks.

- [x] Write validation/native tests for multiple roots, limits, links and raw names.
- [x] Observe missing API/behavior fail before implementation.
- [x] Implement request/output/source contracts and separate native adapters.
- [ ] Verify Windows and Linux native tests and strict Clippy.

### Task 2: Bounded traversal and content worker

Files: new engine `find_worker.rs`, `find_content.rs`, focused tests.
Consumes `FindRequest` and `FindService`; produces shared `FindResult` batches and
truthful `FindSummary` without UI access or automatic actions.

- [x] Write failing tree/content tests, including BOM, cross-chunk UTF-8, invalid
  encoding, access failures, matching case and lowered budgets.
- [x] Implement bounded depth-first traversal and strict streaming UTF-8 content.
- [x] Validate each captured result after matching; report incomplete work.
- [ ] Verify tests and source limits.

### Task 3: Controller and engine wiring

Files: new engine `find.rs`, `find_tests.rs`; Find-only engine `lib.rs` hooks.

- [x] Write cancellation/backpressure, latest-generation and stale-result tests.
- [x] Implement four fixed slots, two scan permits and bounded target validation.
- [x] Add optional Find service, commands/events and shutdown cancellation.
- [ ] Run actual Windows/Linux backend fixtures, formatting and strict Clippy.
- [ ] Record evidence and remaining UI/provider limits in `_build/goal/find/report.md`.

## Execution rulings and evidence

- Existing authorization and root's explicit API approval replace additional
  design/commit/delegation gates. All work remains in the shared checkout.
- Initial missing-core API and missing-worker-file tests failed as intended.
- First Windows core/native/worker/actor pass: 13 tests; strict library Clippy green.
- Initial Windows revision omitted native ChangeTime; same-size edits with
  restored LastWriteTime reproduced a false validation. Adding FileBasicInfo's
  ChangeTime made the exact regression pass.
- Initial native Linux core/source pass: 3 tests. Approved checksum dependency
  cache additions were fetched with `--locked`; Find adds no dependencies.
- Root requested practical large-file content search. The 9 MiB regression failed
  under the original 8 MiB cap. Incremental UTF-8/KMP replaces whole-body retention;
  defaults become 1 GiB per file / 16 GiB aggregate with truthful lowerable budgets.
- Windows idle log sharing regression failed with ERROR_SHARING_VIOLATION. Find's
  read-only reader now permits normal sharing and checks identity/ChangeTime again;
  the regression passes without altering copy sharing semantics.
- Backend locale regression observed None versus the existing native localized
  timestamp. Constructor-only per-OS formatting fixes it; raw snapshots/chunks stay
  formatting-free. Windows native5 and Linux full18 + strict Clippy pass.
- Native modeless UI, Alt+F7, lazy rows and panel reveal are integrated separately:
  `docs/developer/FIND.md` and `2026-11-07-find-window.md`. Root fixed the review's
  stale row-index callback and host-close recreation issues with regressions.
- Root delegated tooltip host registration polish to `tooltip.rs` and focused tests;
  placement must be shared while Main-only footer/pointer/drag observers stay Main-only.
