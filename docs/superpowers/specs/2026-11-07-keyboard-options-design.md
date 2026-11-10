# Settings and shortcut editor refinement

The user supplied Visual Studio's Keyboard options as the interaction reference.
Replace the small action dropdown and multiline shortcut syntax editor with a
searchable command list and explicit assignment workflow. Keep the current settings
storage, action identities, native key normalization and safe input contexts.

The keyboard page contains:

- Search commands by label or stable action ID, case-insensitively; virtualize the list.
- A selected-command heading and its current shortcuts in a compact chooser. Remove
  affects only the selected alias; an empty list intentionally disables the action.
- A focused `Press shortcut keys` capture field. Record one chord, including function
  keys and physical digits. Consume captured shortcuts before application dispatch.
  Tab and Escape can also be recorded and leave the capture field without executing
  the application's command, so existing bindings for these keys remain editable.
- Show the effective `Use in` context and commands currently using the captured chord,
  with their contexts. Overlapping conflicts disable Assign; no silent reassignment.
- Assign adds one alias without removing existing aliases; enforce the existing four
  aliases per action, canonical deduplication, native-editing and IME restrictions.
- Restore command and Restore all defaults, with confirmation for the global reset.
  Stage changes in the Options transaction; OK persists the complete session,
  while Cancel restores its original shortcut mappings.

Scopes remain defined by action behavior and text-sensitive key rules. The reference's
arbitrary context chooser and Visual C++ scheme are not copied as inactive controls;
this application currently has one default map plus user overrides. No new shortcut
serialization version or multi-chord sequences are needed for this UI change.

Polish settings overall with a larger bounded window, consistent topic navigation,
aligned labels, readable section spacing and a fixed footer. The keyboard command
list receives most of the available height; long errors scroll within their own card.
Preview settings immediately; preserve explicit save-failure feedback. The
[OK/Cancel transaction](2026-11-08-options-transaction.md) governs all pages.

Tests must use actual input events to verify capture never opens/deletes/executes,
physical digits and AltGr remain correct, search preserves selected command identity,
alias removal is precise, conflicts are shown before save, and saved overrides still
route correctly after reopening. Render desktop/touch, dark/light and large fonts.
