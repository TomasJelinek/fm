# Export and import configuration

Options has **Export configuration…** and **Import configuration…** in its footer,
next to **Reset to defaults**. Use them to keep a backup of your settings or to
move them to another computer. Both use the same file format as Midnight
Explorer's own settings file. In a narrow Options dialog, or with a large font,
the buttons show only their icons; their tooltips keep the full names. Where the
footer has no room even for their icons, in a very small window or with a very
large font, they move to the end of the **General** page, under **Settings
file** (as **Export…** and **Import…** in a narrow dialog), so that **OK** and
**Cancel** always stay in view.

**Reset to defaults** resets the Options settings and keeps the tabs, hot paths,
window placement, command history, custom commands and the settings kept from
another operating system. OK saves the reset; Cancel restores the previous settings.

File choice, reading, writing and validation run in backend services. The Options
dialog only shows the result.

## What a settings file contains

A settings file is a complete snapshot of everything Midnight Explorer saves:

- every Options page: appearance, fonts and spacing, the scrolling glide and speed, General choices, copy
  verification, disk I/O concurrency, editors, comparators, keyboard remaps,
  custom commands and their toolbar, filename colors, previews and thumbnails,
  viewer and thumbnailer plugin choices and associations, archive associations
  and custom archive extensions;
- the state saved outside Options: tabs of both panels, hot paths, window placement
  and the panel split, panel view modes, panel filters and filter history, the last
  Select or Deselect by mask pattern and its **Include folders** choice, viewer
  window settings, the terminal choice, checksum algorithm choices, and Make File
  List templates and output path;
- the command history. It is written only when **Remember command history after
  restart** is on. Commands run while it was off are never written.

A settings file contains folder paths, hot paths and command history. Treat it as
personal data.

## Export configuration

1. Choose **Export configuration…** and pick a destination. The suggested name is
   `midnight-explorer-settings.json`.
2. Export writes what **OK** would save at that moment. This includes changes on
   Options pages that you have not yet confirmed with OK. Unfinished page edits,
   such as an editor rule, comparator rule, custom command or color that is still
   being edited, are not included. Apply or discard them first.
3. The line above the footer reports the result. Exporting does not change your
   settings and does not close Options.

If the chosen file already exists, Options asks whether to replace it:

- **Replace** writes over it. Where the filesystem supports it, the replaced file
  is kept next to it for recovery, and the message names that file.
- **Keep** leaves the existing file unchanged and exports nothing.

Replacing follows the same safety rules as the other exports that replace a file,
such as checksum files and Make File List output. These files are not replaced and
stay unchanged; choose another name instead:

- a folder, a link or a special file;
- on Windows, a file that has alternate data streams or extended attributes, or whose
  contents may not be on this computer (an offline or cloud file). A read-only,
  hidden, compressed or encrypted file is replaced and keeps those attributes; a file
  whose encryption cannot be kept is not replaced
  ([the rules](developer/MAKE_FILE_LIST.md#output-lifetime-and-safety));
- on Linux, a file that another user owns, that has setuid, setgid or sticky bits,
  or whose security attributes, permissions or filesystem flags could not be kept.

The file is staged exclusively, synced and read back for verification before it is
published, like the other exports. Like them, it waits until running Copy, Move
and other file operations have finished changing files. Closing Options does not
cancel an export that has started; its result then appears in the status bar, or
as a notice if it failed.

## Import configuration

Choose **Import configuration…** and pick a settings file. The whole file is read
and checked before anything changes. **OK** waits while it is being read.
**Reset to defaults** waits from choosing the file until an import or an export has
finished: an export saves the values shown when its file is chosen.

A file is refused, with the reason shown above the footer, when:

- it is not a JSON settings document, or has no settings `version`;
- its `version` is newer than this version of Midnight Explorer reads;
- it contains a field this version does not know, or any invalid value, for
  example an unknown theme, an out-of-range font size, a duplicate hot path, an
  invalid filter or a key binding that conflicts with another;
- it is larger than 4 MiB, or is not an ordinary file.

A refused file changes nothing, including unfinished page edits. An import never
applies part of a file; only paths of another operating system are handled as
described in [Settings from another operating system](#settings-from-another-operating-system).

An accepted file is previewed like other Options edits:

- Every Options page shows the imported settings at once, including the theme,
  application icon, fonts and keyboard remaps. Unfinished page edits are discarded.
- **OK** saves the whole file. Only after that save succeeds are the settings
  outside Options applied: the panels open the imported tabs, as at startup; the
  hot paths, panel split, view modes, filters, viewer settings, terminal choice,
  checksum algorithm choices, Make File List templates and output path and the last
  mask choice take effect; and the window moves to the imported placement, adjusted
  to the current displays. A panel without saved tabs keeps its current tabs, and a
  file without a saved window placement leaves the window where it is. Viewer
  windows that are already open keep their current view settings until reopened.
- **Cancel**, Escape or the X button discard the import: the previewed settings
  return to what they were, and nothing outside Options changes. **Reset to
  defaults** after an import also discards its settings outside Options.

If saving fails, Options stays open with the error. You can retry **OK** or
discard the import with **Cancel**.

Files from older versions of Midnight Explorer are accepted. Settings they do not
contain get their default values, exactly as when this version reads an older
settings file.

## Settings from another operating system

A settings file exported on Linux can be imported on Windows, and the reverse.
Paths are stored in each system's own form, and Midnight Explorer never turns a
path of one system into a path of the other. On Windows, `/home/user` would
otherwise mean that folder on the current drive. Instead:

- **Hot paths** of the other system are kept but inactive. Manage Hot Paths and the
  hot path list show them marked, for example "Linux path, not available on
  Windows". Choosing one, or pressing its numbered shortcut, only explains this.
  An inactive hot path keeps its number until you give the number to another hot
  path, and you can remove it.
- **Tabs** at the other system's folders are not opened. A panel whose saved tabs
  are all such folders keeps its current tabs.
- **The editor, editor rules, custom commands, comparator rules and the Make File
  List output file** that use the other system's programs or folders are kept
  inactive. Options › General lists them under "Kept from Linux" or "Kept from
  Windows", where you can remove them. Removing is an Options edit: OK saves it,
  and Cancel keeps them.
- A custom command is also the other system's when its working folder is one of that
  system's paths, or when it runs in a shell that only that system's terminal runs:
  a POSIX shell script on Windows; a PowerShell or Command Prompt script on Linux.
  The list then shows that folder or the shell's name. This applies to a command you
  create with the other system's shell too, because it could not run here.
- While they are inactive, Midnight Explorer behaves as if they were absent. F4
  uses the next matching editor rule or the default editor, file comparison uses
  the next matching comparator rule, and those custom commands are not in the
  toolbar or menus. Choosing your own editor, or an output file in Make File
  List, replaces the kept one and removes it from the list; a settings file holds
  only one of each.
- **Key bindings** of kept custom commands are inactive too: Options › Keyboard
  does not list them and their keys do nothing here. They stay with their commands.
  If you give one of their keys to another action here, saving leaves that key out
  of the kept binding (and the binding itself once it has no key left), so that the
  file stays valid on the other system.
- Kept entries are saved and exported unchanged, and so are the key bindings of
  kept custom commands, apart from keys taken here as described above. When the file
  returns to its own system, they work again there, in their original positions in
  their lists.

The import result line says what was kept and what was left out. For example:
"Kept inactive because they use Linux paths: 2 hot paths, 1 custom command and
the editor. Not opened: 1 tab at a Linux folder. Kept inactive with those custom
commands: 1 key binding."

## File format

A settings file is the same JSON document that Midnight Explorer stores as its own
settings file: `%LOCALAPPDATA%\Midnight Explorer\preferences.json` on Windows, or
`$XDG_CONFIG_HOME/midnight-explorer/preferences.json` (by default
`~/.config/midnight-explorer/preferences.json`) on Linux. While that file does not
exist, Midnight Explorer reads the settings of releases from before its rename,
`%LOCALAPPDATA%\FM\preferences.json` or `$XDG_CONFIG_HOME/fm/preferences.json`, and
saves to the current location; it never changes the old file.

- The document is one JSON object in UTF-8. Exports are indented (compact only
  when indentation would pass the limit) and end with a newline. The whole file is
  at most 4 MiB.
- `version` is the number of the settings schema. This version reads schema
  versions 1 to 45. Saves and exports use the lowest version that can hold the settings.
  The latest schemas added:
  - 33: `file_comparators`, the external comparison rules;
  - 34: `glyph_fitting` (Text rendering) and `thumbnailer_associations`;
  - 35: `show_hidden_files` and `mask_selection` (the last Select or Deselect by mask
    pattern and its Include folders choice);
  - 36: `viewer.document_layout` (`"continuous"`, the PDF layout chosen last),
    written only when it is not the default Paged;
  - 37: `scroll_glide`, the Options › Appearance › **Scrolling glide** stop as a
    percentage of the default glide: `0` (Off), `25`, `50`, `200` or `400`. It is
    written only when it is not the default (`100`); a file without it, including
    every file of an earlier schema, reads as the default. Other numbers are refused;
  - 38: `wheel_speed`, the Options › Appearance › **Scrolling speed** stop as a
    percentage of the system's wheel notch: `50` (Very slow), `75`, `150` or `200`
    (Very fast). It is written only when it is not the default Medium (`100`); a file
    without it, including every file of an earlier schema, reads as Medium. Other
    numbers are refused;
  - 39: `invocation.output` of a Run directly (Program) Custom Command: where its output
    goes (`{"window": "hidden"|"output"|"console", "close_on_success": <bool>}`), written
    only when it is not the default Hidden; a command without it, including every command
    of an earlier schema, reads as Hidden;
  - 40: `updates`, Options › General › **Updates**:
    `{"check_at_startup": <bool>, "previews": <bool>}` (Check for updates at startup,
    Include preview releases). It is written only when one of them is off; a file
    without it, including every file of an earlier schema, reads as both on, so
    settings saved before this version get the startup check;
  - 41: `find`, Find's accepted file-mask history (`mask_history`, newest first) and the
    Find window's placement (`window_geometry`), written only when there is one of them;
  - 42: `nested` in the archive address of a hot path or saved tab inside archives
    inside an archive (archive address `version` 3, which also needs `format`): one
    object per archive opened from a member, the outermost first, as
    `{"member": <raw name bytes>, "format": <decoder>, "components": [<raw folder
    name bytes>, …]}`. The address's own `components` then name the folder of the
    archive opened from `container` that holds the first member. Members and folders
    are stored as raw provider names, never as display text, so a member and a folder
    of the same name stay distinct. At most three levels (four archives with the one
    opened from the file); a deeper chain, an unknown decoder or an invalid name makes
    the file invalid. Hot paths and tabs in one archive keep address version 1 or 2
    and their older schema;
  - 43: `copy_preserve_metadata`, Options › General › File operations › **Preserve
    times and attributes in Copy and Move** (a boolean), how the Copy/Move dialog's option
    starts. It is written only when it is off; a file without it, including every file of
    an earlier schema, reads as on. Existing explicit `true` and `false` choices are kept.
    The former per-dialog "Preserve exact modified time" choice was never saved, so
    nothing else migrates
    ([transfer options](developer/TRANSFER_OPTIONS.md)).
  - 44: `app_icon`, the design chosen in **Options → Appearance → Application icon**.
    Choosing Origami Trail or Interleave stores `origami-trail` or `interleave`. The
    default, Harvest Navigator, is not written; a file without `app_icon`, including
    every file of an earlier schema, reads as Harvest Navigator. `harvest-navigator`
    is still read. The retired Moon icon's identifier, `moon`, reads as Harvest
    Navigator and is not written back.
  - 45: choosing a custom dark or light palette saves both `dark_theme` and
    `light_theme`. Earlier profiles, and files that keep both defaults, open with Blue
    for dark appearance and Letterhead for light appearance and retain their other
    settings. Pencil Lead, the dark default of the first 0.6.0 builds, is a choice
    like any other: a file that names it keeps it.

  Historical schemas 34 through 44 require at least one of the fields each introduced.
  Schema 45 requires both theme choices. The reader also checks that fields belong to
  the declared schema. The update check's own state (the last check,
  GitHub's retry time and a skipped version) is not a setting: it is kept in
  `update-state.json` in the private update folder
  ([Updates](developer/UPDATES.md#check)).
- The other top-level fields belong to that schema version. A field that the
  version does not define is an error, not ignored.
- Native paths are stored losslessly. A path that is valid Unicode is a JSON string.
  Other paths use an object with the encoding name (`windows-utf16` or
  `unix-bytes`) and the exact bytes in hexadecimal.
- A path belongs to the other system when its text is absolute only in that
  system's syntax (a Linux path begins with `/`; a Windows path has a drive letter
  or begins with `\\`), or when its encoding names the other system. Such records
  are kept exactly as stored.
- A custom command belongs to the other system when its program or icon path does,
  when its `working_directory` is such a path, or when its terminal `shell` is the
  other system's: `posix` on Windows; `powershell` or `cmd` on Linux. The key
  binding of a kept command stays in `shortcuts`, beside the others, so the schema
  is the same on both systems. When saving, a kept binding keeps only the keys that
  leave the whole `shortcuts` list valid for its command's own system.
- A leading UTF-8 byte order mark, as some text editors add, is ignored.

Developers: the codec is `midnight_platform_common::settings`
(`encode_settings_file` / `decode_settings_file`). The backend service is
`midnight-engine`'s `preferences_file` module, and the Options staging is the
presenter's `preferences::settings_file` module. Entries of another system are
`UserPreferences::foreign_hot_paths` and `foreign_settings`
(`midnight_core::foreign_settings`); their stored records are interpreted only by
the settings codec. In memory, a kept custom command's record also holds its key
binding, which the codec moves out of `shortcuts` when reading and writes back when
saving.
