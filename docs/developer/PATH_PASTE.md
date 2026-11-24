# Pasting folder paths

The top panel path editor treats keyboard Paste (Ctrl+V or Shift+Insert) and its
context-menu Paste as the same explicit action. Paste inserts exactly the clipboard
text: it adds no separator and removes none, so a full path pasted with or without a
final `\` or `/` (a drive path, a UNC path or a Linux path) stays as copied. An empty
paste is unchanged. Existing text after the selection stays in place. Ordinary typing
is not modified.

Spaces, quotes, combining characters and other Unicode are literal. Paste does
not trim, unquote, normalize, expand variables, interpret URIs or open a folder.
Enter, or the editor's Go button, is the navigation action. Breadcrumb/provider
identities and unchanged raw native paths keep their navigation routes.

Enter and Go take edited text without its surrounding whitespace and without one pair
of surrounding double quotes, which Windows "Copy as path" adds. The unchanged
location text keeps its exact identity. A native path that names a file (or a link to one, a special file or
an archive file) opens the file's folder with the cursor on the file. The listing
worker that failed to list the path asks the provider what it names
(`browse_directory_identity`, following links) and sends `Event::ListingNotAFolder`
before the failure; only the path editor's request, while still pending, then lists
the folder with the file as its focus hint. The worker asks only after a "not a
directory" failure (`io::ErrorKind::NotADirectory`); the installed native filesystem
package, the application's only browsing provider, fails a listing of a file that way
(see the [native filesystem API](NATIVE_FILESYSTEM_API.md)). Other navigations keep their failure and
recovery, and a missing path its error. A final separator does not make a file's
path a folder's. A file that the panel's own
listing shows gets the cursor without a navigation. A Windows folder ignores letter
case, so when its listing has no entry of exactly the typed name, the one entry whose
name differs only in letter case (simple uppercase mapping of each UTF-16 unit, as
NTFS compares names) gets the cursor and the file goes by that name; with two or more
such entries (a folder that keeps letter case apart) none is chosen. Linux names match
exactly. While the panel does not show the file, a note leads the panel's status line
and says why: hidden files are not shown, the panel filter leaves it out, or the
folder does not list it under that name. An inline tree, a quick search or search
results are left as by any navigation; the unchanged text of an archive folder and
Network locations keep their routes. The editor's tooltips say that a file's path
opens the file's folder with the cursor on the file. Tests: presenter `typed_paths`,
one search-results case in `find_feed_tests` and the platform cases in the presenter's
`windows`/`unix` tests and `name_case_windows`; the engine's `not_a_folder` unit and
`navigation_not_a_folder` integration tests, the latter also through the installed
native package (opt-in, `MIDNIGHT_NATIVE_TEST_INSTALLATION`; CI's `engine-native-ignored`
step), and the native host's
`a_listing_of_a_node_that_is_no_directory_is_not_a_directory`; the UI integration modules
`path_bar_file` and `path_bar_file_windows`, and `tooltips`.

Clipboard text is read by the bounded backend clipboard worker, with a 4 MiB
encoded-payload resource policy. This is not a path-length restriction. Invalid
encoding, NUL or line breaks produce an explicit error with the editor unchanged;
single-line input must not silently replace a native filename's line break.
Native file-list clipboard data is not guessed into a text path.

Each reply belongs to the captured pane, location generation, editor lifetime,
text, selection and composition. Typing, selection changes, navigation, switching
editors, Escape, a newer paste or host shutdown invalidates the old reply.
Queue refusal reports a visible error. Stale payloads are reclaimed through the
backend retirement route.

The path-only widget uses native TextInput editing and owns its Paste menu route;
other LineEdits are unchanged. A small adapter uses the pinned Slint committed
text event on the exact captured input, after releasing Presenter/controller
borrows. It keeps native selection, composition and undo/redo semantics. Assigning
the text property after Paste would clear undo, while synthetic key presses would
reject private-use Unicode; neither is used for insertion.

Windows reads bounded CF_UNICODETEXT and rejects unpaired UTF-16 rather than
creating a replacement-character path. Linux reuses bounded X11/XWayland selection
transfers or the Wayland MIME/pipe reader. Linux clipboard ownership is
initialized lazily so text reads do not require an unrelated arboard owner first.
The pinned Slint Winit clipboard also uses arboard data-control or X11/XWayland.
Native Wayland requires the compositor's data-control protocol; missing capabilities are
reported explicitly. Known limit: the tests do not cover every native compositor.

Linux middle-click primary-selection editing keeps Slint's own behavior;
the explicit path-paste operation covers keyboard and context-menu Paste.

Focused tests: `path_paste_ui` (including full Windows, UNC and Linux paths with and
without a final separator), the engine's `clipboard_text` and clipboard read tests, UI
presenter path-paste tests and separate native clipboard text/transport tests. Evidence:
[path paste record](../verification/2026-11-07-path-paste.md).

## Opening the editor and Go

A click anywhere in the blank part of the path bar after the path components, up to
the bar's right end, opens the editor with the shown path; Ctrl+G does the same from
the keyboard. Gaining focus selects the whole path (`input.select-all()` in
`path-editor.slint`), as a browser's address bar does, so what is typed or pasted
replaces it. Clicking a component still opens that folder, and a right click still
copies its path. The open editor ends in a Go button: a click on it, or Enter or Space
while it has keyboard focus (Tab moves there from the editor), does exactly what Enter
in the editor does, with the same file-path handling and the same errors. Escape
cancels from either. Tests: the UI integration modules `path_bar_edit` and
`path_bar_fit`, and `tooltips`.
