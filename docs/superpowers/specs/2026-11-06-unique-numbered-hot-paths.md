# Unique numbered Hot Paths

Hot Paths store each native folder path once. Repeated Add preserves the existing
identity and compatibility name. New paths receive the first free slot 1–10;
additional paths remain accessible from the menus. Slot 10 is displayed as 0.
The drive bar shows numbered buttons. Bare 1–9/0 opens that slot in the focused
file panel; existing Ctrl/Shift aliases remain configurable in Options → Keyboard.
Unassigned digits are consumed without navigation. Search, command, path and
other text editors keep normal digit input, including composition and AltGr.
Existing custom bare-digit shortcuts take precedence over the new Hot Path
defaults. Only free digits acquire the default action; effective hints reflect
the retained binding. Conflicting explicit assignments still fail validation.

Manage Hot Paths shows full folder paths and assigned shortcuts, with no name
editor or redundant custom names. Selecting a row keeps the manager open. Apply
shortcut changes that selected identity, independent of the panel's current
folder. Moving a numbered path to an occupied slot swaps the two slots. An
unnumbered/new path taking an occupied slot leaves the displaced path unnumbered;
no unique location is removed. Add current and Remove keep the manager open.
The ordinary menu and drive bar continue to navigate when clicked.

Stored duplicates are merged deterministically in saved order: keep the first
identity, original native path and legacy name; if it has no slot, retain the
first later duplicate's slot. Other preferences remain intact. Startup edits
preserve already exposed callback identities and take precedence over loaded
duplicates. Validation prevents publishing duplicate paths. Storage still uses
the backend settings worker and existing lossless native-path codec.

Identity follows the project's existing `Path` equality used in navigation and
native path indexes. It is lexical, without filesystem access, symlink resolution,
case folding, Unicode normalization or conversion through display strings.
Standard `Path` component equivalences apply (ordinary repeated separators,
trailing separators and `.`); case variants, `..`, symbolic links and Windows
verbatim-prefix aliases are not resolved. In particular, `C:\folder` and
`\\?\C:\folder` remain distinct because verbatim paths have different parsing
semantics. This avoids merging distinct paths on case-sensitive filesystems and
preserves non-UTF-8/UTF-16 and long-path identity. Physical-alias deduplication
would require a separate backend filesystem identity contract.

The reference inspected was Salamander's `CCfgPageHotPath` in
`D:\experiments\salamander-main\src\dialogs4.cpp`, where list selection loads
management controls. Its separate name field is deliberately omitted here per
the requested path-only presentation.

Regression coverage exercises repeated adds, occupied-slot moves, first-free and
overflow slots, duplicate settings migration, startup reconciliation, raw native
paths, configurable digit bindings, real editor focus, and manager row selection.
