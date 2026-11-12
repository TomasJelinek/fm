# Keyboard options execution ledger

Base: `5466f8068f9ce04feaddd0e0dc9d745cfb087f57`, shared working tree contains
several already-tested UI/backend refinements. Keep their changes; no isolated
checkout is needed for this narrowly coordinated root-owned task.

Ruling: scope is read-only and derived from action plus chord — preserves text
editing/IME protection and avoids a new shortcut storage schema not requested here.

Ruling: keep one default map plus user overrides — the supplied Visual C++ preset
is a UI reference, not a request to claim compatibility with another application's map.

Ruling: capture Tab/Escape then leave capture — all current configurable keys remain
assignable while keyboard users can exit the field. Neither invokes its app command.

Implementation started. Root owns `keyboard-options.slint`, `options-dialog.slint`,
shortcut bridge/presenter and settings tests. Attributes agent owns directory sizes.
Media/archive agents work in separate managed worktrees.

Prior error dialog regression: Copy details closed the popup because Slint defaults
to closing popups on any click. `message` now explicitly uses `no-auto-close`;
the actual click regression still needs its final rerun after the next bindings build.

Windows RED observed for `shortcut_editor_filters_commands_and_captures_without_dispatching_actions`:
the old UI has no accessible `Search commands` field. Draft implementation files
`presenter/keyboard_editor.rs` and `ui/keyboard-editor.slint` exist but are intentionally
not hooked into the application while the user-requested clean release build runs.
They still require wiring and compilation; no passing claim is made for these drafts.

The later Linux error-notice run passed all three actual UI checks, including Copy
details keeping the popup open. `cargo fmt` then exposed a trailing-comma macro pattern
bug in the new zero-argument callback; the macro now accepts zero or more comma-prefixed
arguments and the clean release build was restarted. This does not change the feature.

Next wiring: declare `mod keyboard_editor` in presenter, add its `ShortcutEditor` to
`KeyboardState`, expose `reset_editor` to sibling presenter modules, reuse
`replace_aliases` inside `change_shortcuts`. Export `KeyboardEditorView` and
`ShortcutCommandRow` through main.slint. Add global callbacks/refresh in a focused
Rust UI adapter, and intercept logical/physical capture before shortcut dispatch.
Replace the old keyboard controls in Options; migrate existing UI interaction tests
from multiline replacement to search/capture/add/remove workflow. Keep the existing
programmatic replacement API for callers that deliberately replace every alias.

## Implemented and verified checkpoint — 2026-11-07

The draft is now wired into the application. The old keyboard-options component
was removed; command filtering, identity-stable selection, alias add/remove,
conflict preview, key capture, individual restore and confirmed full reset are live.
The existing replacement API remains for intentional programmatic replacements.

Windows actual keyboard interaction tests: 14 passed. This includes asynchronous
preference save, physical digits/AltGr, captured Tab/Escape, updated captions/help,
and the category focus regression found during review. Jobs integration: 1 passed.
The all-theme dialog-layout matrix passed eight themes, desktop/touch and large
fonts. Dark/light normal/large settings and a light touch Jobs render were inspected.
Strict UI and Windows-platform Clippy, all targets with warnings denied, passed.

Linux final UI checks, the new Previews settings category, integrated plugin builds
and final source review remain tracked separately. Do not interpret this checkpoint
as verification of subsequent media/archive changes.

Linux final UI verification passed: 357 library tests and 28 actual interactions,
including all 15 keyboard/Previews cases. Final Windows UI library passed 360;
all seven settings topics passed the layout matrix. Full application workspace
Clippy/all targets and all twelve independent rustfmt checks passed. Source-size
and dependency inventory checks are complete. The associated changes are ready
for the final release smoke test and requested commit/push.
