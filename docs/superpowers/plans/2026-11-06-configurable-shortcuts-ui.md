# Configurable shortcuts UI implementation plan

> **For agentic workers:** Use superpowers:executing-plans for this integration,
> with the existing parent reviewer. No new agents or commits in this subtask.

**Goal:** Route every application shortcut through the shared catalog and let users
change, disable, and restore bindings in Options → Keyboard.

**Architecture:** Core owns chord normalization, scopes, defaults and validation.
The presenter retains a compiled immutable `ShortcutMap` and updates existing
preferences revisions. UI routing reads that map before dispatching the existing
semantic callbacks; Slint widget editing continues to use the toolkit.

**Tech stack:** Existing Rust and Slint 1.18.1, Windows/Linux native event adapters.

**Spec:** [Approved keyboard design](../specs/2026-11-06-configurable-shortcuts-design.md).

## Global constraints

- Native text editing, IME and standard widget navigation remain native.
- Physical digits retain Czech keyboard behavior; AltGr never invokes shortcuts.
- All invalid changes fail validation without partial application or saving.
- Settings use the existing backend, revision ordering and startup reconciliation.
- Rustfmt 100 columns; source files under 100KB; no UI filesystem work.
- One Cargo process per platform; QA artifacts under `_build/qa/panel-shell/`.

## Review focus

- An edited shortcut replaces every old alias, including physical digit handling.
- A disabled default does not fall through to an old hardcoded handler.
- Editing command/search text or composing cannot publish file clipboard contents.
- Changes made before preferences load survive the delayed initial response.
- Options, Help and tooltips show the same effective bindings used by routing.

## Task 1: Presenter preferences and editor state

Files: new `crates/midnight-ui/src/presenter/keyboard.rs`; narrow changes in
`presenter.rs` and `presenter/preferences.rs`; tests in `presenter/tests/keyboard.rs`.

Consumes `ShortcutMap::new`, `shortcut_catalog`, `ShortcutBinding` and
`UserPreferences.shortcuts`. Produces a cached `Arc<ShortcutMap>`, selected action,
binding text/context/error presentation, and methods for apply, disable, restore
one and restore all. All mutations use `update_preferences` and an edited-field
flag for startup reconciliation.

- [x] Add failing tests: valid reassignment replaces aliases and saves one new
  revision; conflict and protected-native chord leave map/preferences unchanged;
  disable/defaults/all-defaults work; delayed preferences preserve local edits.
- [x] Run `scripts/cargo.ps1 test -p midnight-ui --lib keyboard --offline` and
  observe the missing behavior.
- [x] Implement the cached state and preference reconciliation.
- [x] Rerun the focused tests; ensure an unrelated preference change preserves keys.

## Task 2: Central routing and native physical digits

Files: `src/file_keys.rs`, `src/file_key_bridge.rs`, `src/native_shortcuts.rs`,
new focused `src/shortcut_input.rs`/`src/shortcut_bridge.rs` as needed;
`ui/main.slint`, `ui/controls.slint`, and the volume picker callback.

Consumes the presenter's immutable map (a default map in standalone UI tests).
Canonicalizes Slint keys and physical digits to `ShortcutChord`, resolves the
correct Files/Search/Command/Other/Volume context, and dispatches existing semantic
callbacks after releasing presenter borrows. Existing search editing return codes
remain private UI behavior. The volume popup's application mnemonic uses Volume
context; its toolkit arrows/Enter/Escape stay native.

- [x] Add failing tests for reassigned Copy/command execution/global action,
  disabled Enter, context-sensitive search, physical digit aliases and AltGr.
- [x] Verify every catalog action ID has a dispatch mapping.
- [x] Remove hardcoded application-key branches from main/file/native routing,
  retaining only toolkit editing and unsupported-modifier safeguards.
- [x] Run unit routing tests plus real keyboard integration tests. Check that the
  old keys no longer invoke application actions after reassignment or disabling.

## Task 3: Options Keyboard, hints and Help

Files: new `ui/keyboard-options.slint` and presentation/bridge module;
`ui/options-dialog.slint`, narrow `ui/main.slint` wiring, existing hint/help sources;
new `tests/keyboard_options.rs` plus relevant existing interaction tests.

Keyboard provides an action chooser, current bindings text (one chord per line),
scope explanation, Apply, Disable, Default and Restore all. Validation errors stay
inside Options. Selection changes load that action's current keys. Effective
bindings populate hints and Help from the same catalog/map; no parallel key list.

- [x] Add failing actual-event tests for Options reassignment/validation/reset,
  persistence dispatch, and changed hint/help text.
- [x] Implement the extracted Keyboard component and bounded cached models.
- [x] Replace application key labels with map-derived descriptions and keep empty
  binding captions clear (disabled actions do not advertise an old key).
- [x] Run real UI tests and the dialog layout test for all themes/densities/font32.
- [x] Run full UI tests and strict Clippy after the integration stabilizes; review
  source-size and formatting checks, then hand evidence to the parent reviewer.
