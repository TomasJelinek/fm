# Keyboard shortcuts — Midnight Explorer 0.3

File dragging follows Explorer-style internal defaults: same-volume Move,
otherwise Copy, with uncertain or mixed volume evidence falling back to Copy.
**Ctrl + drag** forces Copy and **Shift + drag** forces Move. **Escape** cancels
the gesture. Ctrl+Shift/Alt link gestures are unavailable and do not silently
perform a different action. These rules also apply to captured native folders in
expanded tree branches; see the [drag design](superpowers/specs/2026-11-10-folder-drag-and-native-handoff.md).

**Ctrl+F8 — checksums** opens the Checksums window for the selected files (or the
focused file or folder; on the parent row with nothing selected, the files of the
shown folder), described in the [checksum API](developer/CHECKSUMS.md). Nothing is
calculated until you press **Calculate**; the algorithm dropdown has a checkbox per
algorithm, with only SHA-256 checked in a new window, and an *Include subfolders*
checkbox. If the selection contains a checksum
file, the window opens on **Verify** with that file chosen. Ctrl+F7 continues to open
filename case conversion. Default registration preserves user-customized bindings.
These controls describe current source; packaged releases may precede it.
The same window saves and verifies SFV/CRC32, MD5, SHA-1, SHA-256 and SHA-512
files. **Save manifest…** asks for a file and saves at once; partial results and an
existing file are confirmed in the window first (**Save** / **Replace**).
On the **Verify** page, **Browse** and then **Verify** read a manifest on background workers. Empty
panels can also open it.
Inside the checksum results list, **Ctrl+C** copies the chosen hash and
**Ctrl+Shift+C** copies the full row. **Shift+F10** opens the per-algorithm copy
menu; **Escape** closes that modeless window and requests cancellation of its work.
The [current follow-ups](superpowers/specs/2026-11-07-panel-viewer-followups.md)
record pending changes separately: archive Hot Paths, Find modes/row layout,
detailed drive information, Markdown presets and source-viewer position controls.
The old source-recovery Move workflow is being removed, not offered as a second
user-selectable mode.

The table lists default bindings. **Options → Keyboard** can change, disable, or
restore each application action, including navigation, selection, command editing,
drive letters, numbered tabs/Hot Paths, and the drive dropdown's As Other Panel action.
Search commands by name or command ID, select a command, then focus **Press shortcut
keys** and press a combination. **Assign** adds it while preserving the command's
other aliases; choose an existing alias and use **Remove** to remove just that one.
An action accepts up to four alternate single chords. The editor displays the
effective context and current owners before assignment. An overlapping assignment
opens a **Replace shortcut** dialog listing the affected commands. **Replace**
removes that chord from those commands and assigns it to the selected command;
other aliases and nonoverlapping contexts remain intact. **Cancel**, Escape or
the default Enter response keep the existing assignments. Invalid or reserved
shortcuts remain unavailable. **Restore command** restores one action; **Restore all
defaults** asks before resetting the complete map. Tab or Escape records that key
and leaves the capture field without running an application command.
Changes preview immediately; **OK** saves them with the other preferences and
closes Options. **Cancel**, X or Escape outside capture discard the entire Options
session, restoring previous mappings and previewed appearance. An open inline
editor takes Escape first: the custom archive extension field and a new argument
row cancel only their own edit, an open color picker closes, and the next Escape
closes Options. The action catalog also supplies the application's displayed
shortcut hints and Help.

Native widget editing and IME shortcuts remain native; operating-system-reserved
keys may not reach the application. Ctrl+Space stays available to the IME, and broad
application shortcuts cannot take over ordinary typing or editor copy/paste.
While filename search is active, Ctrl/Meta+C, X, V and the Insert copy/paste aliases
edit search text even if reassigned to file actions. Those bindings apply to files
after leaving search. Ctrl+A and Shift+Delete retain their listed file actions.
Physical number keys may be entered as `Ctrl+1` or `Ctrl+Digit1`; these are the same
binding. The editor accepts `Ctrl`, `Alt`, `Shift`, `Meta`, named keys such as
`PageDown`, and Unicode characters. `Plus` denotes the plus key.

The F1–F12 footer follows the held Ctrl, Alt, Shift and Meta modifiers, including
combinations. Every cell uses the effective binding from Options → Keyboard;
unassigned cells are disabled. Changing, removing or restoring a binding updates
its caption, tooltip and click action, including unmodified function keys. AltGr
is typing input and does not activate a modified layer. Operating-system-reserved
combinations may be intercepted before they reach the app.

Connect/Disconnect buttons are hidden when the operating system has no network
drive dialog service, including on Linux. The remaining buttons use the available
space. This follows the effective action after remapping; supported Ctrl+F11 Zoom
and Ctrl+F12 Filter actions remain visible.

At normal widths the action text takes priority over its icon: a caption first
gives up its icon, then shrinks to a readable minimum. A cell that still cannot
show the whole caption, and every very narrow cell, shows the function key alone,
never a shortened caption; the tooltip and accessibility label retain the complete
chord and action. A mouse press captures the action and pane, so releasing or
changing modifiers before releasing the mouse cannot trigger a different action.
While a panel is enlarged, switching panels transfers the enlargement; centering
the splitter exits enlargement.

| Function key | Unmodified | Ctrl | Shift | Alt | Ctrl+Shift |
| --- | --- | --- | --- | --- | --- |
| F1 | Help | Drive info | — | Left drive | — |
| F2 | Rename | Attributes | — | Right drive | — |
| F3 | View | Sort by name | — | Plain-text view | — |
| F4 | Edit | Sort by extension | New file | Exit | — |
| F5 | Copy | Sort by modified time | — | — | — |
| F6 | Move | Sort by size | — | — | — |
| F7 | Create directory | Filename case | — | Find | — |
| F8 | Trash | Checksums | Permanent delete | — | — |
| F9 | User menu | Refresh | — | — | — |
| F10 | Main menu | Compare directories | — | — | Directory sizes |
| F11 | Connect | Zoom panel | — | — | — |
| F12 | Disconnect | Panel filter | — | — | — |

This matrix shows main file-pane defaults, including filename search; viewers and
dialogs have their own local shortcuts. Other modifier combinations have no default
main-window function-key bindings; users can assign them if the OS makes them available.

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
| Plain typing in files | Start Unicode case-insensitive filename-prefix search; the number row's + − * are typed too (Ctrl+= / Ctrl+− and the numeric keypad's + − * select by mask) |
| ↑ / ↓ during search | Visit previous / next matching name without wrapping |
| Escape during search | Leave search and restore the complete listing |
| Backspace during search | Remove one extended grapheme; an empty query exits |
| Enter | Enter a folder or open the focused file through its host association |
| F3 / Alt+F3 in files | Open the applicable installed viewer / force basic Text mode. Also works for root files while inline folder branches are expanded; Space / Alt+Right advances through the captured visible file order; Backspace / Alt+Left reverses it. |
| F4 in files | Open the focused regular local file with the configured external editor |
| Ctrl+M in files | Make File List: captured selection to clipboard, viewer or UTF-8 file |
| Backspace / Ctrl+PageUp | Parent folder |
| Alt+← / Alt+→ | Back / forward history |
| Alt+↑ / Alt+↓ in files | Previous / next selected file or folder in displayed order, including expanded tree rows. Skip the current row, preserve all marks, and stop at the boundary without wrapping; inactive during filename search. Configurable as `panel.previous-selected` / `panel.next-selected`. |
| Shift+Backspace / Ctrl+Backspace / Ctrl+Backslash | Root of the focused panel's location: the filesystem root (`/` on Linux, the drive's root on Windows), the archive's root inside an archive, the share's root in a network panel |
| Ctrl+PageDown | Enter a known folder; never launch a regular file |
| Ctrl+G | Focus the path editor |
| Alt+F1 / Alt+F2 | Open left / right volume choices |
| . in the drive dropdown | As Other Panel: open the other panel's current folder |
| Shift+drive letter | Switch volume on Windows, using its remembered path |
| Ctrl+F1 / Ctrl+L | Read-only cached drive information; detailed backend-provided volume information is the requested replacement |
| Ctrl+F2 | Attributes, times and filename-case editor for the captured selection |
| Alt+2 | Switch the active pane to Brief view |
| Alt+3 | Switch the active pane to Detailed list view |
| Alt+5 | Switch the active pane to Thumbnails view |
| Space | Toggle selection, advance one row and calculate the focused directory's size on the backend |
| Insert | Toggle selection and move to the next row |
| Ctrl+A | Select displayed top-level files only; deselect folders and nested rows |
| Ctrl+Shift+A | Select displayed top-level files and folders; exclude `..` and nested rows |
| Ctrl+D | Clear all selection, including folders and expanded descendants, in the focused pane |
| Ctrl+I | Invert file selection in the focused pane; preserve selected and unselected folders, including directory links |
| Ctrl+Shift+I | Invert top-level files and folders; preserve nested tree marks |
| Ctrl+= or Num + / Ctrl+− or Num − in files | Select / Deselect files by mask (Ctrl+= and Ctrl+− need no numeric keypad): a small dialog with the mask (the last one, `*` at first) and Include folders. Configurable as `selection.mask-select` / `selection.mask-deselect`; see [selection by mask](#hidden-files-and-selection-by-mask) |
| Num * in files | Invert files only, or top-level files and folders when Select by mask last had Include folders checked; without a keypad, Ctrl+I and Ctrl+Shift+I invert the same two ways. Configurable as `selection.mask-invert` |
| Ctrl+H | Show or hide hidden files in both panels (`panel.hidden-files`); also in F10, each panel's View menu and Options → General |
| Ctrl+C / Ctrl+V in files | Publish selected/focused files to the native clipboard / paste into the captured active folder; a Cut clipboard opens Move confirmation |
| Ctrl+X in files | Publish a Cut clipboard for captured native files and folders. Paste opens the same Move confirmation as F6 (at most 64 selected items; on Linux a link moves as a link); archive Move is unavailable |
| F5 | Copy the captured selection or focused item, including selected nested native/archive folders in expanded trees; edit the destination folder, initially the other panel. Enter queues the copy; Escape cancels the dialog |
| F6 | Move captured native files or folders, including selected nested directories in expanded native trees, to an editable destination initially set to the other panel. Same-volume native rename reads no content; copied files are verified before source removal. On Linux a link moves as a link. Archive Move is unavailable |
| Ctrl+F / Alt+F7 | Open a separate Find window for the active native folder: masks, multiple roots, subfolders and optional UTF-8 content text or per-line regular expression, optionally inside archives (every archive and member, however large, until the search ends or Stop). Feed to panel shows the results as a list in the panel: the panel keys (F3, F4, F5, F6, F8, Shift+F8, Insert, Space, Enter) act on the results, Backspace returns to the folder. Up/Down selects results; Enter or Go to file validates the result and locates it in its originating panel; F1 shows or hides syntax help for the chosen match mode and moves the keyboard into it (Up/Down and Page Up/Page Down scroll it); Escape first closes the help or the Add folder entry, and otherwise closes that Find window. In a viewer, Ctrl+F searches the opened file instead. |
| Ctrl+R / Ctrl+F9 | Refresh |
| Ctrl+F3 / Ctrl+F4 / Ctrl+F5 / Ctrl+F6 | Sort the active panel by name / extension / modified time / size; repeat to reverse the order. Each detailed panel also has a clickable Ext header next to Name, with the current sort arrow; repeat-click reverses the order. The drive-bar sort dropdown offers Extension in every view mode. |
| Ctrl+F7 | Open the same metadata editor with filename case conversion enabled |
| Ctrl+F8 | Open modeless Checksums: Calculate (SHA-256 default; optional SHA-512/MD5/SHA-1/CRC32 and recursion in the algorithm dropdown), Stop, copy, Save manifest… (confirms replacing an existing file) and Verify (`.sfv`, `.md5`, `.sha1`, `.sha256`, `.sha512`) |
| Ctrl+F10 | Compare the two displayed native directories by size and/or modified time, optionally by contents and with subfolders; select differences; also available as Compare directories in the shared toolbar. While a comparison runs, Ctrl+F10 shows its progress with Cancel |
| Ctrl+Shift+C | Compare two selected files in the active panel; otherwise use its single selected/focused file and the opposite explicit selection, or its unique visible same-name file when nothing is selected there. If the pair cannot be determined, choose File A and File B using paths or Browse; Enter compares, Escape cancels. Files inside an open archive can be compared directly with other members or with files on disk. Pairs containing an archive member open in the built-in comparator. Image pairs support Side-by-Side, Swipe and Onion Skin. Binary files, text over the text limits (32 MiB, 1,000,000 lines or a line over 16 KiB) and other pairs are compared as bytes in a hex view; the Hex button shows the bytes of a text pair. Compare files in the shared toolbar also works without a panel target. |
| Ctrl+F11 | Fill the entire panels area with the active panel, hiding the other panel and splitter; press again to restore the previous split |
| Ctrl+F12 | Configure the active panel's saved Masks, Regex or typed metadata Expression filter; also available on each drive bar immediately left of View. OK applies the filter, Cancel discards the draft; Show all or the panel indicator × disables it without discarding its expression |
| Alt+↓ in Panel filter | Open the list of recent filters, also available from the down arrow beside the pattern. Both panels share this list across sessions. Select a filter to recall its pattern, mode and case setting. |
| Ctrl+Shift+S / Ctrl+Shift+F10 | Start or cancel folder size calculation in the active pane: selected folders when selection exists, otherwise all displayed folders including descendants in expanded branches. Native/archive listings and inline branches are supported. |
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
| Ctrl+Shift+Plus or Ctrl+Shift+Equal | Add the current folder once, using the first free numbered slot |
| Ctrl+Enter in files or command editor | Append the focused basename to the end of the command draft and focus the command editor |
| Ctrl+Shift+P in files or command editor | Append the focused native item's complete path, including filename, and focus the command editor; no added quotes or spaces |
| Ctrl+P in files or command editor | Insert the active pane path, ending in a native path separator, at the command selection |
| Ctrl+[ / Ctrl+] (also with Shift) | Insert the left / right pane path, ending in a native path separator, at the command selection |
| Ctrl+Space | Available to the input method (IME); Ctrl+P inserts the path |
| Enter in command editor | Explicitly submit the command to the selected system terminal |
| Alt+↑ / Alt+↓ in command editor | Rotate to older / newer stored commands, wrapping; terminal type stays unchanged |
| Escape / Ctrl+Tab in command editor | Return to the active file list |
| F1 | Help |
| F2 | Rename the focused native file/folder, including an expanded folder occurrence; Enter confirms, Escape cancels. Marked selection does not change the target; collisions require overwrite consent. While the rename runs and until the new name is listed (at most 3 s after it completes) the panel holds input and then replays the keys typed meanwhile; Escape releases it at once |
| Shift+F4 in files | New empty file in the captured panel folder; Enter confirms the name, Escape cancels. A listed file's name is asked about first; only the Replace file button (not another Enter) replaces it, keeping the original in a recovery folder |
| F7 | Create an absolute or relative folder path, including `..` and missing intermediate folders |
| Delete / F8 | Confirm moving the selection (or focused entry) to the system Trash |
| Shift+Delete / Shift+F8 | Explicitly confirm permanent deletion |
| F11 / F12 | Connect / disconnect network drive through Windows system dialogs; unavailable on Linux, where these footer buttons are hidden |
| F9 | User menu: Command line, Bookmarks and Hot Paths, Properties, folder sizes and Checksums, then each configured Custom Command, which runs on the active panel as from its toolbar button (a disabled one is listed but cannot run). Without Custom Commands the menu says so, and Set up opens Options → Custom Commands |
| F10 | All important actions grouped into Files, Selection, Panels and navigation, Tools, Application; includes Options and Jobs |
| Alt+F4 | Exit |

Bracket shortcuts use the physical US `[` / `]` key positions, including on
keyboard layouts with different labels. They work from the file list and command
editor; AltGr remains available for typing. Options → Keyboard can reassign or
disable these actions, and their command-editor hints follow the saved mapping.
Outside the number row and these bracket keys, letter shortcuts such as Ctrl+C in
the checksum results and Ctrl+V in the path editor follow the character the keyboard
layout produces, as in other text fields, so on layouts without Latin letters (for
example Russian or Greek) paste a path with Shift+Insert or the context menu and copy
a checksum with Shift+F10 or the context menu.

Inserted paths/names are raw text: no generated quotes, escaping or separator
spaces. Directory insertions append a native path separator only when one is
missing; filename insertion remains unchanged. Actual spaces and quotes in the
name remain unchanged. Insertion never
submits a command; the draft is shell syntax only on explicit Enter/Run. Names
with control characters or unrepresentable native bytes fail without a lossy
replacement. Editor word deletion and ordinary text editing retain
their standard behavior. The file context menu and drop-down choosers support
arrows, Enter and Escape. In the F9 User menu and the F10 Main menu, Tab and
Shift+Tab move between items, Enter or Space activates the focused item and
Escape closes the menu; Up/Down in the Main menu's category chooser change the
category. Unclear controls have tooltips and accessible names.

Mouse: left click focuses, Ctrl+left click toggles selection, Shift+left click
selects an inclusive range, and Ctrl+Shift+left click adds that range. An unmodified
left double-click activates. Clicking a column header switches sorting direction.

**Ctrl+click on a folder expander** starts bounded recursive expansion inline,
including directory symlink/junction expanders. Collapse/navigation cancels the
asynchronous walk; cycles and other failures stay local to the affected branch.
Known-empty directory links show no `+`. Expansion can pause at its capacity
limit while preserving the valid tree. Ordinary row Ctrl+click keeps the selection
behavior above. See [tree behavior and limits](developer/TREE_PANEL.md#retry-diagnostics-and-recursive-expansion)
for details.

Insert and Space clamp at the last row; `..` is never selected or measured.
In expanded trees, Space measures its focused directory independently of other
marks. Bulk size calculation uses selected directories, including explicitly
marked descendants; without marks it measures every eligible displayed directory,
including rows in expanded branches outside the scroll viewport. A
files-only selection starts no directory traversal. Accepted scans continue while
selection, sorting or filtering changes; progress never automatically resorts rows.
Wheel scrolling animates in pixels and settles on a full row when idle. A
touchpad flick keeps scrolling with momentum that slows down naturally and stops
at the list's end; any click, key or new wheel/touchpad movement stops it at once.
On Windows and macOS the system's own touchpad momentum is used instead.

Drag a file/folder with the left mouse button after a short movement
threshold. A selected item supplies the selected group; an unselected item supplies
only itself. Touch scrolling does not start a file drag. Escape cancels the drag;
releasing before asynchronous preparation finishes cannot start a delayed drag.
A release less than 0.4 s after the mouse button was pressed is taken for an
accident and cancels the drag wherever it ends: nothing is copied or moved and no
dialog opens, the outline and drag label go away, selection and cursor stay, and
for a few seconds the source panel's status line starts with "Drag cancelled:
released within 0.4 s". The time counts from the press, so holding the button
before moving counts too.
Drop on an outlined folder to target that folder. In flat panels, empty list
content and ordinary file rows/image tiles target the panel's current folder;
the panel outline and drag label show that destination. The default Copy/Move choice follows volume evidence
as described above. Editors, toolbars, dialogs
and the hidden panel do not receive file drops.

Windows outgoing dragging offers Shell Copy and Move: same-volume default Move,
cross-volume default Copy, Ctrl-Copy and Shift-Move. X11/XWayland outgoing dragging
remains Copy-only. Native Wayland currently supports internal dragging only.
A drag reaches other applications only once its button has been held for 0.4 s:
until then the window keeps the pointer, so a quicker release outside it is
cancelled as above. Drops from other applications are not timed.
Incoming native transfers are Copy-only; use Ctrl/Copy in an external source application if it
otherwise offers only Move. Legacy X11 sources without XDND proxy support cannot
drop into the application. Incoming drops share the verified copy queue and its
conflict prompts. On Windows and X11/XWayland, an incoming drop refused after the Copy
cursor was shown (for example more than 4,096 items, a full copy queue, a destination
that changed, or a source that does not deliver its file list) shows its reason in the
status line instead of ending silently; a drop that never showed the Copy cursor ends
without a message. An external destination application performs outgoing operations
under its own rules; Midnight Explorer never deletes sources merely because an
external target requests deletion. Unoptimized targets that request source cleanup
receive a retained-source notice; missing completion feedback is reported as
unconfirmed. Native Shell tests pass; final integrated UI/deployment qualification
is tracked [separately](verification/file-drag-drop.md#2026-11-10-folder-targets-and-windows-shell-move).

Right-click a file/folder or drive to open its Windows system context menu; Linux
offers the available application actions. A selected group keeps its selection;
an unselected item targets just that item. Pasting on a sole folder targets that
folder. Native-menu Copy/Paste uses the same verified copy service as Ctrl+C/V.

Each panel drive bar includes a sort chooser beside View, available in every mode.
F10 shows the current configured shortcut beside each action. Its category picker
can narrow the list; unsupported operations stay visible but disabled. Options,
Jobs, About, Custom Commands settings and Manage Hot Paths have configurable
action entries without default keys; the three panel view modes default to
Alt+2, Alt+3 and Alt+5. See the
[main menu design](superpowers/specs/2026-11-08-main-menu.md).

**Recycle Bin / Trash** appears at the same position among special locations in
the drive dropdown and drive toolbar, using a system icon with a fallback.
It opens the Windows Shell Recycle Bin or, on Linux including Ubuntu, desktop
Trash through the fixed `trash:///` URI. The panel location, selection and history
stay unchanged; this typed desktop action adds no internal filesystem provider.
Linux availability follows the backend-reported desktop opener, and opening
failures are shown to the user.
References: [GIO Trash handling](https://gnome.pages.gitlab.gnome.org/gtk/gio/migrating-gnome-vfs.html)
and [desktop URI opening](https://portland.freedesktop.org/doc/xdg-open.html).

The Hot Paths star adds the current folder; its arrow lists saved paths, then
Add path and Manage below a separator. Saved paths also appear in their own group
in the drive dropdown after drives and special locations. Both menus show
numbered Hot Paths as 1–9/0; saved paths have no direct drive-toolbar buttons.
Adding an existing path keeps its saved identity;
new paths take the first free number, and paths beyond ten remain in the menus.
Archive Hot Paths are an accepted implementation task: display
`archive.zip@path/in/archive` with native separators, persist the container and
internal location separately, and restore through the installed provider.
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
to close it (the last tab remains). Double-click the empty space after the tabs to
create a new tab at that panel's current location; tab bodies and the + button keep
their own actions. Tabs and their active position survive restart.
Explicit `--left` / `--right` locations override restoration for that pane. Drag the central
splitter to resize the panels, or double-click it to center. Tabs shrink uniformly
and then expose scroll arrows at their minimum width. The details strip below each
panel copies a field's displayed value
when clicked (filename, exact bytes, date/time, attributes or Unix permissions).
File-only selections show their total known byte size using the system locale.
Selected-folder totals are shown when known; the accepted status correction omits
uncomputed totals and `unknown` messages while keeping partial values approximate.
Available capacity appears at the right and
comes from the displayed directory's filesystem query, including mount/junction targets.

The terminal icon opens a new interactive terminal without submitting the draft;
its adjacent arrow selects the shell. The command directory ends with `>`.
Command history recalls a draft without
running it; only the configured Execute command shortcut or Run submits it.
Options > General groups Panels and Command line settings, including
readable/exact-byte sizes, history persistence and Clear. Appearance and Keyboard
remain separate pages.
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
F6 opens the same captured-destination workflow for moves of at most 64 native
roots, including ordinary directories and selected nested directories in adopted
native trees. Ctrl+X publishes a Cut clipboard from native flat or captured tree
selections; Ctrl+V then opens Move confirmation. Tree clipboard actions retain
provider identities and their original selection scope.
Unsupported mixed-provider, virtual or special-entry batches are rejected before
transfer. Same-volume native Move renames files or whole directories without
content reads or hashes. Cross-volume Move removes each exact source file only
after destination verification and publication succeed.
It does not wait for the entire batch or retain sources in recovery folders.
Source directories are removed postorder only when their contents were moved;
skips and failures preserve unfinished sources. Platform and metadata limits
remain explicit in the transfer contract.
Options is the gear at the right end of the top toolbar. The application moon
icon immediately to its right opens About, including the application version;
Escape or Close dismisses it. Both controls remain fixed when panel tools scroll.
In About, Left and Right switch between its tabs. In the library list, Up, Down,
Page Up, Page Down, Home and End move the selection, and Enter or a double-click
opens the selected library; Down moves from the filter field into the list.
Escape returns from a library's details to the list, and the next Escape closes
About.
The bottom bar contains F1–F12 and Jobs, omitting unavailable buttons for network
drive dialogs. Jobs shows queued/running operations and allows cancellation.
In active Copy/Move/Delete progress, Escape or X asks whether
to cancel; Continue is the default, and Escape in that question leaves work running.
Cancellation retains changes already completed. No Trash
failure ever falls back to permanent deletion. F7 accepts paths outside the active
pane, refuses intermediate links and leaves already-created intermediate folders
on error. Relative paths start in the captured panel, never the process directory.
F3 routes by extension to an applicable installed viewer; Alt+F3 forces the basic
plain-text viewer, preserving Unicode and disabling highlighting and markup rendering.
Both require a compatible, enabled package in the installation's `viewers/` directory.
Clipboard copy has a background queue and full SHA-256 destination readback by
default. Explicit quick mode checks size and 64 KiB first/middle/last samples
(all bytes up to 192 KiB); it can miss corruption between samples.
Existing names offer Skip, Skip all, a different name, recoverable Replace/Replace
all or explicit directory Merge/Merge all, Cancel job and Decide later. In such a
decision window, Escape and X close it like Decide later and keep the decision
pending in Jobs; only Cancel job cancels the job. Jobs can reopen deferred conflicts
while browsing continues.
Copy retains its sources;
replacement retains displaced originals. See [transfer limits](developer/TRANSFERS.md).
Other unlisted Salamander/Total Commander
shortcuts, such as Shift+F9 for a bookmark menu, are not implemented.

Custom Commands can be reordered by dragging rows in Options → Custom Commands,
or with its Up/Down buttons. Escape cancels an active drag before closing Options.
Reordering preserves command identities and assigned shortcuts.

Custom Commands appear in the keyboard editor as `user-command.<id>` actions,
with their configured names and no default chord. They apply to the focused file
panel outside filename-search editing. Renaming or reordering preserves bindings;
deleting a command removes only that command's overrides. Adding a command updates
the catalogue without restarting the application. Opening settings or Preview
never executes it.

Viewer windows have their own configurable shortcut context:

| Keys | Viewer action |
| --- | --- |
| Escape | Close the viewer, including when search has focus |
| Space / Alt+Right in content | Next regular file in captured panel order, wrapping at the end; in very large folders the order is a window around the opened file |
| Backspace / Alt+Left in content | Previous regular file in the same captured order, wrapping at the beginning |
| Ctrl+F | Focus search |
| Enter in search / F3 | Next match |
| Shift+F3 | Previous match |
| Ctrl+A in content | Select the visible rows of the loaded page (bounded copy) |
| Ctrl+C in content | Copy selected displayed text |
| Arrows / PageUp / PageDown | Scroll content |
| Ctrl+Home / Ctrl+End | First / last page; an image larger than the window goes to its top / bottom (left / right edge when it is only wider) |
| Ctrl+W | Toggle word wrap in Text or Source mode |
| Ctrl+Plus / Ctrl+Equal | Zoom image in |
| Ctrl+Minus | Zoom image out |
| Ctrl+0 | Fit image without upscaling |
| Ctrl+1 | Actual image pixels (1:1) |
| Ctrl+PageUp / Ctrl+PageDown | Previous / next PDF document page; also available as page buttons |
| F7, Alt+Down, Alt+Right or N in file comparison | Next difference: changed lines, or the next region of differing bytes in the hex view; configurable as `compare.next-difference` |
| Shift+F7, Alt+Up, Alt+Left or P in file comparison | Previous difference (lines or byte regions); configurable as `compare.previous-difference` |
| Alt+Home / Alt+End in file comparison | First / last difference (lines or byte regions); configurable as `compare.first-difference` / `compare.last-difference`. Difference navigation stops at either end and aligns the changed rows in both panes. Editing controls retain their own keys. |

In image mode, Ctrl+wheel zooms around the pointer and dragging pans the image.
Text search/wrap controls are hidden for images. The same Close and previous/next
file actions apply to images and PDF documents. Images exceeding the host's four-megapixel display surface are
resampled to that bounded surface; 1:1 retains the physical size, with reduced
display detail on larger canvases.

The optional Chromium HTML viewer uses the same configurable Find, Copy, zoom,
navigation and previous/next-file actions. Escape always closes it. Space,
Backspace and Alt+Left/Right first reach Chromium, which keeps editable-field keys
inside the page. Outside an editable field the host resolves the current configured
action; delayed replies are ignored after viewer content loses focus. If that shortcut was removed, Space scrolls
the page; Shift+Space does the same in reverse when unassigned. AltGr input stays
with the page instead of triggering application shortcuts. Ctrl+0 resets HTML
layout zoom to 100%; the image viewer's fit behavior is unchanged. HTML file
cycling uses only the browser-arbitrated chords above. Other Previous/Next-file
remaps, such as Q or Ctrl+J, remain page input in HTML; they still work in the
other viewer modes.

Search retains ordinary text editing, clipboard keys, spaces and Backspace. Previous/next
file actions apply only to viewer content; every alias can be remapped or removed in
Options → Keyboard. Comparison windows have a separate content context, so their
Alt+Left / Alt+Right aliases continue to navigate differences. Main-window actions
do not dispatch inside viewer windows. Viewer mode is selected using the bottom
dropdown; Markdown search/copy uses rendered text, while Text mode uses literal source.

Bulk selection runs through the backend and excludes `..`. Text editors retain
their ordinary Ctrl+A behavior. The requested Ctrl+D takes precedence over
Total Commander's bookmark meaning. During progressive listing, all/invert may
wait for that listing to complete; input remains responsive.

Select files and folders (`selection.all`) uses Ctrl+Shift+A. It replaces
selection with displayed top-level files and folders, clearing nested tree marks.
Invert files only (`selection.invert`, Ctrl+I) toggles displayed files, including
expanded descendants, while preserving folders. Invert files and folders
(`selection.invert-all`, Ctrl+Shift+I) toggles only displayed top-level rows and
preserves nested marks. The synthetic parent and context-only ancestors remain
unselectable.

Select files only (`selection.files`, Ctrl+A) replaces selection with displayed
top-level files, clearing folder and nested marks. Deselect files and folders
(`selection.clear`, Ctrl+D) clears every mark, including nested rows. Ctrl+Shift+D
has no default assignment. These selection actions appear in F10 → Selection and
Options → Keyboard, including filtered lists and archives. The optional files-only
clear action (`selection.clear-files`) remains configurable without a default key
for compatibility with saved custom mappings. Directory
links/junctions count as folders; archive containers in native listings count as
files. Explicitly selecting nested rows by click or range remains available.
Existing user-assigned shortcuts take precedence over new default bindings.

The synthetic `..` row is navigation only: it cannot be renamed, viewed, edited,
copied, dragged, deleted or have its attributes changed. With no real selection,
the corresponding footer buttons are disabled. If real entries are selected,
Copy/Delete still target that captured selection even when the caret is on `..`;
Rename/View/Edit require a real focused entry. Creating a file/folder and pasting
into the current location remain available. This follows Salamander's parent-row
enablers in `src/mainwnd1.cpp:2748–2910`; backend target validation also rejects
paths ending in parent navigation instead of relying only on disabled UI controls.

Behavioral references: local Salamander `src/salamand.rc:61–102` and
`src/fileswn0.cpp:1220–1260`. Requested Ctrl+P/Ctrl+Enter behavior and folder-tab
navigation take precedence over an upstream shortcut with a different meaning.

Panel warning/error icons are independent detail controls. With an icon focused,
Enter or Space opens its diagnostic report; Escape closes the report viewer.
This does not change file selection or retry/expand a folder. A failed tree
branch has separate Retry/Collapse controls. No new global shortcut is reserved.

## Hidden files and selection by mask

**Show hidden files** (`panel.hidden-files`, default **Ctrl+H**, in the file list and
during filename search) is one choice for both panels. Hidden entries are names
starting with a dot on Linux and macOS and entries with the hidden attribute on
Windows (dot-names are ordinary there). Dot-named archive members are hidden on
Linux and macOS too; on Windows they carry no attribute and stay listed. Network
locations never hide anything. Hidden entries are shown by default. F10 → Panels
and navigation and each panel's View menu offer **Hide hidden files** or **Show
hidden files** for the current state, and Options → General has a **Show hidden
files** check box. The command saves the choice at once; the check box previews it
until OK. The saved choice (settings schema 35) applies after a restart.

The backend leaves hidden entries out of listings, archive listings and inline-tree
projections (a hidden folder's loaded contents included). A change re-reads both
panels in place: the cursor on a hidden row moves to the nearest shown row (the
next one, or the previous one at the end), marks on hidden entries are dropped and
stay dropped when they are shown again, and the status line under the list shows
"N hidden" (the listing's hidden entries, and in an inline tree also those of
expanded folders).
Ctrl+A, the invert and mask selections, Make File List, folder sizes without a
selection, F3's previous/next file and **Compare directories** see only shown
entries. A selected
or focused folder is still copied, moved, deleted, measured or checksummed with all
of its contents, hidden ones included. **Find** is independent of the choice and
searches hidden entries as well; use its masks to leave them out. At startup the
first listings are asked for before the settings are read; when the saved choice
hides hidden entries they are listed again (a listing still in progress restarts).

**Select by mask** (`selection.mask-select`, default **Ctrl+=** and **Num +**) and
**Deselect by mask** (`selection.mask-deselect`, **Ctrl+−** and **Num −**) open a
small dialog: the mask field starts with the last mask (`*` at first) and Enter or
OK applies it, Escape or Cancel closes it. Masks use the panel filter's Masks syntax: `*` any text, `?` one
character, `;` between masks, one `|` before excluded masks and `;;` for a literal
semicolon; letter case is ignored. Files whose names match are marked or unmarked;
folders and directory links take part only with **Include folders**, which is
remembered with the mask. Only rows the panel shows change (a panel filter or
filename search filter limits them too), marks on other rows stay, and `..` is
never marked. In inline trees every shown row takes part, so with Include folders
the subfolders of expanded branches (which list folders only) do too; context-only
ancestors kept for a filter never change. **Num \*** (the `selection.mask-invert`
action) inverts files only, like Ctrl+I, or top-level files and folders, like
Ctrl+Shift+I, when Include folders is checked; without a keypad, press the one of
those two that matches. OK and Enter do not accept an invalid mask (two `|`, nothing
after `|`, more than 64 masks or 4096 bytes): the dialog stays open and says why,
as the panel filter's does, and neither the marks nor the remembered mask change.

The keypad keys are physical keys: the number row's + − * stay typed text (they
start or extend a filename search), and during filename search the keypad keys type
too. Ctrl+= and Ctrl+− work without a numeric keypad. They follow the typed
character: on a Czech layout Ctrl+= is the = key next to ´ and Ctrl+− the − key,
and Ctrl++ selects too where + needs no Shift (Nordic layouts). The number row and
the bracket keys keep their meanings by position: where = needs Shift on a number
key and + sits on a bracket key (German), Ctrl+= is Hot Path 10's assignment and
Ctrl++ inserts the right panel's path, and where − is on a number key (French
AZERTY), Ctrl+− opens Hot Path 6. There, use the keypad, F10 → Selection or a key
of your own in Options → Keyboard. Ctrl+Shift+= still bookmarks the folder. Menus show Ctrl+= and Ctrl+−, and their tooltips list every
key. Options → Keyboard records the keypad keys as `NumpadAdd`, `NumpadSubtract`
and `NumpadMultiply` and can move the three actions to other keys. A saved
assignment of Ctrl+H, a keypad key, Ctrl+= or Ctrl+− to another action keeps
working; only that default then stays unassigned. All three are also in F10 →
Selection.

## Attributes, times and case editor

On the Times page of the Ctrl+F2 editor, the Date, Time and UTC offset fields
accept typing. **Up/Down** change the part under the cursor (year, month or day;
hour, minute or second; offset hours or quarter hours), and **Alt+Down** opens
the field's calendar or time picker. In the calendar, **Left/Right** move one
day, **Up/Down** one week and **Page Up/Page Down** one month; **Enter** or
**Space** chooses the date and **Escape** closes only the calendar. In the time
picker, **Left/Right** choose hours, minutes or seconds, **Up/Down** change them
by one and **Page Up/Page Down** by ten; **Enter**, **Space** or **Escape**
close it. **Space** or **Enter** on a row's **Now** button fills its fields with the
current local date, time and offset and ticks the row; Apply sets exactly that
moment. Outside a picker, **Escape** closes the editor without applying.

## Find result actions

Within the focused Find result list, View (**F3**), Basic View (**Alt+F3**) and
Edit (**F4**) use the current configured file-action bindings. Their button hints
update from the same map. **Enter** and double-click retain Go to file; **Open** is
an explicit button because the system association may execute the file.
Actions require a regular native result and fresh identity validation; a stopped
search retains its results, while a closed or restarted search revokes pending
actions. See the [Find contract](developer/FIND.md).

### Panel view and sort menus

Both panels show the current configured bindings beside Detailed, Brief,
Thumbnails, Name, Size, Modified and Extension in their dropdown menus. Reverse
order shows the binding of the current sort column, which toggles its direction.
Rebinding or disabling a shortcut updates these hints immediately, including an
already open menu. The compact closed buttons keep their short labels.
