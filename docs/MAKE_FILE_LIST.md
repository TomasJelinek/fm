# Make File List

Press **Ctrl+M** in a panel, choose **Make File List** in the F10 Tools menu,
or use its document/list icon on the top toolbar. The shortcut is configurable.
The workflow follows Salamander's Make File List: export the selected displayed
files and folders, or the focused real entry when nothing is selected. `..` is
always excluded. Output follows the captured panel order, including active filters.

The resizable dialog is modal, with its own title/action icon and no separate
taskbar item. The preview fills the remaining space as the window grows;
Escape/Cancel restores the main window. Its input is an immutable capture, so
background refreshes do not retarget the export. Preview generation, formatting,
file access and clipboard access happen on backend workers.

## Template

The default `$(FileName)$(CRLF)` writes one name per line. The variable menu inserts
at the text cursor; presets supply names, full paths and metadata columns. The
last 30 unique successfully used templates are remembered.

| Variable | Meaning |
| --- | --- |
| `$(FileName)` | Complete file or folder name |
| `$(FileNamePart)` | File name without its last extension; complete folder name |
| `$(FileExtension)` | File extension without the dot; empty for folders |
| `$(FileSize)` | Ungrouped decimal bytes; `<DIR>` for an unmeasured folder |
| `$(FileDate)` / `$(FileTime)` | Modification date/time formatted using the system locale |
| `$(FileAttributes)` | Compact Windows attribute letters or Linux octal permissions |
| `$(Drive)` / `$(Path)` | Drive/share and remaining directory path, with a final separator |
| `$(CRLF)` / `$(CR)` / `$(LF)` / `$(TAB)` | Literal line endings and tab |

Variable names are case-insensitive. `$(FileName:max)` aligns to the longest
captured value. A numeric width such as `$(FileName:20)` pads or truncates to that
many Unicode characters, following Salamander; it never changes a filesystem
identity. `$$` writes `$`. `$[ENVIRONMENT_VARIABLE]` explicitly expands a backend
environment value. Missing metadata is `-`, not an invented value.

## Destinations

- **Clipboard:** copy UTF-8 text through the native clipboard service. The existing
  clipboard memory budget is 4 MiB; larger lists can use File or Internal Viewer.
- **Internal Viewer:** open an immutable generated text source through the basic
  viewer plugin. It supports the normal viewer search/scrolling controls and Escape.
- **File:** choose an output with the system Save dialog or enter its path. Output
  is UTF-8. Existing files require explicit confirmation showing their name, size
  and modification time. Append includes the old bytes before the new list.

File and viewer output are streamed through private temporary storage, without
loading the entire list into RAM. File publication uses verified staging; failed
or cancelled generation does not partly append to the original file. Destination
identity is retained through confirmation and checked again before publication.
Unsupported metadata preservation produces an error instead of silently weakening
the existing file's permissions. Recovery paths or durability warnings are shown
when the underlying filesystem cannot complete publication cleanly.

On success, the captured selection is cleared only if the same panel view and
selection revision are still current. Failed output keeps the dialog and selection.
Archive listings export their metadata directly; no member extraction is required.
Directory traversal is not implicit: this feature lists the captured visible items.

See the [design](superpowers/specs/2026-11-09-make-file-list-design.md) for the
backend contracts and the [feature status](FEATURE_STATUS.md) for validation scope.

DOS/8.3 name support is removed. Existing saved `$(FileDOSName)` and `$(DOSPath)`
tokens remain accepted only as migration aliases for the full Unicode filename
and directory path. They are no longer offered by the variable picker.
