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

**Reset to defaults** resets the Options settings and keeps the tabs,
[Hot Paths](GLOSSARY.md#hot-paths) (the numbered favorite locations), window
placement, command history, [Custom Commands](developer/USER_COMMAND_SETTINGS.md)
and the settings kept from another operating system. OK saves the reset; Cancel
restores the previous settings.

File choice, reading, writing and validation run in backend services. The Options
dialog only shows the result.

## What a settings file contains

A settings file is a complete snapshot of everything Midnight Explorer saves:

- every Options page: appearance, fonts and spacing, the scrolling glide and speed, General choices, copy
  verification, disk I/O concurrency, editors, comparators, keyboard remaps,
  Custom Commands and their toolbar, filename colors, previews and thumbnails,
  viewer and thumbnailer plugin choices and associations, archive associations
  and custom archive extensions;
- the state saved outside Options: tabs of both panels, Hot Paths, window placement
  and the panel split, panel view modes, panel filters and filter history, the last
  Select or Deselect by mask pattern and its **Include folders** choice, viewer
  window settings, the terminal choice, checksum algorithm choices, and Make File
  List templates and output path;
- the command history. It is written only when **Remember command history after
  restart** is on. Commands run while it was off are never written.

A settings file contains folder paths, Hot Paths and command history. Treat it as
personal data.

Custom Command pictures are not in the file: it names each picture's file in the
application data's `icons` folder. Saving settings removes pictures that no saved
command has used for seven days, so a command imported from an older file can come
back without its picture ([managed icon snapshots](developer/USER_COMMAND_SETTINGS.md#managed-icon-snapshots)).

## Export configuration

1. In Options, choose **Export configuration…** (in the footer, or under **Settings
   file** at the end of the **General** page).
   Result: the **Export configuration** file dialog opens and suggests the name
   `midnight-explorer-settings.json`.
2. In the file dialog, choose the destination folder and file name.
3. Choose **Save**.
   Result: the line above the footer reports "Exported settings to *file*." If the
   file already exists, Options first asks whether to replace it
   ([Replacing an existing file](#replacing-an-existing-file)).

Export writes what **OK** would save at that moment. This includes changes on
Options pages that you have not yet confirmed with OK. Unfinished page edits, such
as an editor rule, comparator rule, Custom Command or color that is still being
edited, are not included: apply or discard them before you export. Exporting does
not change your settings and does not close Options.

### Replacing an existing file

When the chosen file already exists, Options asks "*file* already exists. Replace
it?":

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
  ([Make File List output safety rules](developer/MAKE_FILE_LIST.md#output-lifetime-and-safety));
- on Linux, a file that another user owns, that has setuid, setgid or sticky bits,
  or whose security attributes, permissions or filesystem flags could not be kept.

The file is staged exclusively, synced and read back for verification before it is
published, like the other exports. Like them, it waits until running Copy, Move
and other file operations have finished changing files. Closing Options does not
cancel an export that has started; its result then appears in the status bar, or
as a notice if it failed.

## Import configuration

1. In Options, choose **Import configuration…**.
   Result: the **Import configuration** file dialog opens.
2. Select the settings file and choose **Open**.
   Result: the whole file is read and checked before anything changes. An accepted
   file is previewed on every Options page; a refused one is named, with the reason,
   in the line above the footer.
3. Review the imported settings on the Options pages.
4. Choose **OK** to save the import, or **Cancel** to discard it.
   Result: [Saving or discarding an import](#saving-or-discarding-an-import) lists
   what each answer changes.

**OK** waits while the file is being read. **Reset to defaults** waits from choosing
the file until an import or an export has finished: an export saves the values shown
when its file is chosen.

### Refused files

A file is refused, with the reason shown above the footer, when:

- it is not a JSON settings document, or has no settings `version`;
- its `version` is newer than this version of Midnight Explorer reads;
- it contains a field this version does not know, or any invalid value, for
  example an unknown theme, an out-of-range font size, a duplicate Hot Path, an
  invalid filter or a key binding that conflicts with another;
- it is larger than 4 MiB, or is not an ordinary file.

A refused file changes nothing, including unfinished page edits. An import never
applies part of a file; only paths of another operating system are handled as
described in [Settings from another operating system](#settings-from-another-operating-system).

### Saving or discarding an import

An accepted file is previewed like other Options edits:

- Every Options page shows the imported settings at once, including the theme,
  application icon, fonts and keyboard remaps. Unfinished page edits are discarded.
- **OK** saves the whole file. Only after that save succeeds are the settings
  outside Options applied: the panels open the imported tabs, as at startup; the
  Hot Paths, panel split, view modes, filters, viewer settings, terminal choice,
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

- **Hot Paths** of the other system are kept but inactive. Manage Hot Paths and the
  Hot Path list show them marked, for example "Linux path, not available on
  Windows". Choosing one, or pressing its numbered shortcut, only explains this.
  An inactive Hot Path keeps its number until you give the number to another Hot
  Path, and you can remove it.
- **Tabs** at the other system's folders are not opened. A panel whose saved tabs
  are all such folders keeps its current tabs.
- **The editor, editor rules, Custom Commands, comparator rules and the Make File
  List output file** that use the other system's programs or folders are kept
  inactive. Options › General lists them under "Kept from Linux" or "Kept from
  Windows", where you can remove them. Removing is an Options edit: OK saves it,
  and Cancel keeps them.
- A Custom Command is also the other system's when its working folder is one of that
  system's paths, or when it runs in a shell that only that system's terminal runs:
  a POSIX shell script on Windows; a PowerShell or Command Prompt script on Linux.
  The list then shows that folder or the shell's name. This applies to a command you
  create with the other system's shell too, because it could not run here.
- While they are inactive, Midnight Explorer behaves as if they were absent. F4
  uses the next matching editor rule or the default editor, file comparison uses
  the next matching comparator rule, and those Custom Commands are not in the
  toolbar or menus. Choosing your own editor, or an output file in Make File
  List, replaces the kept one and removes it from the list; a settings file holds
  only one of each.
- **Key bindings** of kept Custom Commands are inactive too: Options › Keyboard
  does not list them and their keys do nothing here. They stay with their commands.
  If you give one of their keys to another action here, saving leaves that key out
  of the kept binding (and the binding itself once it has no key left), so that the
  file stays valid on the other system.
- Kept entries are saved and exported unchanged, and so are the key bindings of
  kept Custom Commands, apart from keys taken here as the **Key bindings** entry of
  this list describes. When the file returns to its own system, they work again
  there, in their original positions in their lists.

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
  at most 4 MiB. A leading UTF-8 byte order mark, as some text editors add, is
  ignored.
- `version` is the number of the settings schema. This version reads schema
  versions 1 to 45. Saves and exports use the lowest version that can hold the
  settings, at least 10.
- The other top-level fields belong to that schema version
  ([Top-level fields](#top-level-fields)). A field that the version does not define
  is an error, not ignored. The reader also checks that fields belong to the
  declared schema.

The update check's own state (the last check, GitHub's retry time and a skipped
version) is not a setting: it is kept in `update-state.json` in the private update
folder ([update-state.json](developer/UPDATES.md#update-statejson)).

### Recent schema versions

Historical schemas 34 through 44 require at least one of the fields each introduced,
because a save uses such a schema only for those fields. Schema 45 requires both theme
choices. A file without a field that the table names, including every file of an
earlier schema, reads as its default.

| Schema | Field it adds | Value | Written only when | Default |
| --- | --- | --- | --- | --- |
| 33 | `file_comparators` | the external comparison rules (`{"rules": [...]}`) | always from schema 33 on | no rules |
| 34 | `glyph_fitting` | Text rendering: `"fitted"`, `"snapped"` or `"subpixel"` | it is not the default | `"fitted"` |
| 34 | `thumbnailer_associations` | thumbnailer plugin choices per provider | there is a choice of your own | the catalog's choices |
| 35 | `show_hidden_files` | whether panels and inline trees list hidden entries (boolean) | it is off | `true` |
| 35 | `mask_selection` | the last Select or Deselect by mask: `{"pattern": <string>, "folders": <boolean>}` (Include folders) | it is not the default | pattern `"*"`, folders `false` |
| 36 | `viewer.document_layout` | `"continuous"`, the PDF layout chosen last | it is not the default Paged | Paged |
| 37 | `scroll_glide` | Options › Appearance › **Scrolling glide** stop as a percentage of the default glide: `0` (Off), `25`, `50`, `200` or `400`; other numbers are refused | it is not the default | `100` |
| 38 | `wheel_speed` | Options › Appearance › **Scrolling speed** stop as a percentage of the system's wheel notch: `50` (Very slow), `75`, `150` or `200` (Very fast); other numbers are refused | it is not the default Medium | `100` (Medium) |
| 39 | `invocation.output` of a [Run directly (Program)](developer/USER_COMMANDS.md#program-output) Custom Command | where its output goes: `{"window": "hidden"\|"output"\|"console", "close_on_success": <boolean>}` | it is not the default Hidden | Hidden |
| 40 | `updates` | Options › General › **Updates**: `{"check_at_startup": <boolean>, "previews": <boolean>}` (Check for updates at startup, Include preview releases) | one of them is off | both on, so settings saved before this version get the startup check |
| 41 | `find` | Find's accepted file-mask history (`mask_history`, newest first) and the Find window's placement (`window_geometry`) | there is one of them | no history, default placement |
| 42 | `nested` in an [archive address](#archive-addresses) | the archives inside archives of a Hot Path or saved tab (archive address `version` 3) | a saved location lies inside archives inside an archive | none; Hot Paths and tabs in one archive keep address version 1 or 2 and their older schema |
| 43 | `copy_preserve_metadata` | Options › General › File operations › **Preserve times and attributes in Copy and Move** (a boolean): how the Copy/Move dialog's option starts | it is off | `true`; existing explicit `true` and `false` choices are kept |
| 44 | `app_icon` | **Options › Appearance › Application icon**: `"origami-trail"` or `"interleave"` | it is not the default Harvest Navigator | Harvest Navigator; `"harvest-navigator"` is still read, and the retired Moon icon's `"moon"` reads as Harvest Navigator and is not written back |
| 45 | `dark_theme`, `light_theme` (replacing `theme`) | the palettes for dark and light appearance, for example `"blue"`, `"light"` | a custom dark or light palette is chosen; both are then saved | Blue for dark appearance and Light for light appearance |

The former per-dialog "Preserve exact modified time" choice was never saved, so
schema 43 migrates nothing else ([transfer options](developer/TRANSFER_OPTIONS.md)).
Earlier profiles, and files that keep both default palettes, open with Dark for dark
appearance (Blue until 0.6.3) and Light for light appearance and retain their other settings; files with
explicit choices keep them, including Pencil Lead and Letterhead.

### Top-level fields

**Since** is the first schema that may contain the field. A field marked
*required* must be present in every file of that schema and later ones; an
*optional* one is written only when its value differs from the default.

| Field | JSON type | Since | Presence | Meaning |
| --- | --- | --- | --- | --- |
| `version` | number | 1 | required | Settings schema, 1 to 45. |
| `density` | string | 1 | required | Spacing: `"desktop"` or `"touch"`. |
| `windows_shell` | string | 1 | optional | Windows terminal shell: `"powershell"` (default) or `"cmd"`. |
| `theme` | string | 1 | required in 1 to 44, absent in 45 | The single palette of earlier schemas; written as `"blue"`. It reads as the default palettes. |
| `dark_theme`, `light_theme` | string | 45 | required in 45 | Palettes for dark and light appearance. |
| `font_family` | string | 2 | required | Font name; empty selects the system default. Never a path. |
| `desktop_font_size`, `touch_font_size` | number | 2 | required | Font sizes for desktop and touch spacing. |
| `bookmarks` | array of [Hot Path records](#hot-path-records) | 3 | required | The Hot Paths, at most 64, including those of another system. |
| `quick_search_filter` | boolean | 4 | required | Show only matching names during panel type-to-search instead of jumping (default `false`). |
| `size_display` | string | 5 | required | `"readable"` or `"bytes"`. |
| `remember_command_history` | boolean | 5 | required | **Remember command history after restart**. |
| `command_history` | array of `{"command": <string>, "shell": "powershell"\|"cmd"\|"posix"}` | 5 | required | Command history, newest first; empty when the option is off. |
| `tabs` | array of two [tab records](#tab-records) | 6 | required | Tabs of the left and right panels. |
| `panel_view_modes` | array of two strings | 6 | required | `"detailed"`, `"brief"` or `"thumbnails"` for each panel. |
| `shortcuts` | array of `{"action": <action ID>, "keys": [<chord>, ...]}` | 7 | required | Only deviations from the default key bindings, including those of kept Custom Commands. |
| `panel_filters` | array of two [filter records](#filter-records) | 8 | required | The filter of each panel. |
| `editor` | object: `executable` ([path](#paths) or `null`), `arguments` (array of strings), `rules` (array, schema 30 on) | 9 | required | The F4 editor and editor rules. |
| `disabled_viewer_plugins` | array of strings | 10 | required | Viewer plugin IDs turned off. |
| `previews` | object: `enabled`, `network`, `archives` (booleans; `archives` from schema 31) | 12 | required | Previews and thumbnails, network previews, previews inside archives. |
| `archive_association_overrides` | object of extension → boolean | 13 | required | [Archive associations](ARCHIVE_FORMATS.md#associations) that differ from the default. |
| `filename_colors` | array, at most 32 rules | 14 | required | [Filename color rules](developer/FILENAME_COLOR_OPTIONS.md#saved-color-lists). |
| `window_geometry` | object: `normal` (`x`, `y`, `width`, `height`), `monitor` (`name`, `bounds`) or `null`, `maximized`, `fullscreen` | 15 | optional | Main window placement. |
| `checksum_algorithms` | array of 1 to 5 strings | 16 | required | Checksum algorithm choices. |
| `user_commands` | array | 17 | required | [Custom Commands and their toolbar](developer/USER_COMMAND_SETTINGS.md). |
| `archive_custom_extensions` | object of suffix → decoder ID | 20 | optional | [Custom archive extensions](ARCHIVE_FORMATS.md#custom-extensions). |
| `viewer` | object | 21 | required | [Viewer window settings](developer/VIEWER_PREFERENCES.md#viewer-window-choices). |
| `panel_split_basis_points` | number | 23 | required | Left panel width in basis points, 1 to 9999 (default `5000`). |
| `thumbnail_size` | number | 25 | required | Thumbnail edge in logical pixels (default `96`). |
| `file_list` | object | 26 | required | [Make File List templates and output path](developer/MAKE_FILE_LIST.md#preferences-and-ui). |
| `viewer_associations` | object | 27 | required | [Viewer plugin order and file types](VIEWER_ASSOCIATIONS.md#viewers). |
| `copy_checksum_verification` | boolean | 28 | required | `true`: full SHA-256 copy verification (default); `false`: length and three 64 KiB samples. |
| `panel_filter_history` | array of [filter records](#filter-records) | 30 | required | Up to 100 applied filters, newest first. |
| `disk_io_concurrency` | `{"mode": "automatic"}` or `{"mode": "manual", "limit": <number>}` | 31 | required from 32 | Disk I/O concurrency. |
| fields of schemas 33 to 45 | | 33 | see [Recent schema versions](#recent-schema-versions) | |

### Paths

Native paths are stored losslessly. A path value is one of:

| Form | Example | Used for |
| --- | --- | --- |
| JSON string | `"D:\\Projects"`, `"/home/user/Documents"` | a path that is valid Unicode |
| object `{"encoding": <string>, "hex": <string>}` | `{"encoding": "unix-bytes", "hex": "2f746d702fff"}` | a path that is not valid Unicode on its system |

| Field of the object | Type | Meaning |
| --- | --- | --- |
| `encoding` | string | `"windows-utf16"` for a Windows path, `"unix-bytes"` for a Linux path. |
| `hex` | string | The exact path in hexadecimal: four digits per UTF-16 code unit (`windows-utf16`), two digits per byte (`unix-bytes`). Saves write lowercase digits; reading accepts both cases. |

A path belongs to the other system when its text is absolute only in that system's
syntax (a Linux path begins with `/`; a Windows path has a drive letter or begins
with `\\`), or when its encoding names the other system. Such records are kept
exactly as stored.

A Custom Command belongs to the other system when its program or icon path does,
when its `working_directory` is such a path, or when its terminal `shell` is the
other system's: `posix` on Windows; `powershell` or `cmd` on Linux. The key binding
of a kept command stays in `shortcuts`, beside the others, so the schema is the same
on both systems. When saving, a kept binding keeps only the keys that leave the
whole `shortcuts` list valid for its command's own system.

### Hot Path records

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | number | Nonzero identity, unique in the file. |
| `name` | string | 1 to 512 UTF-8 bytes without control characters; kept for compatibility. |
| `path` | [path](#paths) | The absolute folder; for an archive or Network location, the native folder to return to. For an archive it is the archive file's folder. |
| `archive` | [archive address](#archive-addresses) | Optional, schema 19 on: a location inside an archive. |
| `network` | [Network address](#network-addresses) | Optional, schema 29 on: a Network location. A record has at most one of `archive` and `network`. |
| `slot` | number or `null` | The numbered shortcut: 1 to 9, or 10 for the zero key; unique in the file. |

### Tab records

| Field | Type | Meaning |
| --- | --- | --- |
| `paths` | array of [paths](#paths) | The panel's tabs in their displayed order, at most 32; for an archive or Network tab, the native folder to return to. |
| `active` | number | Index of the active tab. |
| `archives` | object of tab index → [archive address](#archive-addresses) | Optional, schema 11 on: tabs inside archives. |
| `networks` | object of tab index → [Network address](#network-addresses) | Optional, schema 24 on: Network tabs. |

### Filter records

| Field | Type | Meaning |
| --- | --- | --- |
| `enabled` | boolean | Whether the filter applies. |
| `mode` | string | `"masks"`, `"regex"` or, from schema 18, `"expression"`. |
| `expression` | string | The masks, regular expression or [metadata expression](developer/METADATA_FILTERS.md). |
| `case_sensitive` | boolean | Case-sensitive matching. |

### Archive addresses

An archive address names a folder inside an archive, by raw provider names: the
exact bytes that the archive plugin reports for each member and folder, stored as
JSON arrays of byte values (0 to 255), never as display text. So a member and a
folder of the same name stay distinct.

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | number | Address version: 1 (no decoder), 2 (with `format`, schema 20 on) or 3 (with `format` and `nested`, schema 42 on). |
| `provider` | string | The archive plugin's ID, 1 to 128 ASCII letters, digits, `.`, `-` and `_`, for example `"midnight.fs.archive"`. |
| `format` | string | Versions 2 and 3: the decoder of the archive file: `"zip"`, `"tar"`, `"gzip"`, `"tar-gzip"`, `"7z"`, `"xz"`, `"tar-xz"`, `"bzip2"`, `"tar-bzip2"`, `"zstd"` or `"tar-zstd"`. |
| `container` | [path](#paths) | The archive file, an absolute native path. |
| `source_version` | array of bytes | The archive file's revision when the location was saved, 1 to 256 bytes. |
| `components` | array of arrays of bytes | The folders inside the archive opened from `container`. With `nested`, the folder that holds the first nested archive member. |
| `nested` | array of nested archive records | Version 3: one record per archive opened from a member, the outermost first. |

| Field of a nested archive record | Type | Meaning |
| --- | --- | --- |
| `member` | array of bytes | The member's raw name in the enclosing archive's folder. |
| `format` | string | The decoder that opens the member, as `format` above. |
| `components` | array of arrays of bytes | The folders inside this archive. |

At most three nested records are allowed (four archives with the one opened from
the file). All names together are at most 256 parts and 64 KiB; no part may be
empty, `.` or `..`, or contain `/`, `\`, `:` or NUL. A deeper chain, an unknown
decoder or an invalid name makes the file invalid.

### Network addresses

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | number | Always `1`. |
| `provider` | string | The network filesystem plugin's ID. |
| `opaque` | array of bytes | The provider's own discovery address; never a native path or credentials. |
| `display` | string | The text shown for the location. |

Developers: the codec is `midnight_platform_common::settings`
(`encode_settings_file` / `decode_settings_file`). The backend service is
`midnight-engine`'s `preferences_file` module, and the Options staging is the
presenter's `preferences::settings_file` module. Entries of another system are
`UserPreferences::foreign_hot_paths` and `foreign_settings`
(`midnight_core::foreign_settings`); their stored records are interpreted only by
the settings codec. In memory, a kept Custom Command's record also holds its key
binding, which the codec moves out of `shortcuts` when reading and writes back when
saving.
