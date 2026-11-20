# Export and import configuration

Options has **Export configuration…** and **Import configuration…** in its footer,
next to **Reset to defaults**. Use them to keep a backup of your settings or to
move them to another computer. Both use the same file format as Midnight
Explorer's own settings file. In a narrow Options dialog the buttons show only
their icons; their tooltips keep the full names.

File choice, reading, writing and validation run in backend services. The Options
dialog only shows the result.

## What a settings file contains

A settings file is a complete snapshot of everything Midnight Explorer saves:

- every Options page: appearance, fonts and spacing, General choices, copy
  verification, disk I/O concurrency, editors, comparators, keyboard remaps,
  custom commands and their toolbar, filename colors, previews and thumbnails,
  viewer and thumbnailer plugin choices and associations, archive associations
  and custom archive extensions;
- the state saved outside Options: tabs of both panels, hot paths, window placement
  and the panel split, panel view modes, panel filters and filter history, the last
  Select or Deselect by mask pattern, viewer window settings, the terminal choice,
  checksum algorithm choices, and Make File List templates and output path;
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
such as checksum files and Make File List output. A file that is read-only,
compressed or encrypted, or is a folder or link, is not replaced and stays
unchanged; choose another name instead. This also applies inside an
NTFS-compressed folder, because new files there are compressed too.

The file is staged exclusively, synced and read back for verification before it is
published, like the other exports. Like them, it waits until running Copy, Move
and other file operations have finished changing files. Closing Options does not
cancel an export that has started; its result then appears in the status bar, or
as a notice if it failed.

## Import configuration

Choose **Import configuration…** and pick a settings file. The whole file is read
and checked before anything changes. **OK** and **Reset to defaults** wait while
it is being read.

A file is refused, with the reason shown above the footer, when:

- it is not a JSON settings document, or has no settings `version`;
- its `version` is newer than this version of Midnight Explorer reads;
- it contains a field this version does not know, or any invalid value, for
  example an unknown theme, an out-of-range font size, a duplicate hot path, an
  invalid filter or a key binding that conflicts with another;
- it is larger than 4 MiB, or is not an ordinary file.

A refused file changes nothing, including unfinished page edits. An import never
applies part of a file.

An accepted file is previewed like other Options edits:

- Every Options page shows the imported settings at once, including the theme,
  fonts and keyboard remaps. Unfinished page edits are discarded.
- **OK** saves the whole file. Only after that save succeeds are the settings
  outside Options applied: the panels open the imported tabs, as at startup; the
  hot paths, panel split, view modes, filters, viewer settings and terminal choice
  take effect; and the window moves to the imported placement, adjusted to the
  current displays. A panel without saved tabs keeps its current tabs, and a file
  without a saved window placement leaves the window where it is. Viewer windows
  that are already open keep their current view settings until reopened.
- **Cancel**, Escape or the X button discard the import: the previewed settings
  return to what they were, and nothing outside Options changes. **Reset to
  defaults** after an import also discards its settings outside Options.

If saving fails, Options stays open with the error. You can retry **OK** or
discard the import with **Cancel**.

Files from older versions of Midnight Explorer are accepted. Settings they do not
contain get their default values, exactly as when this version reads an older
settings file.

Paths are stored in each operating system's own form, and settings that require an
absolute path, such as tabs and hot paths, refuse a path from the other system. A
Windows file with such paths therefore cannot be imported on Linux, or the reverse;
the import is refused instead of being applied without those settings.

## File format

A settings file is the same JSON document that Midnight Explorer stores as its own
settings file: `%LOCALAPPDATA%\Midnight Explorer\preferences.json` on Windows, or
`$XDG_CONFIG_HOME/midnight-explorer/preferences.json` (by default
`~/.config/midnight-explorer/preferences.json`) on Linux.

- The document is one JSON object in UTF-8. Exports are indented and end with a
  newline. The whole file is at most 4 MiB.
- `version` is the number of the settings schema. This version reads schema
  versions 1 to 35. When exporting, it writes the lowest version that can hold the
  settings, at most 35.
- The other top-level fields belong to that schema version. A field that the
  version does not define is an error, not ignored.
- Native paths are stored losslessly. A path that is valid Unicode is a JSON string.
  Other paths use an object with the encoding name and the exact bytes in
  hexadecimal.
- A leading UTF-8 byte order mark, as some text editors add, is ignored.

Developers: the codec is `midnight_platform_common::settings`
(`encode_settings_file` / `decode_settings_file`). The backend service is
`midnight-engine`'s `preferences_file` module, and the Options staging is the
presenter's `preferences::settings_file` module.
