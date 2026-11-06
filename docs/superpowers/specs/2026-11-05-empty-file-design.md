# Empty file creation

The panel toolbar exposes **New empty file**, also available with **Shift+F4**
while a file panel has focus. A short dialog accepts a single Unicode filename
in the captured panel directory. Enter confirms and Escape cancels. Successful
creation refreshes the source panel and focuses the new entry. This action does
not start an editor; editing remains a separate F4 command.

Revealing a created entry exits the previous quick search/filter in that panel so
the new name is visible. A manual caret change after the refresh starts takes
precedence over automatic focus. If the bounded command queue rejects dialog
confirmation or cancellation, the dialog retains its draft and keyboard focus.

Reuse the prepared-directory mutation queue and name dialog instead of adding a
second creation workflow or running shell commands. This is an implementation
choice consistent with the approved backend-only architecture. Salamander uses
Shift+F4 for Edit New File (`src/salamand.rc`, `CM_EDITNEW` and
`CFilesWindow::EditNewFile`); no source code is copied.

The backend captures and revalidates the native base directory. Windows uses a
handle-relative exclusive NT create; Linux uses exclusive no-follow creation
relative to a retained directory descriptor. The name is one basename, with no
traversal, alternate stream, root, or path components permitted by the native
provider's validation. OS-native filename rules still apply. There is no app
MAX_PATH limit or lossy native-path conversion.

Existing files, directories, links, and concurrent name collisions fail without
overwriting or truncation. The job reports errors through the existing operation
notice and permits a fresh attempt. Cancellation before the native call creates
nothing; a successful native create remains a completed zero-byte file. Native
operations and handle cleanup remain on backend workers. As with other native
mutations, opened-directory identity cannot exclude every concurrent same-user
rename of an ancestor; existing platform safety notes apply.

Tests cover Unicode, raw native names where supported, zero length, unchanged
collision targets, concurrent exclusive creation, cancelled requests, replaced
base identity, invalid names, and long Windows parent paths. Engine tests prove
confirmation, captured destination, idempotence and cancellation; UI tests prove
toolbar and shortcut routing, name entry, Escape and text-field focus isolation.
