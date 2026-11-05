# Navigation, bookmarks, tabs and appearance

This extends the read-only native-filesystem milestone. The user's implementation
request authorizes these changes. File-writing operations retain their separate
milestone; copy concurrency is specified in the [copy design](2026-11-04-copy-concurrency-design.md).

The product version is0.1, stored centrally as Cargo SemVer0.1.0. The application
derives the native title `FM 0.1 — File Manager` from that version and supplies
it to Slint as presentation data; no extra in-content header is added.

## Native bookmarks and pane tabs

Each pane has an independent tab strip. A tab starts at the pane's current native
path. Its default caption is always the current folder basename, updated when
navigation changes the folder; roots display their root instead. The full path
is a plain-text tooltip. Captions are presentation, never navigation identities.
Different folders with the same caption remain distinct through typed `TabId`s.

Ctrl+T creates and activates a tab, Ctrl+W closes the active tab, Ctrl+Tab selects
the next and Ctrl+Shift+Tab the previous tab in the focused pane. The final tab
cannot be closed. Each pane supports 32 tabs. The strip also exposes accessible
select, close and add controls. Tab state is session-only in this milestone;
restoring tabs on restart, locked tabs, renaming and reordering can follow later.

A parked tab retains its raw path, back/forward history, sort settings, cursor
identity and remembered locations on individual volumes. Large directory
listings, selection, metadata and scans are released through backend retirement.
Activation queries the directory again and restores the cursor when that raw
entry still exists. Only the two visible tab locations are watched. Navigation
generations increase across all tabs sharing a pane; an old tab's listing,
metadata, size or open-directory result must never commit into the new tab.
Failure to enqueue a switch leaves the current tab usable.

An accessible star button opens saved paths for either pane. Users can name and
add the current folder, optionally assign a numbered slot, open a saved path,
or remove it. There are up to 64 bookmarks and ten numbered slots (1–9, 0 for
slot10). Ctrl+Shift+number assigns the current path to that slot; Ctrl+number or
Shift+number navigates there. Adding Alt targets the other pane without stealing
focus. Ctrl+Shift+Plus adds an unnumbered saved path. These shortcuts apply in
file lists, preserving text editing and command entry elsewhere. Assigning a
slot displaces the old assignment without deleting its bookmark.

Bookmarks use typed, non-reused session IDs and raw absolute `PathBuf`s. Clicking
a stale removed ID is harmless. No bookmark is executed as shell text or rebuilt
from a lossy display string. Unsupported or unavailable paths report a navigation
error. A bookmark does not imply that a target still exists or is trusted.

When archive/remote/registry providers arrive, the stored target evolves into a
typed provider-owned location through a schema migration. UI IDs and callbacks
stay independent of native paths; providers never infer their identity from a
caption, drive-looking prefix or shell syntax. Credentials remain separate.

Bookmarks persist with appearance settings in version4 UTF-8 JSON. Version4 adds
the optional live-search filter preference; versions1–3 migrate with it disabled.
Valid Unicode
paths are strings; host-native non-Unicode paths use tagged lossless encodings
in separate Windows/Unix codec modules. Foreign native encodings are rejected.
Version1/2 settings migrate without bookmarks. IDs, labels, slots, absolute paths,
NULs and duplicate identities/slots are validated. The aggregate settings payload
is bounded at 4MiB for resource safety; this is not a filesystem path-length cap.
An oversized save fails explicitly and preserves the previous settings file.
Early local edits and an asynchronous initial load must preserve unrelated saved
fields; older acknowledgments cannot overwrite newer choices.

Local previews before the initial load do not write a default snapshot over
unread settings. The initial result is merged by the explicitly edited field,
then the latest merged state is saved. Saved and newly added bookmarks are both
preserved, with local slot assignments taking precedence and IDs remapped only
for unseen saved items. If reconciliation temporarily exceeds64 bookmarks,
keep the bounded combined set in this session and block saving until the user
removes enough; never silently discard existing saved paths. A read error blocks
saves for that session to protect the unread file.
Closing first hides the window, then gives a pending initial read and merged
save the existing shared2500ms exit grace. Command/event queue saturation must
not deadlock reconciliation. Expiry preserves the old file; a blocked native
call is not claimed to have completed. Large queued data is retired on backend
workers even when that grace expires.

References inspected: Salamander `src/fileswn0.cpp:1220–1260` (hot-path assignment,
active/other-pane shortcuts), `src/fileswn1.cpp:2191` (navigation),
`src/mainwnd1.cpp` (named saved paths), and `src/salamand.rc:61–102`
(accelerators). Its ANSI buffers and fixed path limits are not adopted. Folder
tab conventions also follow the [Total Commander discussion](https://www.ghisler.ch/board/viewtopic.php?t=42163).
No upstream source/assets are copied.

## Compact controls and clear size estimates

Pane status bars are removed. Errors retain explicit feedback/dialogs; ordinary
success text does not occupy a permanent footer. Desktop F1–F10 buttons use a
22px base height; compact touch targets use a40px baseline. Unclear and disabled controls
have tooltips and accessible labels. Text metrics may grow the controls.

Final successful folder sizes display normally. Active running/queued results
use a contrasting color without `!`; known partial totals also use `≈`.
Completed partial/cancelled totals retain their partial marker but ordinary
styling. File/folder rows have no hover tooltips; F1 explains the states. An unscanned
directory shows `<DIR>`, never its filesystem directory-entry size as a recursive
total. Sorting preserves estimate state and uses backend sorting.

Salamander-compatible implemented aliases include Alt+F1/F2 (left/right drives),
Ctrl+F3/F4/F5/F6 (name/type/time/size sort), Ctrl+F9 (refresh),
Ctrl+Shift+F10 (all folder sizes), Ctrl+G (path editor), Ctrl+PgUp (parent),
Ctrl+PgDown (enter a known folder), Ctrl+F1/Ctrl+L (cached drive information),
and Ctrl+Backspace/Ctrl+Backslash (native root). Ctrl+PgDown cannot launch a file.
Existing Ctrl+R and Ctrl+Shift+S remain aliases. Unsupported operations remain
disabled rather than borrowing an unrelated command's shortcut.

The explicit requested overrides are Ctrl+Enter to append the focused basename
to the end of the command draft, and Ctrl+P to insert the active pane path at the
editor selection. Ctrl+[ and Ctrl+] retain left/right insertion. Backend-provided
raw names are quoted with the draft's shell grammar; unsafe insertion contexts
and unrepresentable names fail explicitly. These actions focus the command
editor and never submit, activate a file or start a terminal.

## Fonts and settings

Options selects an installed font family and independent Desktop/Touch base
sizes, default11/15 logical pixels and permitted8–32. Empty family means the
application default. Other text styles derive their size from the active base;
controls grow to fit, and touch targets retain their minimum. Ordinary strings
and metadata elide visually with full values available in tooltips.

The later user request for a smaller Touch profile supersedes the previous44px
baseline. Touch now uses40px rows/controls/bottom buttons,48px main toolbar/path/
location/command bands,10px window padding,6px gaps/control padding and20px
icons. Font-driven growth still applies. Existing user font family and size
preferences remain intact; this change reduces density metrics, not saved font
choices. Desktop geometry is unchanged.

Font enumeration belongs to `SettingsService` and a bounded cached backend
worker, requested lazily when Options opens. Windows uses native GDI enumeration
in its own adapter; Linux uses safe fontdb against standard system/user font
directories. Fontconfig-only custom locations are a later extension. The UI
uses a virtualized model and menu instead of constructing thousands of entries
on every refresh. A missing font falls back through the renderer's font support.

Storage is asynchronous and atomic through the existing native settings service:

- Windows: the native LocalAppData folder, normally `%LOCALAPPDATA%\FM\preferences.json`.
- Linux: `$XDG_CONFIG_HOME/fm/preferences.json`, otherwise `~/.config/fm/preferences.json`.

The same document stores theme, density, shell preference, family, both sizes
and bookmarks, plus the live prefix-search filter choice. It never stores commands, credentials, sudo passwords or elevated
mode. Native readers reject special files and symlink/reparse targets. Failures
are visible and leave session choices usable.

## Pixel scrolling and native drive icons

Panels retain virtualized rows and Slint's native scrolling physics. Wheel
motion is pixel-based with180ms interpolation; touch/pad inertia continues until
idle. After80ms of quiet, a100ms ease-out settles at the nearest row. Held pointer
gestures, continuing momentum and low-frame-rate updates postpone settling.
Bounds win at the exact end when the viewport is not a whole number of rows;
no blank gap is introduced. Programmatic cursor reveal cancels motion. Refresh
alone does not arm a snap or move a stable viewport. No native input is consumed
by the passive Winit phase tracker.

Native numbered shortcuts use physical digit keys so shifted punctuation and
Czech layouts keep the same slots. Right Alt/AltGr, editors, modal popups,
synthetic events and unsupported modifier combinations propagate without bookmark
actions. Held repeats do not repeatedly save. The Slint logical-key fallback is
disabled when real Winit input handling is authoritative, preventing duplicate
actions and AltGr fallthrough.

Drive captions and expanded choices have a leading cached system icon keyed by
the exact native root. Windows queries the Shell only for validated local fixed
logical-drive roots, preserving system-volume decoration. Network, removable,
optical and unknown media use host stock icons to avoid offline/media probes.
Linux uses the host icon theme and conservative drive class. Backend workers
own all icon acquisition and native resources; UI refresh and hover only read
cached images. Paths, labels and icons are not interchangeable identities.

## Verification obligations

Selection mapping: Ctrl+A select all, Ctrl+D clear all, Ctrl+I invert the active
panel's selection. These bindings are scoped to file lists; editors retain
normal text selection. Backend selection snapshots and scan continuity are
specified in [the current extension](2026-11-04-selection-and-continuous-sizes-design.md).
Salamander's Ctrl+Num+/Num-/Num* aliases remain candidates for a later stage.

Test raw Unicode/native paths, migration and malformed settings, slot replacement,
stale IDs, queue failure, tab limits/final-tab protection, independent history and
sort, default caption updates, stale result rejection and shutdown retirement.
Exercise actual installed bridge and keyboard/pointer events, not only callbacks.
Cover Options at large sizes, virtual font lists, partial folder sizes, wheel and
touch settling, exact bounds, virtualized large lists and root-specific icon
cache/native API policies. Run Windows and Linux workspace tests, strict Clippy,
formatting, source-size guards and application builds; inspect a fresh native
Windows screenshot and leave a visible current app instance running.
