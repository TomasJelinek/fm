# Keyboard options implementation plan

> Use superpowers:executing-plans to implement this scoped UI and presentation change.

**Goal:** A readable settings window with a Visual Studio inspired shortcut editor.

**Architecture:** Keep the existing validated shortcut map and backend persistence.
Add a small cached keyboard editor model, early key capture routing, and reusable
Slint list/capture controls. Native identities and platform key handling stay intact.

**Spec:** [Keyboard options design](../specs/2026-11-07-keyboard-options-design.md).

**Constraints:** Rustfmt 100 columns; source files at most 100,000 bytes; no new
dependencies; all I/O remains backend; errors use the shared error notice.

## Tasks

- [x] Presenter: add command filtering, captured-chord inspection, alias add/remove
  and conflict preview in `presenter/keyboard_editor.rs`, with presenter unit tests
  for exact aliases, duplicate canonical keys, conflicts, and empty search results.
- [x] Routing: add capture hooks before window/physical shortcut dispatch in
  `shortcut_bridge.rs`; preserve AltGr/IME and Tab/Escape focus behavior. Add actual
  native-event regressions to `tests/keyboard_options.rs` before implementation.
- [x] UI: replace `keyboard-options.slint` with a virtual command list, current aliases,
  capture field, context/conflict display and explicit buttons. Export a compact global
  view model to avoid adding many properties to `main.slint`. Polish the settings
  shell in `options-dialog.slint`, including bounded larger geometry and fixed footer.
- [x] Verify: Windows/Linux UI tests, desktop/touch renders, strict Clippy/rustfmt,
  source-size guard. Update shortcut documentation and the main design with actual
  implemented behavior, then review, commit and push with the other verified changes.

## Review focus

Captured Delete/F8/F4 must not invoke file operations. Physical digits must not become
layout punctuation. Typing in the search field keeps native shortcuts. Filtering must
never retarget an assignment by changing row numbers. Failed persistence retains the
validated session map and surfaces the error without losing the editor selection.
