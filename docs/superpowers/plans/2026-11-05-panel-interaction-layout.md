# Panel interaction and layout implementation plan

**Goal:** Implement the requested panel controls and demonstrate faster selection
feedback and resizing without moving non-UI work onto the UI thread.

**Architecture:** Bounded Tokio commands and immutable domain snapshots remain
authoritative. Shared Slint controls own layout/focus; a bounded presenter overlay
provides immediate selection feedback while backend commands complete.

**Spec:** [interaction/layout design](../specs/2026-11-05-panel-interaction-layout-design.md).

## Constraints and integration

- Rust Style Guide/API Guidelines, rustfmt width 100, handwritten source <=100000 B.
- Native implementations and tests stay in separate platform files.
- Preserve raw Unicode/native identities and no application MAX_PATH limit.
- All I/O, terminal/clipboard work and unbounded list processing stay on backend.
- Use current checkout and existing `_build` caches; previous HEAD 08548d5 is
  committed, pushed and fully tested. New QA artifacts go in `_build/qa/interaction`.
- Coordinate Cargo jobs centrally; never build mixed Slint/presenter snapshots.

## Task 1 — Selection semantics, immediate feedback and benchmark

Owned files: core/engine selection, presenter selection/input/state/tests,
`fm-ui/src/lib.rs` row-model and row-click wiring, panel details selection summary.

- [x] Capture selection and resize baseline measurements before product edits.
- [x] Add delayed-ack, queue refusal, range-anchor and selected-count regressions.
- [x] Implement bounded pending-selection presentation, backend range actions and
  separate file/folder counts; preserve navigation/revision guards.
- [x] Expose `Presenter::select_row_modified(pane,index,control,shift,send)` while
  retaining existing select_row callers. Wire row-click's extra Shift flag.
- [x] Optimize measured row/model update bottlenecks and retain benchmark evidence.
- [x] Run focused tests and review the selection diff independently.

## Task 2 — Shared panel layout and native details

Owned files: Slint UI components/theme, per-platform detail formatters, focused UI tests.

- [x] Add regression tests for Ctrl+Tab, right-click path copy, tab shrinking and
  overflow controls, splitter drag/reset, physical side commands after Ctrl+U.
- [x] Move action tools above drive row; fit closed drive picker to caption; remove
  Type column; implement uniform tabs and session splitter/visual panel swapping.
- [x] Decode Windows attributes without hex, retain chmod notation, neutralize Light.
- [x] Implement right-click breadcrumb identity callback and native clipboard route.
- [x] Add `. As Other Panel` to the drive menu with period/mouse activation and
  raw opposite-path navigation; verify Enter opens the focused file or folder.
- [x] Preserve useful command-editor width with large Linux font metrics while
  retaining the full working path and its horizontal panning.
- [x] Verify font/density boundaries and native visuals without changing preferences.

## Task 3 — Terminal split control and command routing

Owned files: shared terminal control/icons, presenter terminal, UI callback wiring/tests.

- [x] Test main icon launching an empty command with selected shell/current path,
  preserving the editor draft; side picker only selects, never launches.
- [x] Implement distinct shell icons and split-button keyboard/accessibility behavior.
- [x] Preserve fixed draft grammar, history and session-only elevation semantics.

## Task 4 — Integration and delivery

- [x] Run current-source Windows/Linux tests, strict Clippy, fmt/source-size, normal
  builds and native Windows UI checks; repeat only when fixes require it.
- [x] Compare before/after selection and resize measurements with identical settings.
- [x] Independent final review; address concrete findings.
- [x] Update design, hotkeys, README and verification report, including F3/Alt+F3 design.
- [x] Commit intended sources/docs only and push with TomasJelinek as authorized.
