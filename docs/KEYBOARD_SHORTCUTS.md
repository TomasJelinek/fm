# Keyboard shortcuts — Midnight Explorer 0.1

Planned addition: **Ctrl+F8 — calculate checksums** for selected files (focused
file fallback), described in the [checksum design](superpowers/specs/2026-11-06-checksums-design.md).
This command is not implemented or bound in the current application. Ctrl+F7
continues to open filename case conversion. Future default registration must
preserve user-customized bindings.

The table lists default bindings. **Options → Keyboard** can change, disable, or
restore each application action, including navigation, selection, command editing,
drive letters, numbered tabs/Hot Paths, and the drive dropdown's As Other Panel action.
Search commands by name or command ID, select a command, then focus **Press shortcut
keys** and press a combination. **Assign** adds it while preserving the command's
other aliases; choose an existing alias and use **Remove** to remove just that one.
An action accepts up to four alternate single chords. The editor displays the
effective context and current owners before assignment; overlapping conflicts are
rejected before saving. **Restore command** restores one action; **Restore all
defaults** asks before resetting the complete map. Tab or Escape records that key
and leaves the capture field without running an application command.
Changes are saved with the other preferences and survive restart. The action catalog
also supplies the application's displayed shortcut hints and Help.

Native widget editing and IME shortcuts remain native; operating-system-reserved
keys may not reach the application. Ctrl+Space stays available to the IME, and broad
application shortcuts cannot take over ordinary typing or editor copy/paste.
While filename search is active, Ctrl/Meta+C, X, V and the Insert copy/paste aliases
edit search text even if reassigned to file actions. Those bindings apply to files
after leaving search. Ctrl+A and Shift+Delete retain their listed file actions.
Physical number keys may be entered as `Ctrl+1` or `Ctrl+Digit1`; these are the same
binding. The editor accepts `Ctrl`, `Alt`, `Shift`, `Meta`, named keys such as
`PageDown`, and Unicode characters. `Plus` denotes the plus key.

Holding Ctrl switches the footer to the effective Ctrl+F1–F12 bindings. Reassigning
or disabling a binding updates its caption, tooltip and click action. A mouse press
captures the displayed action and pane even if Ctrl is released before the click
finishes. AltGr does not activate this layer. While a panel is enlarged, switching
panels transfers the enlargement; centering the splitter exits enlargement.

File navigation, tab and bookmark shortcuts apply
in the focused file list, preserving typing in the path/command/options editors.
Native numbered tabs and hot paths use physical digit keys across keyboard layouts
and do not intercept RightAlt/AltGr. Number 0 refers to bookmark slot 10.
Bare digits remain text while a path, command, search or options editor has focus.
An unassigned Hot Path number is consumed without navigating or starting search.
Existing custom digit bindings take precedence over the new default Hot Path
digits; change or disable them in Options → Keyboard to free those numbers.

| Keys | Current action |
| --- | --- |
| Tab / Shift+Tab in files | Switch the active pane |
| ↑ / ↓, PageUp / PageDown, Home / End | Move the focused row |
| ← / → in Brief | Move to the previous / next file column |
| ← / → and ↑ / ↓ in Thumbnails | Move between cells and grid rows |
| Plain typing in files | Start Unicode case-insensitive filename-prefix search |
| ↑ / ↓ during search | Visit previous / next matching name without wrapping |
| Escape during search | Leave search and restore the complete listing |
| Backspace during search | Remove one extended grapheme; an empty query exits |
| Enter | Enter a folder or open the focused file through its host association |
| F3 / Alt+F3 in files | Open the applicable installed viewer / force basic Text mode |
| F4 in files | Open the focused regular local file with the configured external editor |
| Backspace / Ctrl+PageUp | Parent folder |
| Alt+← / Alt+→ | Back / forward history |
| Ctrl+Backspace / Shift+Backspace / Ctrl+Backslash | Native filesystem root |
| Ctrl+PageDown | Enter a known folder; never launch a regular file |
| Ctrl+G | Focus the path editor |
| Alt+F1 / Alt+F2 | Open left / right volume choices |
| . in the drive dropdown | As Other Panel: open the other panel's current folder |
| Shift+drive letter | Switch volume on Windows, using its remembered path |
| Ctrl+F1 / Ctrl+L | Read-only cached drive information |
| Ctrl+F2 | Attributes, times and filename-case editor for the captured selection |
| Space | Toggle selection, advance one row and calculate the focused directory's size on the backend |
| Insert | Toggle selection and move to the next row |
| Ctrl+A / Ctrl+D / Ctrl+I | Select all / clear / invert selection in the focused file pane |
| Ctrl+C / Ctrl+V in files | Publish the selected/focused files to the native clipboard / queue a verified copy into the captured active folder |
| F5 | Copy the captured selection or focused item; edit the destination folder, initially the other panel. Enter queues the copy; Escape cancels the dialog |
| Ctrl+R / Ctrl+F9 | Refresh |
| Ctrl+F3 / Ctrl+F4 / Ctrl+F5 / Ctrl+F6 | Sort by name / type-extension / modified time / size |
| Ctrl+F7 | Open the same metadata editor with filename case conversion enabled |
| Ctrl+F8 | Unassigned by default; may be configured |
| Ctrl+F10 | Compare the two displayed directories by size and/or modified time; select differences |
| Ctrl+F11 | Maximize the active panel; press again to restore the previous split |
| Ctrl+F12 | Configure the active panel's saved wildcard or regular-expression file filter |
| Ctrl+Shift+S / Ctrl+Shift+F10 | Start or cancel all-folder size calculation in the active pane |
| Alt+Enter / Ctrl+Alt+Enter | Properties of selected items, or focused item |
| Ctrl+T | New tab at the current folder |
| Ctrl+W | Close active tab; keep the last tab |
| Ctrl+Tab in files or path editor | Focus the bottom command editor |
| Ctrl+Shift+Tab in files | Previous tab in that pane |
| Ctrl+U in files | Swap the complete visual panels, preserving their tabs and running work |
| Ctrl+Shift+1…9 | Switch to that numbered tab in the focused pane; missing tabs do nothing |
| Ctrl+Shift+0 | Assign current folder to bookmark slot 10; other slots use the Hot Paths manager |
| 1…9/0, Ctrl+1…9/0 or Shift+1…9/0 | Open the numbered Hot Path in the focused pane |
| Ctrl+Alt+number or Shift+Alt+number | Open the numbered bookmark in the other pane |
| Ctrl+Shift+Plus | Add the current folder once, using the first free numbered slot |
| Ctrl+Enter in files or command editor | Append the focused basename to the end of the command draft and focus the command editor |
| Ctrl+P in files or command editor | Insert the active pane path at the command selection |
| Ctrl+[ / Ctrl+] | Insert the left / right pane path at the command selection |
| Ctrl+Space | Available to the input method (IME); Ctrl+P inserts the path |
| Enter in command editor | Explicitly submit the command to the selected system terminal |
| Alt+↑ / Alt+↓ in command editor | Rotate to older / newer stored commands, wrapping; terminal type stays unchanged |
| Escape / Ctrl+Tab in command editor | Return to the active file list |
| F1 | Help |
| F2 | Rename the focused file/folder; Enter confirms, Escape cancels |
| Shift+F4 in files | New empty file in the captured panel folder; Enter confirms the name, Escape cancels; existing entries are never overwritten |
| F7 | Create an absolute or relative folder path, including `..` and missing intermediate folders |
| Delete / F8 | Confirm moving the selection (or focused entry) to the system Trash |
| Shift+Delete / Shift+F8 | Explicitly confirm permanent deletion |
| F11 / F12 | Connect / disconnect network drive through Windows system dialogs |
| F9 | User menu; no custom command execution is configured in this milestone |
| F10 | Main menu, including Options |
| Alt+F4 | Exit |

Inserted paths/names use the draft's fixed shell grammar: simple paths remain
bare; special characters use escaped double quotes, never surrounding apostrophes. Insertion
never submits a command. Editor word deletion and ordinary text editing retain
their standard behavior. Menus also support arrows, Enter and Escape; unclear
controls have tooltips and accessible names.

Mouse: left click focuses, Ctrl+left click toggles selection, Shift+left click
selects an inclusive range, and Ctrl+Shift+left click adds that range. An unmodified
left double-click activates. Clicking a column header switches sorting direction.
Insert and Space clamp at the last row; `..` is never selected or measured.
Wheel scrolling animates in pixels and settles on a full row when idle; native
touch/trackpad gestures retain their own inertia.

Drag a file/folder with the left mouse button to copy it after a short movement
threshold. A selected item supplies the selected group; an unselected item supplies
only itself. Touch scrolling does not start a file drag. Escape cancels the drag;
releasing before asynchronous preparation finishes cannot start a delayed drag.
Drop on the highlighted file-list area to copy into that panel's displayed folder.
A folder row is not a separate destination in this stage. Editors, toolbars, dialogs
and the hidden panel's zoom rail do not receive file drops.

Native dragging exchanges files with other applications on Windows and
X11/XWayland. Native Wayland currently supports dragging between panels only.
Transfers are Copy-only; use Ctrl/Copy in an external source application if it
otherwise offers only Move. Legacy X11 sources without XDND proxy support cannot
drop into the application. Incoming drops share the verified copy queue and its
conflict prompts. An external destination application performs outgoing copies
under its own rules; Midnight Explorer never deletes the source after a drag.

Right-click a file/folder or drive to open its Windows system context menu; Linux
offers the available application actions. A selected group keeps its selection;
an unselected item targets just that item. Pasting on a sole folder targets that
folder. Native-menu Copy/Paste uses the same verified copy service as Ctrl+C/V.

The top action toolbar includes a sort chooser beside View, available in every mode.
The Hot Paths star adds the current folder; its arrow lists saved paths, then
Add path and Manage below a separator. Saved paths also appear in their own group
in the drive dropdown after drives and special locations. The drive bar shows
numbered Hot Paths as 1–9/0. Adding an existing path keeps its saved identity;
new paths take the first free number, and paths beyond ten remain in the menus.
Manage shows full folder paths without custom names. Selecting a row keeps the
manager open; choose a number and Apply shortcut to update that saved path.
Moving between occupied numbers swaps them without removing either folder.
The drive dropdown's As Other Panel item
also works by mouse and uses the current
opposite folder after Ctrl+U swaps the panels. Back/Forward arrows have history
dropdowns. Tabs sit below the drive
toolbar. Hovering a path segment highlights its prefix; clicking opens that raw
ancestor; right-click copies that prefix without navigating. Click blank space
after the path to edit it; Enter navigates and Escape cancels. Middle-click a tab
to close it (the last tab remains). Tabs and their active position survive restart.
Explicit `--left` / `--right` locations override restoration for that pane. Drag the central
splitter to resize the panels, or double-click it to center. Tabs shrink uniformly
and then expose scroll arrows at their minimum width. The details strip below each
panel copies a field's displayed value
when clicked (filename, exact bytes, date/time, attributes or Unix permissions).
File-only selections show their total known byte size using the system locale;
pending/unknown sizes are identified. Available capacity appears at the right and
comes from the displayed directory's filesystem query, including mount/junction targets.

The terminal icon opens a new interactive terminal without submitting the draft;
its adjacent arrow selects the shell. The command directory ends with `>`.
Command history recalls a draft without
running it; only the configured Execute command shortcut or Run submits it.
Options has Panel, Appearance, Command and Keyboard
topics, including readable/exact-byte sizes, history persistence and Clear.
History keeps the latest 100 distinct exact command strings across shells,
newest first, with a 1MiB aggregate text budget. Elevation is never recalled.

Search matches the beginning of files and folders literally, always ignoring
Unicode case. The matched original prefix is colored; `..` never matches. The
Options checkbox **Show only filename search matches** hides nonmatching entries; otherwise
search jumps among matches in the complete list. Home/End and PageUp/PageDown
also follow match order. Space appends query text while searching; Insert exits
search before its normal selection action. Native IME composition is not searched
until committed. Shift+drive keeps its Windows meaning; shifted numbered
bookmarks are suspended during search/composition. Queries are independent per
pane, survive sorting/refresh and clear on folder/tab navigation. Pending or
empty-match searches cannot activate a leftover nonmatching keyboard cursor.

F5 Copy opens the destination dialog without changing the clipboard. The selection
and other-panel destination are captured at the gesture; later panel changes do
not retarget it. Edited destinations must be absolute paths to existing folders.
F6 Move remains reserved for the next transfer increment.
Options follows F12 on the bottom bar; Jobs shows queued/running operations and
allows cancellation. Cancellation retains changes already completed. No Trash
failure ever falls back to permanent deletion. F7 accepts paths outside the active
pane, refuses intermediate links and leaves already-created intermediate folders
on error. Relative paths start in the captured panel, never the process directory.
F3 routes by extension to an applicable installed viewer; Alt+F3 forces the basic
plain-text viewer, preserving Unicode and disabling highlighting and markup rendering.
Both require a compatible, enabled package in the installation's `viewers/` directory.
Clipboard copy has a background queue and mandatory SHA-256 destination readback.
Existing names offer Skip, Skip all, Copy as, Cancel job and Decide later; Jobs can
reopen deferred conflicts while browsing continues. Sources are retained. Replacing
existing files, directory merging and Move/Cut remain unavailable in this stage.
Other unlisted Salamander/Total Commander
shortcuts, such as Shift+F9 for a bookmark menu, are not implemented.

Viewer windows have their own configurable shortcut context:

| Keys | Viewer action |
| --- | --- |
| Escape | Close the viewer, including when search has focus |
| Space in content | Next regular file in captured panel order, wrapping at the end |
| Ctrl+F | Focus search |
| Enter in search / F3 | Next match |
| Shift+F3 | Previous match |
| Ctrl+A in content | Select the loaded page (bounded copy) |
| Ctrl+C in content | Copy selected displayed text |
| Arrows / PageUp / PageDown | Scroll content |
| Ctrl+Home / Ctrl+End | First / last page |

Search retains ordinary text editing, clipboard keys and spaces. Main-window actions
do not dispatch inside viewer windows. Viewer mode is selected using the bottom
dropdown; Markdown search/copy uses rendered text, while Text mode uses literal source.

Bulk selection runs through the backend and excludes `..`. Text editors retain
their ordinary Ctrl+A behavior. The requested Ctrl+D takes precedence over
Total Commander's bookmark meaning. During progressive listing, all/invert may
wait for that listing to complete; input remains responsive.

Behavioral references: local Salamander `src/salamand.rc:61–102` and
`src/fileswn0.cpp:1220–1260`. Requested Ctrl+P/Ctrl+Enter behavior and folder-tab
navigation take precedence over an upstream shortcut with a different meaning.
