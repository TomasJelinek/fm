# Commander Tools Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans and the explicitly
> delegated backend/UI owners below. Coordinate shared interfaces before edits.

**Goal:** Implement the requested comparison, panel filtering/zoom, Ctrl toolbar,
history rotation, and merged attributes/time/case editor without blocking UI.

**Architecture:** Typed core requests; bounded backend computation and mutation
controllers; independent native metadata implementations; reusable Slint dialogs.
Keep effective shortcuts as the single source for keys, captions and dispatch.

**Tech stack:** Existing Rust/Tokio/Slint/native handles; bounded regex matching
and a shared Unicode case mapper outside the dependency-free domain core.

**Spec:** [Commander tools](../specs/2026-11-06-commander-tools-design.md).

## Constraints and ownership

- Preserve all previous source changes and theme/command-row work.
- No overwrite or lossy native path conversion; no UI filesystem operations.
- One Cargo process per platform; coordinate root before builds or shared edits.
- Rustfmt100, files <=100,000 bytes; generated output and QA under `_build`.
- Root owns shared protocol/export/engine integration unless ownership is handed
  off explicitly. Backend owners first report their concrete interface contracts.
- Compare/filter owner: core comparison/filter contracts, engine projection and
  authoritative selection seeds, settings migration, comparison/filter presenter
  modules and extracted dialogs. Send main-window hooks to the UI owner.
- Metadata owner: typed edit contracts, shared Unicode transformations, native
  Windows/Linux metadata and combined rename operations, queue integration/tests.
- UI owner: command history, configurable Ctrl shortcuts, modifier footer, zoom,
  main-window wiring and the metadata editor once its typed contract is fixed.

## Work

- [x] Confirm source semantics and contracts against Salamander and current code.
- [x] Add failing comparison/filter/seed tests, implement the read-only workers,
  projection, settings migration and dialogs, then run focused backend/UI checks.
- [x] Add failing metadata/case tests, implement typed preparation/preview/apply
  and platform operations with cancellation and explicit partial results.
- [x] Add failing real-key tests for history and Ctrl toolbar/zoom, implement
  effective-binding dispatch and mouse/focus state handling, then rerun them.
- [x] Integrate the merged metadata dialog with explicit Keep states, preview
  revisions, capabilities and existing background jobs. Test stale/conflict paths.
- [ ] Review every new shared/native integration, run relevant Windows/Linux tests
  and release integrity checks, render dialogs/themes, update all design/hotkeys
  documents, build the final executable, commit and push the authorized work.
