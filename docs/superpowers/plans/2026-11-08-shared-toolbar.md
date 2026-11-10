# Shared toolbar implementation plan

> **For agentic workers:** Use superpowers:executing-plans task-by-task. No commits
> are authorized for this task; preserve the shared worktree and index.

**Goal:** Replace both duplicated panel toolbars with one active-panel bar.

**Architecture:** SharedToolbar consumes active panel presentation and emits
existing callbacks. FilePane retains drives, tabs, paths and hidden per-pane
BookmarkMenu dialogs. Popup/pressed-button context includes panel identity.

**Tech Stack:** Existing Slint components, Rust native-event tests, original SVG.

**Spec:** ../specs/2026-11-08-shared-toolbar-design.md

## Global constraints

- Keep the 320×200 minimum; UI performs no filesystem or heavy backend work.
- Preserve native/archive/parent-only action gates and configured shortcut hints.
- Do not mutate shared UI while another agent compiles it; coordinate Cargo.
- No new dependencies, commits or changes to user preferences.

## Review focus

- Active panel changes while a popup is open or a pointer is held: cancel stale
  interaction; equal numeric revisions on the two panels are not equivalence.
- Small windows/touch/32px fonts: scroll to controls without overlap or an enlarged
  minimum; keep paths and file rows usable.
- Keyboard Hot Paths: resolve against the opened choice model and original panel.
- Parent-only and archive contexts: existing operation enablement remains intact.
- Existing pane drive/tab/path geometry and focus behavior survive toolbar removal.

## Task 1: Shared bar and interaction boundaries

Files: new ui/shared-toolbar.slint; ui/main.slint; ui/file-pane.slint;
ui/action-icons.slint; new ui/icons/checksums.svg; docs/ASSET_PROVENANCE.md.
Tests: new tests/shared_toolbar.rs, adjusted tests/comparison_toolbar.rs and
tests/small_window.rs. Existing tests/compact_controls.rs remains a regression.

- [x] Add actual-event tests for singular checksum/comparison controls, active
  panel dispatch, and sort popup invalidation with equal panel revisions. Run RED.
- [x] Extract the action bar into SharedToolbar; preserve existing pane callbacks
  and route captured Hot Path choices through the existing MainWindow methods.
- [x] Add explicit horizontal overflow controls with min-width zero and one row.
- [x] Add checksum SVG and provenance; use it in the toolbar and Checksums menu.
- [x] Extend tests for narrow/touch/32px layouts, reachable overflow, stale pointer
  presses, native/archive/parent gates and unchanged drive/Hot Path key handling.
- [x] Run focused UI tests including comparison_toolbar, shared_toolbar,
  small_window, compact_controls and archive_options with one test thread; retain
  rendered snapshots under _build. Run strict UI checks and review the diff.

## Task 2: Integrated qualification and handoff

- [x] Provide source freeze and exact target list to root for the final debug app
  build/native snapshot; do not run an uncoordinated full app build.
- [x] Record terminal logs, observed screenshots and any concrete remaining gaps
  in a dedicated verification document. No unqualified completion claims.

Qualification: 506 Windows library and 44 toolbar/navigation integration tests
passed, as did all-target strict Clippy. The coordinated archive gate exposed a
separate last-checkbox wheel/resize failure (2 passed, 1 failed); its owner is
investigating and root will append that resolution, Linux results and the final
application build/smoke to the shared verification report. See
`docs/verification/2026-11-08-shared-toolbar.md` for the full evidence ledger.
