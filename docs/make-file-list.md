# Make File List

Press **Ctrl+M** in a panel, choose **Make File List** in the F10 Tools menu,
or use its document/list icon on the top toolbar. The shortcut is configurable.
The workflow follows Salamander's Make File List: export the selected displayed
files and folders, or the focused real entry when nothing is selected. `..` is
always excluded. Output follows the captured panel order, including active filters.
It works in folders, inside archives and in inline folder trees. While the panel's
listing is still loading it asks you to wait, and it is not available in a Network
location or while the panel shows Find results (Feed to panel).

The dialog is modal inside the main window, with its own title and icon; it can be
moved and resized, and the preview fills the space it gains. Escape or Cancel closes
it. Its input is an immutable capture, so background refreshes do not retarget the
export. Preview generation, formatting, file access and clipboard access happen on
backend workers. The preview shows the first 12 rows, at most 16 KiB.

## Template

The default `$(FileName)$(CRLF)` writes one name per line. **Insert variable**
inserts at the text cursor, replacing selected text. **Presets** replace the
template:

- **Names**: `$(FileName)$(CRLF)`;
- **Full paths**: `$(Drive)$(Path)$(FileName)$(CRLF)`;
- **Details**: `$(FileName:max)$(TAB)$(FileSize)$(TAB)$(FileDate) $(FileTime)$(TAB)$(FileAttributes)$(CRLF)`.

The line template is the dialogs' common edit: while it has the keyboard and text,
its × clears it. **Down** (or Alt+Down) in it lists the last 30 different templates
that produced a list, newest first; Up, Down, Home and End browse, Enter or a click
puts the template in the line, and Escape closes the list (a second Escape the
dialog). **Output** is the same kind of dropdown as Presets and Insert variable.

| Variable | Meaning |
| --- | --- |
| `$(FileName)` | Complete file or folder name |
| `$(FileNamePart)` | File name without its last extension; complete folder name |
| `$(FileExtension)` | File extension without the dot; empty for folders |
| `$(FileSize)` | Ungrouped decimal bytes. For a folder, its total when a folder size calculation finished without errors, otherwise `<DIR>` |
| `$(FileDate)` / `$(FileTime)` | Modification date/time formatted using the system locale |
| `$(FileAttributes)` | Windows attribute letters (`R`, `H`, `S`, `D`, `A`, …) or Linux permissions as four octal digits, such as `0644` |
| `$(Drive)` / `$(Path)` | Drive or share (empty on Linux) and the rest of the folder path, with a final separator |
| `$(CRLF)` / `$(CR)` / `$(LF)` / `$(TAB)` | Literal line endings and tab |

Variable names are case-insensitive. `$(FileName:max)` aligns to the longest
captured value. A width from 1 to 9999, such as `$(FileName:20)`, pads or truncates
to that many Unicode characters (code points, not screen columns), following
Salamander; it never changes a filesystem identity. `$$` writes `$`.
`$[ENVIRONMENT_VARIABLE]` explicitly expands a backend environment value. Missing
metadata is `-`, not an invented value. Salamander's `$(FileDOSName)` and
`$(DOSPath)` are accepted as `$(FileName)` and `$(Path)`; there are no 8.3 short
names, and the variable menu does not offer them.

A template has at most 64 KiB, and one expanded row at most 8 MiB. Generation
stops with a message for an unknown variable, a width other than 1–9999 or `max`, a
lone `$`, a missing `)` or `]`, an environment variable that is not set or not
Unicode, a name or folder that cannot be shown exactly as Unicode text, and a capture
without a real entry.

## Destinations

- **Clipboard:** copy UTF-8 text through the native clipboard service, of any size; a
  list too large for the memory left fails with "There is not enough memory to copy
  this text." (File or Internal viewer then still work).
- **Internal viewer:** open the generated text, as `File list.txt`, in the basic text
  viewer. It supports the normal viewer search/scrolling controls and Escape.
- **File:** choose an output with **Browse…** (the system Save dialog) or enter its
  path; it starts as the last output file, or `file-list.txt` in the panel's folder.
  Output is UTF-8. **Append to file** adds the new list after the old contents. An
  existing file needs confirmation (**Confirm existing output**, showing its path,
  size and modification time) with **Replace** or **Append**.

File and viewer output are streamed through private temporary storage, without
loading the entire list into RAM. File publication uses verified staging; failed
or cancelled generation does not partly append to the original file. Destination
identity is retained through confirmation and checked again before publication.
An existing file whose metadata cannot be kept exactly is refused instead of
silently weakening its permissions (the same rules as for
[replacing a settings file](settings-files.md#export-configuration)); on Windows a
read-only, hidden, compressed or encrypted file is replaced and keeps those
attributes. A confirmed
Replace or Append keeps the original file in a recovery copy (a
`.midnight-recovery-…` folder beside it), and a notice names that copy, as Shift+F4's
Replace file does. Durability warnings are shown when the underlying filesystem
cannot complete publication cleanly.

On success, the template, destination, Append choice and output file are remembered,
and the captured selection is cleared only if the same panel view and selection
revision are still current. Failed output keeps the dialog and selection.
Archive listings export their metadata directly; no member extraction is required.
Directory traversal is not implicit: this feature lists the captured visible items.

The dated [design](superpowers/specs/2026-11-09-make-file-list-design.md) describes
the backend contracts; [feature status](feature-status.md) records the feature's status.
