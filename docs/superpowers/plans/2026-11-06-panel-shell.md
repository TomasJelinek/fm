# Panel interaction and shell integration implementation plan

> **For agentic workers:** use superpowers:executing-plans for integration and
> superpowers:dispatching-parallel-agents for independent investigations.

**Goal:** Reliable mouse/keyboard file interaction, native menus, verified clipboard
transfers and compact, legible panel controls/dialogs.

**Architecture:** Slint emits intent and displays cached models. Core owns portable
contracts; engine captures identities and schedules work. Native shell/clipboard
and transfer primitives are implemented per platform. Reuse selection, operations,
bookmarks, sorting, locale and volume state rather than parallel implementations.

**Tech stack:** Existing Rust/Tokio/Slint 1.18.1 and native Windows/Linux services.
**Spec:** [request extension](../specs/2026-11-06-panel-shell-design.md).

## Constraints and review focus

- All non-UI work and potentially large aggregation/destruction stays on backend.
- Lossless native paths, no MAX_PATH buffers, Rustfmt 100 columns, files <=100KB.
- All build and QA output under `_build`; one Cargo process per target platform.
- Right-click identity must survive delayed selection and navigation changes.
- Clipboard data is untrusted: bounded parsing, no shell interpretation or recursive links.
- Copy verification is independent target readback; cancel/error preserves original data.
- Stale overwrite decisions and directory replacement cannot authorize a new target.
- Large selection, narrow windows, long Unicode names and text-editor focus must work.

## Tasks

- [x] Additional user requirement: implement all configurable application shortcuts
  according to [keyboard design](../specs/2026-11-06-configurable-shortcuts-design.md),
  including Options, persistence, contextual conflict checks and keyboard tests.

- [x] Reproduce/fix double-click through actual pointer events, refreshed delegates,
  all view modes, modifiers and stale identities. Tests in `midnight-ui/tests/mouse.rs`.
- [x] Test/fix Ctrl+Enter focus and add panel-only clipboard shortcuts in
  `file_keys.rs`, `file_key_bridge.rs`, `lib.rs`, `main.slint` and integration tests.
- [x] Implement native menu contracts/controller and per-OS adapters; inspect
  Salamander `IContextMenu`/clipboard paths; test routing, cancellation, multiselect,
  drive targets, Unicode and actual Windows menus.
- [x] Add file clipboard contracts and native payloads; connect verified transfer
  requests to the bounded operation queue, with progress/conflict/cancel controls.
  Test corruption, source change, collisions, stale replies, errors and retained sources.
- [x] Add backend selection-byte summaries and cached free-space presentation;
  test unknown metadata, mixed folders, large totals and stale volume responses.
- [x] Add sort chooser, grouped drive choices and split Hot Paths actions using
  existing components; verify keyboard separator skipping and pointer identity.
- [x] Improve operation dialog icons/warnings/layout and render-check existing
  Options, bookmark, operation and conflict windows in all themes/densities.
- [x] Run relevant Windows/Linux tests, strict Clippy, optimized builds, native
  interaction smoke and source-size guard. Document limitations honestly.
- [ ] Independently review the whole change set, fix material findings with tests,
  update architecture/hotkeys/build evidence, commit and push authorized work.
