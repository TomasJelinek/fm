# Settings controls and confirmation defaults

The user's screenshot feedback authorizes these changes within the existing
Options transaction and backend contracts. No dependencies or settings schema
changes are required for the page merge or operation defaults.

- [x] Replace every argument composer with the shared ordered list: select a
  row, edit its literal argument, and use compact + / - / up / down actions in
  the list heading. Preserve empty arguments, Unicode, order, and placeholders.
- [x] Merge command-history controls into General (formerly Panel), with Panels
  and Command line headings.
  Remove Command from both wide and narrow navigation. Keep existing internal
  topic IDs stable so pending file pickers and validation still target the
  correct form. OK saves the combined draft; Cancel restores history/preferences.
- [x] Make the action button primary for Copy, Move and Delete confirmations.
  Copy/Move initially focus the destination editor. Delete initially focuses
  its action. Enter proceeds, including from the option checkbox; Space toggles
  a checkbox. Explicitly focused Cancel, Escape and the close button still cancel.
  Showing a dialog never starts work; empty named destinations cannot submit.
- [x] Verify real keyboard/pointer input, queue refusal, draft lifecycle, compact
  geometry and current settings regression tests. Capture representative renders.
- [x] Update design/evidence, build debug and verify the exact deployed artifact.

The reusable argument/header components are owned by the settings subagent.
The Plugins subagent independently handles catalog correctness, readable names
and Viewers/Filesystems subviews. The root owns the General merge and confirmation
defaults. These source boundaries avoid concurrent edits of the same forms.

Salamander reference: `src/lang/lang.rc` Copy/Move dialogs at lines 288-329 use
`DEFPUSHBUTTON "OK", IDOK`, retaining destination input. This application keeps
its captured-operation, verification and explicit permanent-delete policies.
This change does not select overwrite decisions automatically.

Final review also required Enter on the explicitly focused header X to cancel.
The new actual-focus regression failed before the key-capture correction and
passed afterward for ordinary and permanent deletion. Final scoped operations
tests and all 30 selected integration targets are green; see the
[qualification record](../../verification/2026-11-10-settings-polish.md).
