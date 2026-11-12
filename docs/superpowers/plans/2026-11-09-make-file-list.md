# Make File List Implementation Plan

> **For agentic workers:** Use the assigned independent modules in this shared
> workspace; root owns integration and Cargo runs. Preserve unrelated changes.

**Goal:** Implement Salamander-style Ctrl+M Make File List end to end.

**Architecture:** Immutable captures feed a cancellable bounded backend formatter
and private spool. A native Slint tool window edits drafts and delegates all I/O.

**Tech stack:** Existing Rust/Slint/Tokio, transfer and private spool services.

**Spec:** `docs/superpowers/specs/2026-11-09-make-file-list-design.md`.

## Constraints and review focus

Keep Unicode/native/provider identities lossless, exclude `..`, use backend-only
I/O, separate platforms, no new paid/dependency obligations, 100-column Rust style,
100,000-byte source limit, output under `_build/`. Check large selections without
UI copying, duplicate names, changed targets while prompting, close/cancel races,
and source/destination aliasing before enabling export.

## Tasks

- [ ] Formatter agent: `midnight-core/src/file_list_format.rs` and tests, exact
  reference grammar and Unicode-safe explicit widths, injected environment data.
- [ ] UI agent: `midnight-ui/ui/file-list-window.slint` and integration tests;
  separate native responsive window, template controls, output choices and preview.
- [ ] Output agent: `midnight-engine/src/file_list_output.rs` and tests;
  verified create/append/replace, opaque destination pin retained through consent.
- [ ] Root: core snapshot/request/event/settings contracts, engine selection
  capture/record conversion/spooling/cancellation, clipboard and generated viewer
  adapters, native save picker, presenter/window host and Ctrl+M/F10/toolbar wiring.
- [ ] Root: persistence tests, backend/UI tests, docs and hotkeys, review fixes,
  formatting/Clippy/debug build and native smoke. Record exact evidence.

Tests are written before implementation, with causal failure evidence; agents
coordinate compilation through root to avoid concurrent workspace builds. Existing
dirty work is retained in place because the active application baseline includes
uncommitted integrated features; no automatic whole-workspace commit is made.
