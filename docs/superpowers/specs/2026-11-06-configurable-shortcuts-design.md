# Configurable application shortcuts

The user added this requirement while panel shell integration was in progress.
Every application shortcut uses a shared semantic action catalog and can be changed,
disabled, or restored in **Options → Keyboard**. Default bindings preserve the current
Commander behavior. Native text editing, IME composition, and standard widget editing
remain under the UI toolkit; operating-system-reserved keys may not reach the app.

Core owns the catalog, canonical single-chord parser, context policy, collision checks,
and an immutable compiled lookup. Stable action IDs are independent of captions and
default keys. Contexts distinguish ordinary files, filename search, command editing,
other controls, and the open volume chooser. Global actions overlap these contexts
for collision detection. The chooser-specific As Other Panel mnemonic is configurable
without consuming ordinary filename input.
Physical digit keys keep Hot Paths and numbered tabs consistent on Czech keyboards;
ASCII digit spelling and Digit0…Digit9 canonicalize to the same bindings;
AltGr never triggers a shortcut. No command strings or executable paths are bindings.
While editing a filename search, standard clipboard chords continue to edit its text,
even when assigned to another file action. Such file actions apply outside search.
Search-only actions cannot claim those reserved clipboard chords. The established
Ctrl+A file-selection and Shift+Delete confirmation actions remain available in search.

Settings store bounded per-action overrides; an empty key list disables an action and
an absent override uses its defaults. Existing settings load with default shortcuts.
Unknown actions, malformed chords, duplicates, oversized data and overlapping bindings
fail validation without partially applying. Options explains collisions before saving.
Version 7 preferences load/save on backend workers through existing revision handling;
versions 1–6 migrate with default shortcuts. No settings filesystem I/O enters the UI.

Routing, displayed shortcut hints and Help use the same catalog. Options includes an
action chooser, editable bindings, Apply, disable, per-action defaults and restore all.
Changing a binding removes its old aliases; restarting preserves the new binding.
Restoring defaults also replaces an unsaved editor draft when the effective binding
was already the default. Long bindings are elided in narrow footer buttons and remain
fully available in tooltips, accessibility labels and Help.
Tests cover context collisions, persistence/migration, disabled defaults, physical digits,
IME/text-input focus and real keyboard events for reassigned application actions.
