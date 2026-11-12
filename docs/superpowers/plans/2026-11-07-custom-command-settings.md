# Custom Commands settings and dynamic shortcut editor

Root assigns the complete settings editor under the accepted User Menu-like
configuration goal. Activation, toolbar, native process launch and engine hookup
remain root/backend ownership. Read `_build/goal/commands/integration-api.md` and
the accepted `2026-11-07-user-commands-design.md` before implementation. No new
dependencies or settings schema changes are needed; schema17/core contracts exist.

## UI and behavior

- Append **Custom Commands** as Options topic9, keeping all existing topic indices.
  Use the existing scrolling/narrow Options shell and accessible controls.
- Show the bounded ordered command list and an explicit edit form. New, Apply,
  Cancel, Delete, Move up/down operate on stable IDs. Rename/reorder preserve IDs;
  generate fresh IDs monotonically during the editor session and check collisions
  against loaded commands. Enable editing after initial preferences load.
- Fields: name, enabled, optional toolbar label, optional absolute image path,
  working-directory template, Program/Terminal kind and Single/Per item mode.
  Program uses an absolute executable and an unambiguous JSON argument list,
  matching the existing external-editor options convention. Terminal uses literal
  script, explicit shell and six scalar required-context checkboxes.
- A variable picker appends scalar working-directory tokens or whole Program
  arguments. Terminal insertion uses a quoted shell environment reference and
  adds its scalar declaration atomically. Never interpolate captured paths into
  script source. Grouped selection is available only as a whole Program argument
  in Single mode. Explain `$$` and the contextual Single/Per item distinction.
- Image paths are references only: supported images such as PNG/JPEG use the
  backend thumbnail/icon cache; missing/unsupported images use Run fallback.
  No UI filesystem probes, image file loads, executable inspection or launching.
- Use core parsing/validation for live input errors and Apply admission. Apply
  accepts the whole list through `replace_user_commands`, preserving all unrelated
  preferences and removing exactly deleted commands' shortcut overrides. Existing
  settings persistence handles asynchronous saves, unsaved state and atomic files.
- A weak captured configuration owner plus stable selected ID rejects stale form,
  delete and reorder callbacks. Never replace a dirty draft merely because an
  unrelated preference refresh occurs. Explicit selection/cancel/apply refreshes
  the form; validation-only updates preserve input focus/cursor.
- Preserve unchanged raw executable/icon paths when their UI display is lossy.
  Track actual path-field edits so editing another property cannot rewrite native
  bytes/UTF-16. Configuration/argument budgets are explicit core limits (128
  commands,64 argv,1MiB aggregate); no truncation or hidden shell conversion.

## Concrete ownership and sequence

1. NEW `presenter/user_command_settings.rs` and focused
   `presenter/user_command_settings_tests.rs`: selected/draft identity, validation,
   add/apply/cancel/delete/reorder, native-literal preservation and preference
   transactions. Root adds the narrow `presenter.rs` module/state/default entries
   after this API is ready, avoiding activation-module field conflicts.
2. Narrow `presenter/preferences.rs`: edited user_commands flag/merge, compare
   accepted configurations and call keyboard sync with configured commands.
3. `presenter/keyboard.rs`, `presenter/keyboard_editor.rs`, `shortcut_editor.rs`:
   use `ShortcutMap::with_user_commands` and its new read-only `definitions()`.
   Cache by overrides plus immutable command configuration identity; use a weak
   owner to prevent address reuse without retaining old large configurations.
   Preserve selected action ID through reorder/rename and clear stale candidate
   when that action disappears. Restore/assign/remove use the same dynamic catalog.
4. NEW `ui/user-command-options.slint` (and focused fields component if useful):
   global `UserCommandSettingsView`, bounded rows, form/draft data and callbacks.
   NEW `user_command_settings_presentation.rs`: convert small form/model data,
   register callbacks and publish revisions without resetting the draft on typing.
5. `ui/options-navigation.slint` and `ui/options-dialog.slint`: append topic9 in
   wide/narrow navigation and keyboard bounds, instantiate editor. Coordinate
   narrow `ui/main.slint` global import/export with root. Root owns toolbar/menu
   routing; expose topic9 so toolbar Settings can route there.
6. Narrow `src/lib.rs` module/install/refresh integration after root and Move's
   active source saves are complete. No edits to file_keys/shortcut activation,
   copy/selection_actions, toolbar, app startup, engine or platform sources.
7. Regressions: stable IDs, disabled state, duplicate names, every command kind,
   each variable/declaration, group/per-item errors, raw native unchanged paths,
   stale form and invalid Apply, refused saves, atomic deleted-ID shortcut cleanup,
   delayed preference load, config-sensitive keyboard cache and selection survival.
   Actual Options tests cover navigation, create/edit/delete/order, focus/cursor,
   keyboard assignment and all density/theme/narrow variants. Capture no user
   scripts/environment in diagnostic logs; use owned in-memory fixtures.

## Verification gates

- [x] Core-aligned presenter/settings and dynamic-keyboard tests RED→GREEN.
- [x] Actual Windows/Linux Options editor, drag/callback and layout tests GREEN.
- [x] Windows/Linux affected UI tests and strict Clippy, formatting/source size.
- [x] Report `_build/goal/commands/settings-report.md`; root activation remains a
  separate user workflow and cannot be inferred from settings tests alone.
