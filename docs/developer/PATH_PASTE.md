# Pasting folder paths

The top panel path editor treats keyboard Paste (Ctrl+V or Shift+Insert) and its
context-menu Paste as the same explicit action. Only the pasted text receives a
native trailing separator: Windows accepts an existing `/` or `\` and otherwise
adds `\`; Linux accepts/adds `/`. An empty paste is unchanged. Existing text after
the selection stays in place. Ordinary typing is not modified.

Spaces, quotes, combining characters and other Unicode are literal. Paste does
not trim, unquote, normalize, expand variables, interpret URIs or open a folder.
Enter remains the navigation action. Breadcrumb/provider identities and unchanged
raw native paths retain their existing navigation routes.

Enter takes edited text without its surrounding whitespace and without one pair of
surrounding double quotes, which Windows "Copy as path" adds; a separator that Paste
added after the closing quote goes with them. The unchanged location text keeps its
exact identity. A native path that names a file (or a link to one, a special file or
an archive file) opens the file's folder with the cursor on the file. The listing
worker that failed to list the path asks the provider what it names
(`browse_directory_identity`, following links) and sends `Event::ListingNotAFolder`
before the failure; only the path editor's request, while still pending, then lists
the folder with the file as its focus hint. Other navigations keep their failure and
recovery, and a missing path its error. A final separator does not make a file's
path a folder's, as Paste ends every path with one. A file that the panel's own
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
`navigation_not_a_folder` integration tests; the UI integration modules
`path_bar_file` and `path_bar_file_windows`, and `tooltips`.

Clipboard text is read by the existing bounded backend worker, with a 4 MiB
encoded-payload resource policy. This is not a path-length restriction. Invalid
encoding, NUL or line breaks produce an explicit error with the editor unchanged;
single-line input must not silently replace a native filename's line break.
Native file-list clipboard data is not guessed into a text path.

Each reply belongs to the captured pane, location generation, editor lifetime,
text, selection and composition. Typing, selection changes, navigation, switching
editors, Escape, a newer paste or host shutdown invalidates the old reply.
Queue refusal reports a visible error. Stale payloads are reclaimed through the
existing backend retirement route.

The path-only widget uses native TextInput editing and owns its Paste menu route;
other LineEdits are unchanged. A small adapter uses the pinned Slint committed
text event on the exact captured input, after releasing Presenter/controller
borrows. It keeps native selection, composition and undo/redo semantics. Assigning
the text property after Paste would clear undo, while synthetic key presses would
reject private-use Unicode; neither is used for insertion.

Windows reads bounded CF_UNICODETEXT and rejects unpaired UTF-16 rather than
creating a replacement-character path. Linux reuses bounded X11/XWayland selection
transfers or the existing Wayland MIME/pipe reader. Linux clipboard ownership is
initialized lazily so text reads do not require an unrelated arboard owner first.
The pinned Slint Winit clipboard also uses arboard data-control or X11/XWayland;
there is no additional seat-specific clipboard capability being removed here.
Native Wayland still requires the compositor's supported data-control protocol;
missing capabilities are reported explicitly. Software/transport-selection tests
do not constitute qualification on every native compositor.

Linux middle-click primary-selection editing retains Slint's existing behavior;
the explicit path-paste operation covers keyboard and context-menu Paste.

Focused tests: `path_paste_ui`, UI presenter path-paste tests, engine clipboard
tests and separate native clipboard text/transport tests. Investigation and causal
RED/GREEN evidence are retained under `_build/goal/path-paste/`.
