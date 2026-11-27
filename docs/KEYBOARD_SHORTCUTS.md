# Keyboard shortcuts — Midnight Explorer 0.6.3

This map describes the current source; a packaged release may be older. It lists the
default keys of the main window, its dialogs and the viewer windows and how to change
them; [Key-combination syntax](#key-combination-syntax) and [Action IDs](#action-ids)
at the end define what Options › Keyboard and the settings file accept. Terms such as
[panel](GLOSSARY.md#panel), [Hot Paths](GLOSSARY.md#hot-paths) and the Brief, Detailed
and Thumbnails [views](GLOSSARY.md#views) are defined in the [glossary](GLOSSARY.md).

## Checksums window (Ctrl+F8)

**Ctrl+F8 — checksums** opens a Checksums window for the selected files, or the
focused file or folder; with nothing selected and the cursor on the parent row (or in
an empty folder), for the files of the shown folder. It takes native files only, not
archive members, and at most four Checksums windows are open at a time. The
[checksum API](developer/CHECKSUMS.md) describes the window.

- Nothing is calculated until you press **Calculate**. The algorithm dropdown has a
  checkbox per algorithm (SHA-256, SHA-512, MD5, SHA-1 and CRC32; only SHA-256 is
  checked in a new window) and an **Include subfolders** checkbox. From the keyboard,
  Enter, Space or an arrow key opens it; Up and Down move, Space ticks the row, Enter
  shows that algorithm's column (or ticks Include subfolders), and Escape or Tab
  closes it.
- If the selection contains a checksum file, the window opens on **Verify** with that
  file chosen. The window saves and verifies SFV (CRC32), MD5, SHA-1, SHA-256 and
  SHA-512 files. On the **Verify** page, the **…** button chooses a checksum file and
  **Verify** reads it on background workers.
- **Save manifest…** asks for a file and saves at once; partial results and an
  existing file are confirmed in the window first (**Save** / **Replace**).
- Footer actions that do not fit beside **Close** move into a **More…** menu.

In the checksum results list:

- a click selects a row, **Ctrl+click** toggles one, **Shift+click** selects the range
  from the anchor (the row last clicked, toggled or selected with a key), and
  **Ctrl+Shift+click** adds that range;
- **Up**, **Down**, **Page Up**, **Page Down**, **Home** and **End** select the row
  they reach; with **Shift** they extend the range, with **Ctrl** they move only the
  cursor;
- **Space** toggles the cursor's row and **Insert** toggles it and moves down;
  **Ctrl+A** selects every row and **Ctrl+D** none;
- **Copy** (a footer button) copies the selected rows as checksum-file lines,
  **Ctrl+C** their checksums of the shown algorithm, one per line, and
  **Ctrl+Shift+C** their full rows; **Save manifest…** still writes every calculated
  row;
- **Shift+F10** or a right click opens the copy menu (one entry per algorithm, and full
  rows), for the whole selection when its row is selected, else for that row;
- **Enter** starts the page's primary action, as its button would;
- **Escape** closes the window and cancels its running work.

These keys belong to that window: there Ctrl+D deselects result rows and leaves the
panels alone.

## Changing shortcuts in Options › Keyboard

The tables on this page list the default bindings. **Options › Keyboard** can change,
disable, or restore each application action, including navigation, selection, command editing,
drive letters, numbered tabs/Hot Paths, and the drive dropdown's As Other Panel action.
Search commands by name or command ID, select a command, then focus **Press shortcut
keys** and press a combination. The command list is one Tab stop: its frame takes the
active color while it has the keyboard, and the arrow keys, Home and End select a
command. **Assign** adds it while preserving the command's
other aliases; choose an existing alias and use **Remove** to remove just that one.
An action accepts up to four alternate single chords. The editor displays the
effective context and current owners before assignment. An overlapping assignment
opens a **Replace shortcut** dialog listing the affected commands. **Replace**
removes that chord from those commands and assigns it to the selected command;
other aliases and nonoverlapping contexts remain intact. **Cancel**, Escape or
the default Enter response keep the existing assignments. Invalid or reserved
shortcuts remain unavailable. **Restore command** restores one action. It is enabled
only when that would change something: the command's saved shortcuts differ from what
its defaults give beside the other commands, or a captured shortcut waits in **Press
shortcut keys** (restoring discards it). When the defaults cannot return beside the
other commands' shortcuts, it is enabled and says why. **Restore all
defaults** asks before resetting the complete map. In a clicked capture field, Tab
or Escape records that key and leaves the field without running an application
command; reached with Tab or Shift+Tab, the field lets those keys pass without
recording them, so tabbing through the page changes nothing.
Changes preview immediately; **OK** saves them with the other preferences and
closes Options. **Cancel**, X or Escape outside capture discard the entire Options
session, restoring previous mappings and previewed appearance. An open inline
editor takes Escape first: the custom archive extension field and a new argument
row cancel only their own edit, an open color picker closes, and the next Escape
closes Options. The [action catalog](#action-ids) also supplies the application's
displayed shortcut hints and Help.

### Keys that stay with text editing

Native widget editing and IME shortcuts remain native; operating-system-reserved
keys may not reach the application. Ctrl+Space stays available to the IME, and broad
application shortcuts cannot take over ordinary typing or editor copy/paste.
While filename search is active, Ctrl/Meta+C, X, V and the Insert copy/paste aliases
edit search text even if reassigned to file actions. Those bindings apply to files
after leaving search. Ctrl+A and Shift+Delete retain their listed file actions.
Physical number keys may be entered as `Ctrl+1` or `Ctrl+Digit1`; these are the same
binding. The editor accepts `Ctrl`, `Alt`, `Shift`, `Meta`, named keys such as
`PageDown`, and Unicode characters. `Plus` denotes the plus key
([key-combination syntax](#key-combination-syntax)).

## Function-key footer

The F1–F12 footer follows the held Ctrl, Alt, Shift and Meta modifiers, including
combinations. Every cell uses the effective binding from Options › Keyboard; an
unassigned cell is disabled and its tooltip says so. Changing, removing or restoring
a binding updates its caption, tooltip and click action, including unmodified
function keys. AltGr is typing input and does not activate a modified layer.
Combinations that the operating system reserves may never reach the application.

The Connect and Disconnect buttons are hidden where the operating system has no
network drive dialogs, as on Linux; the remaining buttons share the space. This
follows the effective action after remapping, so Ctrl+F11 (Zoom) and Ctrl+F12
(Filter) stay visible.

A caption first gives up its icon, then shrinks to a readable minimum. A cell that
still cannot show the whole caption, and every very narrow cell, shows the function
key alone, never a shortened caption; the tooltip and the accessible name keep the
complete chord and action. Keys that do not fit beside [**Jobs**](GLOSSARY.md#jobs) scroll into view
(drag the strip) instead of showing part of a key. A mouse press captures the action
and the panel, so releasing or changing modifiers before releasing the mouse cannot
trigger a different action. While one panel fills the panels area (Ctrl+F11),
switching panels shows the other panel in its place.

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
| F11 | Connect | Zoom panel | — | — | Clear comparison marks |
| F12 | Disconnect | Panel filter | — | — | — |

This matrix shows main file-panel defaults, including filename search; viewers and
dialogs have their own local shortcuts. Other modifier combinations have no default
main-window function-key bindings; users can assign them if the OS makes them available.

## Main window keys

File navigation, tab and Hot Path shortcuts apply in the focused file list; the
path, command and Options editors keep their typing. Numbered tabs and Hot Paths use
the physical digit keys on every keyboard layout and do not intercept RightAlt/AltGr.
Number 0 is Hot Path 10. Bare digits remain text while a path, command, search or
Options editor has focus. An unassigned Hot Path number is consumed without
navigating or starting a search. In the drive dropdown and the Hot Paths menu, the
same number keys open that Hot Path. A digit key that your saved settings assign to
another action keeps that action, and its Hot Path default stays unassigned; change
or remove that assignment in Options › Keyboard to free the number.

| Keys | Current action |
| --- | --- |
| Tab / Shift+Tab in files | Switch the active panel |
| ↑ / ↓, PageUp / PageDown, Home / End | Move the focused row and scroll it into view, also when it stays: Home or ↑ on the first row and End or ↓ on the last bring a list the wheel scrolled away back to it |
| ← / → in Brief | Move to the previous / next file column, and scroll the focused file into view (also at either end) |
| ← / → and ↑ / ↓ in Thumbnails | Move between cells and grid rows, and scroll the focused cell into view (also at either end) |
| Plain typing in files | Start Unicode case-insensitive filename-prefix search; the number row's + − * are typed too (Ctrl+= / Ctrl+− and the numeric keypad's + − * select by mask) |
| ↑ / ↓ during search | Visit previous / next matching name without wrapping |
| Escape during search | Leave search and restore the complete listing |
| Backspace during search | Remove one extended grapheme; an empty query exits |
| Enter | Enter a folder or open the focused file through its host association. During filename search, Enter opens only a matching row; otherwise nothing opens and the panel says that no name matches. The search only moves the cursor: F3, F5, F8 and the other commands close it and act on the selection or the focused row |
| F3 / Alt+F3 in files | Open the applicable installed viewer / force basic Text mode. Also works for root files while inline folder branches are expanded; Space / Alt+Right advances through the captured visible file order; Backspace / Alt+Left reverses it. |
| F4 in files | Open the focused regular local file with the configured external editor |
| Ctrl+M in files | Make File List: [captured selection](GLOSSARY.md#captured-selection) to clipboard, viewer or UTF-8 file |
| Backspace / Ctrl+PageUp | Parent folder |
| Alt+← / Alt+→ | Back / forward history |
| Alt+↑ / Alt+↓ in files | Previous / next selected file or folder in displayed order, including expanded tree rows. Skip the current row, preserve all marks, and stop at the boundary without wrapping; inactive during filename search. Configurable as `panel.previous-selected` / `panel.next-selected`. |
| Shift+Backspace / Ctrl+Backspace / Ctrl+Backslash | Root of the focused panel's location: `/` on Linux; on Windows the drive's root, or the share's root for a `\\server\share` path; inside an archive the root of the outermost archive; in a Network location the Network root |
| Ctrl+PageDown | Enter a known folder; never launch a regular file |
| Ctrl+G | Focus the path editor; Enter there, or its Go button, opens the typed folder, or the folder of a typed file with the cursor on the file |
| Alt+F1 / Alt+F2 | Open left / right volume choices |
| . in the drive dropdown | As Other Panel: open the other panel's current folder |
| Shift+drive letter | Switch volume on Windows, using its remembered path; when no drive has that letter (always on Linux), the uppercase letter is typed into the filename search instead |
| Ctrl+F1 / Ctrl+L | Show [drive information](DRIVE_INFORMATION.md) for the volume that holds the active panel's folder (inside an archive, the volume of the archive file): label, filesystem, capacity and storage type |
| Ctrl+F2 | Attributes, times and filename-case editor for the captured selection |
| Alt+2 | Switch the active panel to Brief view |
| Alt+3 | Switch the active panel to Detailed list view |
| Alt+5 | Switch the active panel to Thumbnails view |
| Space | Toggle selection, advance one row and calculate the focused directory's size on the backend |
| Insert | Toggle selection and move to the next row |
| Shift+click / Ctrl+Shift+click in files | Select / add the range from the anchor to the clicked row, in every view. The anchor is the row last clicked or Ctrl+clicked; after the cursor moved without Shift (cursor keys, Space, Insert, search, Alt+↑ / Alt+↓) or Ctrl+D, Ctrl+A or an invert, the range starts at the cursor's row |
| Ctrl+A | Select displayed top-level files only; deselect folders and nested rows |
| Ctrl+Shift+A | Select displayed top-level files and folders; exclude `..` and nested rows |
| Ctrl+D | Clear all selection, including folders and expanded descendants, in the focused panel |
| Ctrl+I | Invert file selection in the focused panel; preserve selected and unselected folders, including directory links |
| Ctrl+Shift+I | Invert top-level files and folders; preserve nested tree marks |
| Ctrl+=, Num + or Ctrl++ / Ctrl+− or Num − in files | Select / Deselect files by mask (Ctrl+= and Ctrl+− need no numeric keypad): a small dialog with the mask (the last one, `*` at first) and Include folders. Configurable as `selection.mask-select` / `selection.mask-deselect`; see [selection by mask](#hidden-files-and-selection-by-mask) |
| Num * in files | Invert files only, or top-level files and folders when Select by mask last had Include folders checked; without a keypad, Ctrl+I and Ctrl+Shift+I invert the same two ways. Search results fed to a panel invert the same way. Configurable as `selection.mask-invert` |
| Ctrl+H | Show or hide hidden files in both panels (`panel.hidden-files`); also in F10, each panel's View menu and Options › General |
| Ctrl+C / Ctrl+V in files | Publish selected/focused files to the native clipboard / paste into the captured active folder; a Cut clipboard opens Move confirmation |
| Ctrl+X in files | Publish a Cut clipboard for captured native files and folders. Paste opens the same Move confirmation as F6 (up to 1,000,000 selected items; links as its link choice says); archive Move is unavailable |
| F5 | Copy the captured selection or focused item, including selected nested native/archive folders in expanded trees; edit the destination folder, initially the other panel. Tab reaches the **Links** choice, where Up and Down choose between copying the files links point to and keeping links as links. Enter queues the copy; Escape cancels the dialog |
| F6 | Move captured native files or folders, including selected nested directories in expanded native trees, to an editable destination initially set to the other panel. Same-volume native rename reads no content; copied files are verified before source removal. Links move as links, or (Copy the files links point to) their files are copied and only the links removed. Archive Move is unavailable |
| Ctrl+F / Alt+F7 | Open a separate Find window for the active native folder: masks, multiple roots, subfolders and optional UTF-8 content text or per-line regular expression, optionally inside archives (every archive and member, however large, until the search ends or Stop). [Feed to panel](GLOSSARY.md#feed-to-panel) shows the results as a list in the panel: the panel keys (F3, F4, F5, F6, F8, Shift+F8, Insert, Space, Enter) act on the results, Backspace returns to the folder. Up/Down moves through the results; Ctrl+click, Shift+click, Shift with the cursor keys, Space, Insert, Ctrl+A and Ctrl+D select several (see [Find result actions](#find-result-actions)); Enter or Go to file validates the result and locates it in its originating panel; F1 shows or hides syntax help for the chosen match mode and moves the keyboard into it (Up/Down and Page Up/Page Down scroll it); Escape first closes the help or the Add folder entry, and otherwise closes that Find window. In a viewer, Ctrl+F searches the opened file instead. |
| Ctrl+R / Ctrl+F9 | Refresh |
| Ctrl+F3 / Ctrl+F4 / Ctrl+F5 / Ctrl+F6 | Sort the active panel by name / extension / modified time / size; repeat to reverse the order. Each detailed panel also has a clickable Ext header next to Name, with the current sort arrow; repeat-click reverses the order. The drive-bar sort dropdown offers Extension in every view mode. |
| Ctrl+F7 | Open the same metadata editor with filename case conversion enabled |
| Ctrl+F8 | Open modeless Checksums: Calculate (SHA-256 default; optional SHA-512/MD5/SHA-1/CRC32 and recursion in the algorithm dropdown), Stop, copy, Save manifest… (confirms replacing an existing file) and Verify (`.sfv`, `.md5`, `.sha1`, `.sha256`, `.sha512`). Enter in the results or on the page runs Calculate or Verify when its button would, not from Details or while a save question waits |
| Ctrl+F10 | Compare the two displayed native directories as [Open Salamander](GLOSSARY.md#salamander) does (size, date and time, attributes, contents, subfolders) and select the differences; also available as Compare directories in the shared toolbar. In its dialog Enter compares from an option or a field (Space ticks an option); Alt+Z Size, Alt+D Date and time, Alt+T tolerance, Alt+H one- or two-hour differences, Alt+A Attributes, Alt+C Content, Alt+S Compare subfolders, Alt+O folder attributes, Alt+E one-sided folders, Alt+M exact case, Alt+F Ignore files, Alt+N Ignore folders, Alt+R Compare. While contents or subfolders are compared the dialog shows the progress: Escape, Alt+C or Cancel stops it and leaves the panels unchanged |
| Escape (no filename search) / Ctrl+Shift+F11 | Clear directory comparison marks: remove the =, ≠, + and ? badges from both panels and the result line below the panel; the comparison's selection stays. Also the × at the right end of the result line (it closes the line) and *Clear comparison marks* in the main menu. Without marks or a result line, Escape does nothing in the panel, as before. Point at a badge to see what differs for that row |
| Ctrl+Shift+C | Compare two selected files in the active panel; otherwise use its single selected/focused file and the opposite explicit selection, or its unique visible same-name file when nothing is selected there. If the pair cannot be determined, choose File A and File B using paths or Browse; Enter compares, Escape cancels. Exact native names are matched. Image pairs support Side by side, Swipe and Onion skin. Binary files, text over the text limits (32 MiB, 1,000,000 lines or a line over 16 KiB) and other pairs are compared as bytes in a hex view; the Hex button shows the bytes of a text pair. Compare files in the shared toolbar also works without a panel target. |
| Ctrl+F11 | Fill the entire panels area with the active panel, hiding the other panel and splitter; press again to restore the previous split |
| Ctrl+F12 | Configure the active panel's saved Masks, Regex or typed metadata Expression filter; also available on each drive bar immediately left of View. OK (or Enter in the expression) applies the filter, Cancel discards the draft; Show all or the panel indicator × disables it without discarding its expression |
| Down / Alt+Down in the panel filter's expression | List recently applied filters, newest first. Up / Down move through the list; Enter or a click puts the filter in the expression and restores its mode and case, without applying it. Escape closes the list, a second Escape the dialog |
| Ctrl+Shift+S / Ctrl+Shift+F10 | Start or cancel folder size calculation in the active panel: selected folders when selection exists, otherwise all displayed folders including descendants in expanded branches. Native/archive listings and inline branches are supported. |
| Alt+Enter / Ctrl+Alt+Enter | Properties of selected items, or focused item |
| Ctrl+T | New tab at the current folder |
| Ctrl+W | Close active tab; keep the last tab |
| Ctrl+Tab in files or path editor | Focus the bottom command editor |
| Ctrl+Shift+Tab in files | Previous tab in that panel |
| Ctrl+U in files | Swap the complete visual panels, preserving their tabs and running work |
| Ctrl+Shift+1…9 | Switch to that numbered tab in the focused panel; missing tabs do nothing |
| Ctrl+Shift+0 | Assign the current folder to Hot Path 10 (shown as 0); other numbers are assigned in Manage Hot Paths |
| 1…9/0, Ctrl+1…9/0 or Shift+1…9/0 | Open the numbered Hot Path in the focused panel |
| Ctrl+Alt+number or Alt+Shift+number | Open the numbered Hot Path in the other panel |
| Ctrl+Shift+Plus or Ctrl+Shift+Equal | Add the current folder once, using the first free numbered slot |
| Ctrl+Enter in files or command editor | Append the focused basename to the end of the command draft and focus the command editor |
| Ctrl+Shift+P in files or command editor | Append the focused native item's complete path, including filename, and focus the command editor; no added quotes or spaces |
| Ctrl+P in files or command editor | Insert the active panel path, ending in a native path separator, at the command selection |
| Ctrl+[ / Ctrl+], also Ctrl+Shift+[ / Ctrl+Shift+] | Insert the left / right panel path, ending in a native path separator, at the command selection |
| Ctrl+Space | Available to the input method (IME); Ctrl+P inserts the path |
| Ctrl+click on the terminal button, or Ctrl+Enter / Ctrl+Space while it has keyboard focus | Open the selected shell elevated in the active panel's folder: as administrator after Windows asks for consent (UAC), or a root shell through sudo on Linux. The same command, **Open terminal as administrator** (Linux: **Open root terminal (sudo)**), has no default key, can be assigned in Options › Keyboard and is in the F10 menu under Tools |
| Enter in command editor | Explicitly submit the command to the selected system terminal |
| Alt+↑ / Alt+↓ in command editor | Rotate to older / newer stored commands, wrapping; terminal type stays unchanged |
| Escape / Ctrl+Tab in command editor | Return to the active file list |
| F1 | Keyboard help: every action with its current keys and where they apply |
| F2 | Rename the focused native file/folder, including an expanded folder occurrence; Enter confirms, Escape cancels. Marked selection does not change the target; collisions require overwrite consent. While the rename runs and until the new name is listed (at most 3 s after it completes) the panel holds input and then replays the keys typed meanwhile; Escape releases it at once |
| Shift+F4 in files | New empty file in the captured panel folder; Enter confirms the name, Escape cancels. A listed file's name is asked about first; only the Replace file button (not another Enter) replaces it, keeping the original in a recovery folder |
| F7 | Create an absolute or relative folder path, including `..` and missing intermediate folders. Enter with an empty name keeps the dialog open and says "Type the folder name or path first." under the field until a name is typed; New file, Rename, Copy and Move do the same for their name fields |
| Delete / F8 | Confirm moving the selection (or focused entry) to the system Trash |
| Shift+Delete / Shift+F8 | Explicitly confirm permanent deletion |
| F11 / F12 | Connect / disconnect network drive through Windows system dialogs; unavailable on Linux, where these footer buttons are hidden |
| F9 | User menu: Command line, Bookmarks and Hot Paths, Properties, folder sizes and Checksums, then each configured [Custom Command](GLOSSARY.md#custom-commands), which runs on the active panel as from its toolbar button (a disabled one is listed but cannot run). Without Custom Commands the menu says so, and Set up opens Options › Custom Commands |
| F10 | Main menu: all actions, or one category of Files, Selection, Panels and navigation, Tools and Application; includes Options and Jobs, and each entry shows every key assigned to it |
| Alt+F4 | Exit |

### Command line insertion, menus and dialogs

Bracket shortcuts use the physical US `[` / `]` key positions, including on
keyboard layouts with different labels. They work from the file list and command
editor; AltGr remains available for typing. Options › Keyboard can reassign or
disable these actions, and their command-editor hints follow the saved mapping.
Outside the number row and these bracket keys, letter shortcuts such as Ctrl+C in
the checksum results and Ctrl+V in the path editor follow the character the keyboard
layout produces, as in other text fields, so on layouts without Latin letters (for
example Russian or Greek) paste a path with Shift+Insert or the context menu and copy
a checksum with Shift+F10 or the context menu.

Inserted paths and names are raw text: no generated quotes, escaping or separator
spaces. A folder insertion appends a native path separator only when one is missing;
a filename is inserted unchanged, with its own spaces and quotes. Insertion never
submits a command; the draft is shell syntax only on an explicit Enter or Run. A name
with control characters or bytes that cannot be shown exactly is refused rather than
inserted with replacements. Word deletion and ordinary text editing keep their
standard behavior. The file context menu and drop-down choosers support arrows, Enter
and Escape. In the F9 User menu and the F10 Main menu, Tab and Shift+Tab move between
items, Enter or Space activates the focused item and Escape closes the menu; Up/Down
in the Main menu's category chooser change the category. Unclear controls have
tooltips and accessible names. In dialogs, Tab and Shift+Tab move through the
controls, the header's close button (×) included, which Enter or Space presses;
Escape closes the dialog too, as the ×'s tooltip says. Help, Jobs and the job and
decision windows leave closing to their title bar, Escape and their own buttons,
without a ×.

## Mouse

Mouse: left click focuses, Ctrl+left click toggles selection, Shift+left click
selects an inclusive range, and Ctrl+Shift+left click adds that range. An unmodified
left double-click activates. Clicking a column header sorts by that column; clicking
the current one reverses the order.

### Inline folder trees

**Ctrl+click on a folder expander** starts bounded recursive expansion inline,
including directory symlink/junction expanders. Collapse/navigation cancels the
asynchronous walk; cycles and other failures stay local to the affected branch.
Known-empty directory links show no `+`. The tree shows folders down to 64
levels below the panel's own entries; expansion stops there and at the tree's
capacity limit, keeping what it expanded, and an **Expansion incomplete** notice
then says why and shows the folder where it stopped. Ordinary row Ctrl+click
keeps its selection behavior ([Mouse](#mouse)). See
[tree behavior and limits](developer/TREE_PANEL.md#retry-diagnostics-and-recursive-expansion).

### Selection, folder sizes and scrolling

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
On Windows the touchpad's own momentum is used instead.

## Drag and drop

### Copy or Move by default

File dragging follows Explorer-style defaults: a drop moves when every item and the
destination are on the same known volume, and copies otherwise, also when the volume
evidence is uncertain or mixed. **Ctrl + drag** forces Copy and **Shift + drag**
forces Move. **Escape** cancels the gesture. Ctrl+Shift, Alt or Meta while dragging
(link gestures elsewhere) offer no drop instead of silently doing something else.
These rules also apply to captured native folders in expanded tree branches; see the
dated [drag design](superpowers/specs/2026-11-10-folder-drag-and-native-handoff.md).

### Dragging within and between panels

Drag a file or folder with the left mouse button once the pointer has moved 6
pixels. A selected item supplies the selected group; an unselected item supplies
only itself. Touch scrolling does not start a file drag. Escape cancels the drag;
releasing before asynchronous preparation finishes cannot start a delayed drag.
A release less than 0.4 s after the mouse button was pressed is taken for an
accident and cancels the drag wherever it ends: nothing is copied or moved and no
dialog opens, the outline and drag label go away, selection and cursor stay, and for
4 seconds the source panel's status line shows "Drag cancelled: released within
0.4 s". The time counts from the press, so holding the button before moving counts
too.
Drop on an outlined folder to target that folder. In flat panels, empty list
content and ordinary file rows/image tiles target the panel's current folder;
the panel outline and drag label show that destination. The default Copy/Move
choice follows the volumes ([Copy or Move by default](#copy-or-move-by-default)). Editors, toolbars, dialogs and the
hidden panel do not receive file drops.

### Dragging to and from other applications

Dragging out to Windows applications offers Copy and Move; Explorer as the target
moves on the same volume and copies to another one, Ctrl copies and Shift moves, as
Explorer decides. Dragging out under X11, XWayland or native Wayland offers Copy only.
A drag goes to the application under the pointer as soon as it leaves the window; a
release there less than 0.4 s after the press still cancels it (the other application
sees the drag leave, or under Wayland receives an empty list), and the source panel
says so. Drops from other applications are not timed. Rows of an archive or the Network
view cannot be dragged. A native Wayland window exchanges files with other applications
only with `MIDNIGHT_WAYLAND_DND=1` (not yet qualified); otherwise it drags between its
panels.
Drops from other applications always copy; in a source application that offers only
Move, hold its Copy key (usually Ctrl). Legacy X11 sources without XDND proxy support
cannot drop into the application. Incoming drops share the verified copy queue and its
conflict prompts. On Windows, X11/XWayland and Wayland, an incoming drop refused after the Copy
cursor was shown (for example more than 1,000,000 items, a full copy queue, a destination
that changed, or a source that does not deliver its file list) shows its reason in the
status line instead of ending silently; a drop that never showed the Copy cursor ends
without a message. The target application performs an outgoing operation under its
own rules; Midnight Explorer never deletes sources because a target asks for it. When
a target asks for the sources to be removed after its copy, they are kept and a
**Move not completed** notice says so; a target that never confirms completion
gives a **Drop not confirmed** notice. Test evidence is in the dated
[drag-and-drop record](verification/file-drag-drop.md).

## Context menu

Right-click a file, folder or drive to open its Windows system context menu. Linux
file managers share no such menu, so on Linux the application shows its own: Open,
Copy, Cut, Paste, Rename, Move to Trash and Properties, each where it applies. A
selected group keeps its selection; an unselected item targets just that item.
Pasting on a sole folder targets that folder. The menu's Copy and Paste use the same
verified copy service as Ctrl+C/V.

## Drive bar, menus and special locations

Each panel drive bar includes a sort chooser beside View, available in every mode.
F10 shows every key assigned to each action. Its category picker can narrow the
list; unsupported operations stay visible but disabled. Options, Jobs, About,
Custom Commands settings and Manage Hot Paths have configurable action entries
without default keys; the three panel view modes default to Alt+2, Alt+3 and Alt+5.
See the dated [main menu design](superpowers/specs/2026-11-08-main-menu.md).

**Recycle Bin / Trash** appears at the same position among special locations in
the drive dropdown and drive toolbar, using a system icon with a fallback.
It opens the Windows Shell Recycle Bin or, on Linux including Ubuntu, desktop
Trash through the fixed `trash:///` URI. The panel location, selection and history
stay unchanged; this typed desktop action adds no internal filesystem provider.
Linux availability follows the backend-reported desktop opener, and opening
failures are shown to the user.
References: [GIO Trash handling](https://gnome.pages.gitlab.gnome.org/gtk/gio/migrating-gnome-vfs.html)
and [desktop URI opening](https://portland.freedesktop.org/doc/xdg-open.html).

## Hot Paths, tabs and the path bar

The Hot Paths star adds the current folder; its arrow lists saved paths, then
**Add path** and **Manage Hot Paths…** below a separator. Saved paths also appear in
their own group in the drive dropdown after drives and special locations. Both menus
show numbered Hot Paths as 1–9/0; saved paths have no direct drive-toolbar buttons.
Adding an existing path keeps its saved identity; new paths take the first free
number, and paths beyond ten remain in the menus. A Hot Path can be a folder, a
folder inside an archive or inside archives inside an archive, shown by its whole path
(`D:\data\a.zip\docs\b.7z\inner`) and opened through the installed archive provider,
or a Network location. A Hot Path
saved on the other operating system stays inactive: it keeps its number, opening it
only explains why, and Manage Hot Paths can remove it.
Manage shows full folder paths without custom names. Selecting a row keeps the
manager open; choose a number and **Apply shortcut** to update that saved path.
Moving between occupied numbers swaps them without removing either folder.
The drive dropdown's As Other Panel item
also works by mouse and uses the current
opposite folder after Ctrl+U swaps the panels. Back/Forward arrows have history
dropdowns. Tabs sit below the drive
toolbar. Hovering a path segment highlights its prefix; clicking opens that raw
ancestor; right-click copies that prefix without navigating. Click anywhere in the
blank space after the path, up to the bar's end, to edit it; Paste keeps the copied
text exactly; Enter or the editor's Go button navigates and Escape cancels. Middle-click a tab
to close it (the last tab remains). Double-click the empty space after the tabs to
create a new tab at that panel's current location; tab bodies and the + button keep
their own actions. Tabs and their active position survive restart.
Explicit `--left` / `--right` locations override restoration for that panel. Drag the
central splitter to resize the panels, or double-click it to center; with the
splitter focused (click it), Left and Right move it and Enter centers it. Tabs shrink
uniformly and then expose scroll arrows at their minimum width. The details strip
below each panel copies a field's displayed value when clicked (filename, exact
bytes, date/time, attributes or Unix permissions); Ctrl+click on the filename copies
the full native path.
File-only selections show their total known byte size using the system locale.
Selected-folder totals are shown when known; uncomputed totals are left out and
partial values stay approximate. Available capacity appears at the right and comes
from the displayed directory's filesystem query, including mount/junction targets.

## Command line and terminal

The terminal icon opens a new interactive terminal without submitting the draft;
its adjacent arrow selects the shell. Ctrl+click opens the selected shell
elevated instead: Windows asks for consent and a declined prompt only says so on
the status line; on Linux sudo asks for the password in the terminal. Programs
run as administrator on Windows do not see network or `subst` drives of your own
sign-in, so a folder on one is refused with that reason before any prompt. The
command directory ends with `>`.
Command history recalls a draft without
running it; only the configured Execute command shortcut or Run submits it.
Options › General groups its settings under Panels, File operations, Disk
operations, Command line, Updates and Settings file, including readable or exact-byte sizes,
history persistence and Clear. Appearance and Keyboard are separate pages.
History keeps the latest 100 distinct exact command strings across shells,
newest first, with a 1MiB aggregate text budget. Elevation is never recalled.

## Filename search

Search matches the beginning of files and folders literally, always ignoring
Unicode case. The matched original prefix is colored; `..` never matches. The
Options checkbox **Show only filename search matches** hides nonmatching entries; otherwise
search jumps among matches in the complete list. Home/End and PageUp/PageDown
also follow match order. Space appends query text while searching; Insert exits
search before its normal selection action. Native IME composition is not searched
until committed. Shift+drive keeps its Windows meaning; the plain and Shift digit
Hot Path keys type text during search and composition. Queries are independent per
panel, survive sorting/refresh and clear on folder/tab navigation. Pending or
empty-match searches cannot activate a leftover nonmatching keyboard cursor.

## Copy and Move

F5 Copy opens the destination dialog without changing the clipboard. The selection
and other-panel destination are captured at the gesture; later panel changes do
not retarget it. Edited destinations must be absolute paths to existing folders.
F6 opens the same captured-destination workflow for moves of up to 1,000,000
native items, including ordinary directories and selected nested directories in adopted
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

## Options and About

Options is the gear next to the right end of the top toolbar. The application
icon to its right, at the end, opens About, with the application version,
copyright and license; its View license shows the GNU GPL text, and Escape or Back
returns from it. Escape or Close dismisses About. Both controls stay in place when the panel
tools scroll.
In the Options page list, Up and Down (or Left and Right) show the page before or
after the focused entry and move the focus with it, stopping at the first and the
last page; Home and End show the first and the last page. Tab moves the focus from
entry to entry without changing the page, and Enter or Space shows the focused one.
In About, a focused tab heading switches between **About** and **Licenses** with Left
and Right. In the library list, Up, Down, Page Up, Page Down, Home and End move the
selection, and Enter, Space or a double-click shows the selected library's license;
Down moves from the filter field into the list. Escape returns from a library's
details to the list, and the next Escape closes About.

## Jobs, progress and conflicts

The bottom bar contains F1–F12 and Jobs, omitting unavailable buttons for network
drive dialogs. Jobs shows queued/running operations and allows cancellation.
In active Copy/Move/Delete progress, Escape or X asks whether
to cancel; Continue is the default, and Escape in that question leaves work running.
Cancellation retains changes already completed. No Trash
failure ever falls back to permanent deletion. F7 accepts paths outside the active
panel, refuses intermediate links and leaves already-created intermediate folders
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

## Custom Commands

Custom Commands can be reordered by dragging rows in Options › Custom Commands,
or with its Up/Down buttons. Escape cancels an active drag before closing Options.
Reordering preserves command identities and assigned shortcuts.

Custom Commands appear in the keyboard editor as `user-command.<id>` actions,
with their configured names and no default chord. They apply to the focused file
panel outside filename-search editing. Renaming or reordering preserves bindings;
deleting a command removes only that command's overrides. Adding a command updates
the catalog without restarting the application. Opening settings or Preview
never executes it.

## Viewer windows

Viewer windows have their own configurable shortcut context:

| Keys | Viewer action |
| --- | --- |
| Escape | Close the viewer, including when search has focus; while a whole-file copy runs, the first Escape stops it instead |
| Space / Alt+Right in content | Next regular file in captured panel order, wrapping at the end; in very large folders the order is a window around the opened file. In an HTML page with a page to go forward to, Alt+Right goes forward instead |
| Backspace / Alt+Left in content | Previous regular file in the same captured order, wrapping at the beginning. In an HTML page with a page to go back to, Alt+Left goes back instead |
| Tab / Shift+Tab in content | Next / previous link of the shown Markdown page, with a focus ring; Tab starts at the first link in view (`viewer.next-link`, `viewer.previous-link`). From the last link Tab moves on to the window's controls, and so does Shift+Tab without a focused link. HTML pages keep these keys |
| Enter in content | Follow the focused link (`viewer.follow-link`) |
| Alt+Shift+Left / Alt+Shift+Right in content, mouse Back / Forward | Back to the document a link was followed from, and forward again (`viewer.link-back`, `viewer.link-forward`); nothing happens without a document in that direction. This also works in HTML pages, whose Back and Forward buttons do the same |
| F5 / Ctrl+R in content | Read the shown file again from disk (`viewer.reload`): a text page reopens where it was, an HTML page at its top, keeping its Online and JavaScript choices. Also the HTML row's Reload button |
| Ctrl+F | Focus search |
| Enter in search / F3 | Next match |
| Shift+F3 | Previous match |
| Ctrl+A in content | Select the whole file in Text, Source and Markdown views (`viewer.select-page`); the status line says "Whole file selected" with the file's size. In the hex view, the rows shown. HTML pages select all in the page. A click, a drag, a Find match or another file replaces the selection; scrolling keeps it |
| Ctrl+C in content | Copy selected displayed text. With the whole file selected, the viewer reads the file in the background and copies its text whole, with progress in the status line, up to 64 MiB of text; a larger file is not copied, and its notice suggests copying the file itself (F5) |
| Arrows / PageUp / PageDown | Scroll content |
| Ctrl+Home / Ctrl+End | Beginning / end of the file (the first / last page of a PDF); an image larger than the window goes to its top / bottom (left / right edge when it is only wider) |
| Ctrl+W | Toggle word wrap in Text or Source mode |
| Ctrl+Plus / Ctrl+Equal | Zoom image in |
| Ctrl+Minus | Zoom image out |
| Ctrl+0 | Fit image without upscaling |
| Ctrl+1 | Actual image pixels (1:1) |
| Ctrl+Y | Type a zoom percentage for an image, PDF or HTML page (`viewer.zoom-to`, as Acrobat's Zoom To); also a click on the shown percentage. Enter applies, Escape keeps the zoom |
| Ctrl+PageUp / Ctrl+PageDown | Previous / next PDF document page; also available as page buttons |
| M in a video | Mute or unmute its sound, keeping the level (`viewer.mute`); also the speaker button beside the volume slider. Nothing happens for a video without sound or in other content. Keys go to the video's controls while one of them has the focus: Space or Enter presses a focused Play/Pause or speaker button, and on the timeline or the volume slider the arrow keys step it (Left/Down back, Right/Up forward; the volume by 5%) and Home/End go to its ends. Up and Down stay scrolling keys elsewhere, so the volume has no shortcut of its own |
| F7, Alt+Down, Alt+Right or N in file comparison | Next difference: changed lines, or the next region of differing bytes in the hex view; configurable as `compare.next-difference` |
| Shift+F7, Alt+Up, Alt+Left or P in file comparison | Previous difference (lines or byte regions); configurable as `compare.previous-difference` |
| Alt+Home / Alt+End in file comparison | First / last difference (lines or byte regions); configurable as `compare.first-difference` / `compare.last-difference`. Difference navigation stops at either end and aligns the changed rows in both sides. Editing controls retain their own keys. |

### Images and PDF documents

In image mode, Ctrl+wheel zooms around the pointer and dragging pans the image.
Text search and wrap controls are hidden for images. The same Close and previous/next
file actions apply to images and PDF documents. An image is drawn into a surface of
at most four megapixels and 4,096 pixels a side, so in a larger viewer it shows with
reduced detail; 1:1 keeps the physical size. PDF pages are always rendered at the
exact zoom shown, one pixel per screen pixel.

The zoom percentage in the footer (images and PDF documents, with one decimal) and in
the HTML row (whole percentages) is shown in the user's number format (`62,5%` in a
decimal-comma locale), as the field opens with it. It is
also a field: click it or press Ctrl+Y, type a value and press Enter. The value is a
percentage, with or without `%` (`150`, `150%`), and may have decimals in the user's
number format (`62,5` in a decimal-comma locale, `62.5` otherwise; a dot also works in
a comma locale that does not group digits with dots). Escape, or moving the focus
elsewhere, keeps the current zoom. A value outside the range (0.01–6400% for images
and PDF documents, 10–1000% for HTML) is applied at the nearest limit and a hint says
so, above the field (beside it in the HTML row); text without a number keeps the field
open with a hint. Fit and 1:1 work as before.

### HTML pages

The optional Chromium HTML viewer uses the same configurable Find, Copy, zoom,
navigation and previous/next-file actions. Escape always closes it. Space,
Backspace and Alt+Left/Right first reach Chromium, which keeps editable-field keys
inside the page. Outside an editable field the host resolves the current configured
action; delayed replies are ignored after viewer content loses focus. If that shortcut was removed, Space scrolls
the page; Shift+Space does the same in reverse when unassigned. AltGr input stays
with the page instead of triggering application shortcuts. Ctrl+0 resets HTML
layout zoom to 100%; the image viewer's fit behavior is unchanged. HTML file
cycling uses only Space, Backspace and Alt+Left/Right, which Chromium sees first. Other Previous/Next-file
remaps, such as Q or Ctrl+J, remain page input in HTML; they still work in the
other viewer modes. In an HTML page, Tab, Shift+Tab and Enter stay with the page;
Alt+Shift+Left/Right and the mouse's Back and Forward buttons go back and forward
through the documents opened from links, as in Markdown. As in a browser, Alt+Left and
Alt+Right do so too while there is a page in that direction (the Back and Forward buttons
are enabled then), after Chromium kept them for a focused text field; otherwise they keep
their file action. Backspace stays Previous file, since browsers no longer use it for
Back. F5 and Ctrl+R reload the page from its file instead of reaching Chromium.

### Links

Links follow one policy in every viewer: `#name` stays in the document, a local file
that a viewer shows opens in the same window, a folder opens in the active panel, any
other local file is shown in the active panel with the cursor on it, and `http`,
`https` and `mailto` links go to the system's default handler; a link never runs a
program. The five link actions and Reload are configurable viewer-content actions and do
not apply in comparisons; Alt+Left/Right still cycle files outside HTML pages.

### Viewer search and modes

Search retains ordinary text editing, clipboard keys, spaces and Backspace. Previous/next
file actions apply only to viewer content; every alias can be remapped or removed in
Options › Keyboard. Comparison windows have a separate content context, so their
Alt+Left / Alt+Right aliases continue to navigate differences. Main-window actions
do not dispatch inside viewer windows. Viewer mode is selected using the bottom
dropdown; Markdown search/copy uses rendered text, while Text mode uses literal source.

## Selection actions

Bulk selection runs through the backend and excludes `..`. Text editors retain
their ordinary Ctrl+A behavior. Ctrl+D deselects, unlike Total Commander, where it
opens the directory hotlist. During progressive listing, all/invert may wait for that
listing to complete; input remains responsive.

Select files and folders (`selection.all`) uses Ctrl+Shift+A. It replaces
selection with displayed top-level files and folders, clearing nested tree marks.
Invert files only (`selection.invert`, Ctrl+I) toggles displayed files, including
expanded descendants, while preserving folders. Invert top-level files and folders
(`selection.invert-all`, Ctrl+Shift+I) toggles only displayed top-level rows and
preserves nested marks. The synthetic parent and context-only ancestors remain
unselectable.

Select files only (`selection.files`, Ctrl+A) replaces selection with displayed
top-level files, clearing folder and nested marks. Deselect files and folders
(`selection.clear`, Ctrl+D) clears every mark, including nested rows. Ctrl+Shift+D
has no default assignment. These selection actions appear in F10 › Selection and
Options › Keyboard, including filtered lists and archives. The optional files-only
clear action (`selection.clear-files`) remains configurable without a default key
for compatibility with saved custom mappings. Directory
links/junctions count as folders; archive containers in native listings count as
files. Explicitly selecting nested rows by click or range remains available.
If saved settings already assign one of these keys to another action, that
assignment keeps working and the selection default stays unassigned.

## The parent row

The synthetic `..` row is navigation only: it cannot be renamed, viewed, edited,
copied, dragged, deleted or have its attributes changed. With no real selection,
the corresponding footer buttons are disabled. If real entries are selected,
Copy/Delete still target that captured selection even when the caret is on `..`;
Rename/View/Edit require a real focused entry. Creating a file/folder and pasting
into the current location remain available. This follows
[Salamander](GLOSSARY.md#salamander)'s parent-row rules (`src/mainwnd1.cpp:2748–2910`); backend target validation also rejects
paths ending in parent navigation instead of relying only on disabled UI controls.

Behavioral references are Salamander's `src/salamand.rc:61–102` and
`src/fileswn0.cpp:1220–1260` in [Open Salamander](https://github.com/OpenSalamander/salamander).
Ctrl+P, Ctrl+Enter and the folder-tab keys keep the meanings in
[Main window keys](#main-window-keys) where Salamander uses them differently.

## Panel warning icons

Panel warning/error icons are independent detail controls. With an icon focused,
Enter or Space opens its diagnostic report; Escape closes the report viewer.
This does not change file selection or retry/expand a folder. A failed tree
branch has separate Retry/Collapse controls. No global shortcut is assigned to them.

## Hidden files and selection by mask

**Show hidden files** (`panel.hidden-files`, default **Ctrl+H**, in the file list and
during filename search) is one choice for both panels. Hidden entries are names
starting with a dot on Linux and entries with the hidden attribute on Windows
(dot-names are ordinary there). Dot-named archive members are hidden on Linux too;
on Windows they carry no attribute and stay listed. Network
locations never hide anything. Hidden entries are shown by default. F10 › Panels
and navigation and each panel's View menu offer **Hide hidden files** or **Show
hidden files** for the current state, and Options › General has a **Show hidden
files** checkbox. The command saves the choice at once; the checkbox previews it
until OK. The saved choice applies again after a restart.

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

**Select by mask** (`selection.mask-select`, default **Ctrl+=**, **Num +** and
**Ctrl++**) and **Deselect by mask** (`selection.mask-deselect`, **Ctrl+−** and
**Num −**) open a small dialog, **Select files** or **Deselect files**: the mask field
starts with the last mask (`*` at first) and Enter or OK applies it, Escape or Cancel
closes it. Masks use the panel filter's Masks syntax: `*` any text, `?` one
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
AZERTY), Ctrl+− opens Hot Path 6. There, use the keypad, F10 › Selection or a key
of your own in Options › Keyboard. Ctrl+Shift+= still adds the current folder as a
Hot Path. Menus show Ctrl+= and Ctrl+−, and their tooltips list every key. Options › Keyboard records the keypad keys as `NumpadAdd`, `NumpadSubtract`
and `NumpadMultiply` and can move the three actions to other keys. A saved
assignment of Ctrl+H, a keypad key, Ctrl+= or Ctrl+− to another action keeps
working; only that default then stays unassigned. All three are also in F10 →
Selection.

## Attributes, times and case editor

On the Times page of the Ctrl+F2 editor, the Date, Time and UTC offset fields
accept typing. **Tab** moves through a row without ticking it; a click into a field
or onto a picker button, typing, or any other change of a value ticks the row.
**Up/Down** change the part under the cursor (year, month or day; hour, minute or
second; offset hours or quarter hours, from UTC−12 to UTC+14), and **Alt+Down** opens
the field's calendar or time picker. In the calendar,
**Left/Right** move one day, **Up/Down** one week, **Page Up/Page Down** one month
and **Ctrl+Page Up/Ctrl+Page Down** one year; **Home/End** go to the first or last
day of the month; **Enter** or **Space** chooses the date and **Escape** closes only
the calendar. In the time picker, **Left/Right** choose hours, minutes, seconds or
AM/PM, and **Tab/Shift+Tab** go round them. **Up/Down** change the chosen column by
one and **Page Up/Page Down** by ten, wrapping within the column (23 → 00 leaves
the date as it is); **Home/End** set its first or last value; the column's ▲ and ▼
and the mouse wheel over it step it too. Digits type the column: two digits, or one
that no second digit could follow (3 to 9 for hours, 2 to 9 for the hours of a
12-hour clock, 6 to 9 for minutes and seconds), complete it and move to the next
column; the AM/PM column takes the first letter of either marker. The time field
follows at once. **Enter** or **Space** closes the picker and keeps the time;
**Escape** closes it and restores the time, and the row's tick, as they were when it
opened. **Space** or **Enter** on a row's **Now** button fills its fields with the
current local date, time and offset and ticks the row; Apply sets exactly that
moment. Outside a picker, **Escape** closes the editor without applying.

## Find result actions

Within the focused Find result list, **View** (**F3**), plain-text view (**Alt+F3**)
and **Edit** (**F4**) use the current configured file-action bindings. Their button
hints update from the same map. **Enter** and a double-click run **Go to file**;
**Open** is an explicit button because the system association may execute the file.

The list selects several results, as the Checksums results do:

- a click selects one result, **Ctrl+click** toggles one, **Shift+click** selects the
  range from the anchor and **Ctrl+Shift+click** adds that range;
- **Up/Down**, **Page Up/Page Down**, **Home** and **End** move the cursor and select
  its result; with **Shift** they select the range, with **Ctrl** they move the cursor
  alone; **Left/Right** scroll the list sideways;
- **Space** toggles the cursor's result, **Insert** toggles it and moves down,
  **Ctrl+A** selects every result and **Ctrl+D** none;
- **Ctrl+C** copies the full paths of the selection (or of the cursor's result when it
  is not selected);
- the context menu (right click, **Shift+F10** or the **Menu** key) offers Go to file,
  View, Edit, Open, Copy names, Copy full paths, Copy row details, Select all and
  Deselect all; its copy entries take the whole selection when it holds the menu's
  row.

F3, F4, Enter and the footer actions act on the cursor's result only, as in a panel;
Feed to panel marks the selected results in the panel's list, where F5, F6 and F8 act
on them. The status line counts the selected results and the size of their files.
Footer actions that do not fit beside **Close** are in its **More…** button: **Tab**
reaches it, **Space** or **Enter** opens its menu, **Up/Down** choose, **Enter** or
**Space** runs and **Escape** closes it.

**Tab** moves from the result list to its column headings (Name, Extension, Path,
Size, Modified), each shown with a focus frame and scrolled into view; **Enter** or
**Space** sorts by the focused heading, as a click does (repeat to reverse).
Actions require a regular native result and fresh identity validation; a stopped
search retains its results, while a closed or restarted search revokes pending
actions. See the [Find contract](developer/FIND.md).

## Panel view and sort menus

Both panels show the current configured bindings beside Brief, Detailed,
Thumbnails, Show (or Hide) hidden files, Name, Size, Modified and Extension in their
View and Sort menus; the View menu lists the views in shortcut order (Alt+2, Alt+3,
Alt+5). Reverse order shows the binding of the current sort column,
which toggles its direction. Rebinding or disabling a shortcut updates these hints
immediately, including an already open menu. In a compact layout the closed buttons
show only their icons.

## Key-combination syntax

A shortcut is one key combination (a chord): any modifiers, then one key, joined by
`+`, for example `Ctrl+Shift+F10`. Options › Keyboard records the keys you press; the
settings file stores the same text, as [Action IDs](#action-ids) shows. The parts:

| Part | Accepted spellings | Stored as |
| --- | --- | --- |
| Control | `Ctrl`, `Control` | `Ctrl` |
| Alt | `Alt` | `Alt` |
| Shift | `Shift` | `Shift` |
| Meta (the Windows or Super key) | `Meta`, `Super`, `Win` | `Meta` |
| Named key | `Enter` (`Return`), `Escape` (`Esc`), `Tab`, `Space`, `Backspace`, `Delete` (`Del`), `Insert` (`Ins`), `Home`, `End`, `PageUp` (`PgUp`), `PageDown` (`PgDown`, `PgDn`), `Up`, `Down`, `Left`, `Right` (also `UpArrow`, `ArrowUp`, `↑` and the like) | The first spelling |
| Function key | `F1` to `F24` | `F1` to `F24` |
| Number-row digit | `Digit0` to `Digit9`, or the digit itself | `Digit0` to `Digit9`: the physical key, whatever the keyboard layout types |
| Numeric keypad | `NumpadAdd` (`NumPlus`, `KeypadPlus`), `NumpadSubtract` (`NumMinus`, `KeypadMinus`), `NumpadMultiply` (`NumMultiply`, `NumStar`, `KeypadMultiply`) | The first spelling |
| Punctuation key | `Plus`, `Minus` (`-`), `Equal` (`Equals`, `=`), `Backslash` (`\`), `Slash` (`/`), `BracketLeft` (`[`), `BracketRight` (`]`), `Comma` (`,`), `Period` (`.`), `Semicolon` (`;`), `Quote` (`'`), `Backquote` | The first spelling |
| Any other key | One character, such as `A` or `ž` | ASCII letters in upper case, other characters unchanged |

- Names ignore letter case and the spaces around each part. The key comes last; each
  modifier may appear once. Write the plus key as `Plus`, since `+` separates the parts.
- A chord has 1 to 128 bytes and no control characters. An action has at most four
  chords, and at most 256 actions can have changed keys.
- Refused text names the problem: `Unknown shortcut modifier: …`,
  `Repeated shortcut modifier: …` or `Unknown shortcut key: …. Use a named key or one
  character.`

### Scopes

Each action works in one scope; a chord can mean different actions in scopes that do not
overlap, and an assignment that overlaps another action's scope opens the **Replace
shortcut** question ([Changing shortcuts](#changing-shortcuts-in-options--keyboard)).

| Scope | Where the action's keys work |
| --- | --- |
| File list outside filename search | The focused file list while no filename search is active |
| Filename search | The file list while a filename search is active |
| File list and filename search | Both of the above |
| File list, filename search and command editor | Both of the above and the command editor |
| Command editor | The command line under the panels |
| Application window | The main window, wherever the keyboard focus is in it |
| Volume choices popup | The drive dropdown, opened with Alt+F1, Alt+F2 or a click |
| Viewer content | The content of a viewer window |
| Comparison content | The content of a file comparison window |
| Viewer and comparison content | Both of the above |
| Viewer and comparison windows, including search | Viewer and comparison windows, also while their search field has the focus |

Some chords work only outside filename search, although their action's scope is
wider: plain Backspace as the parent folder, plain Escape as clearing comparison marks,
plain digits as Hot Paths, and the clipboard chords (Ctrl or Meta with C, X or V, and
Ctrl+Insert or Shift+Insert) in any file-list action, since filename search edits its
text with them.

These chords are reserved, and Options › Keyboard refuses them:

- `Ctrl+Space`, for the input method: `Ctrl+Space is reserved for the input method (IME).`
- In filename search, the clipboard chords, which edit the search text.
- In the volume choices popup, unmodified Enter, Escape, Tab, arrows, Home, End, Page Up
  and Page Down, which navigate it.
- In scopes that include the command editor, the rest of the main window or a viewer's
  search field: text-editing chords (Ctrl or Meta with A, C, X, V, Z, Y, Insert,
  Backspace, Delete, an arrow, Home, End, Page Up or Page Down), unmodified printable
  and editing keys, and unmodified Enter or Escape, except the command editor's own Enter
  and Escape and the viewer's Escape: `… is reserved for native text editing in …`.

## Action IDs

Every configurable action has a stable ID. The settings file
(`preferences.json`, see [settings files](SETTINGS_FILES.md)) keeps only the actions
whose keys you changed, in its `shortcuts` list:

```json
"shortcuts": [
  { "action": "files.copy-to", "keys": ["F5", "Ctrl+Alt+Y"] },
  { "action": "panel.hidden-files", "keys": [] }
]
```

| Field | Type | Meaning |
| --- | --- | --- |
| `action` | String, at most 96 bytes | An ID from the table below or `user-command.<id>`; an unknown ID makes the settings file invalid |
| `keys` | List of 0 to 4 chords | Replaces every default key of the action; an empty list disables it |

The table lists each action with its name in Options › Keyboard, its default keys as
stored and its [scope](#scopes). "None" means that the action has no default key.

| Action ID | Name | Default keys | Scope |
| --- | --- | --- | --- |
| `panel.as-other` | Open other panel's folder | `Period` | Volume choices popup |
| `panel.switch` | Switch active panel | `Tab`, `Shift+Tab` | File list and filename search |
| `panel.open` | Open focused file or folder | `Enter` | File list and filename search |
| `panel.up` | Previous row or search match | `Up` | File list and filename search |
| `panel.down` | Next row or search match | `Down` | File list and filename search |
| `panel.previous-selected` | Previous selected file or folder | `Alt+Up` | File list outside filename search |
| `panel.next-selected` | Next selected file or folder | `Alt+Down` | File list outside filename search |
| `panel.page-up` | Move one page up | `PageUp` | File list and filename search |
| `panel.page-down` | Move one page down | `PageDown` | File list and filename search |
| `panel.first` | First row | `Home` | File list and filename search |
| `panel.last` | Last row | `End` | File list and filename search |
| `panel.left` | Previous file column or grid cell | `Left` | File list outside filename search |
| `panel.right` | Next file column or grid cell | `Right` | File list outside filename search |
| `panel.parent` | Parent folder | `Backspace`, `Ctrl+PageUp` | File list and filename search |
| `panel.root` | Filesystem root | `Shift+Backspace`, `Ctrl+Backspace`, `Ctrl+Backslash` | File list and filename search |
| `panel.enter-folder` | Enter focused folder without opening files | `Ctrl+PageDown` | File list and filename search |
| `panel.back` | Previous folder in history | `Alt+Left` | File list and filename search |
| `panel.forward` | Next folder in history | `Alt+Right` | File list and filename search |
| `panel.refresh` | Refresh active panel | `Ctrl+R`, `Ctrl+F9` | File list and filename search |
| `panel.path` | Focus path editor | `Ctrl+G` | File list and filename search |
| `panel.swap` | Swap complete panels | `Ctrl+U` | File list and filename search |
| `panel.compare` | Compare panel directories | `Ctrl+F10` | File list and filename search |
| `panel.compare-clear` | Clear directory comparison marks | `Escape`, `Ctrl+Shift+F11` | File list and filename search |
| `panel.zoom` | Zoom active panel | `Ctrl+F11` | File list and filename search |
| `files.view` | View focused file | `F3` | File list and filename search |
| `files.find` | Find files and text | `Ctrl+F`, `Alt+F7` | File list and filename search |
| `files.checksums` | Calculate checksums | `Ctrl+F8` | File list and filename search |
| `files.compare` | Compare two files | `Ctrl+Shift+C` | File list and filename search |
| `files.view-basic` | View as plain text | `Alt+F3` | File list and filename search |
| `files.edit` | Edit focused file externally | `F4` | File list and filename search |
| `viewer.close` | Close viewer | `Escape` | Viewer and comparison windows, including search |
| `compare.next-difference` | Next comparison difference | `F7`, `Alt+Down`, `Alt+Right`, `N` | Comparison content |
| `compare.previous-difference` | Previous comparison difference | `Shift+F7`, `Alt+Up`, `Alt+Left`, `P` | Comparison content |
| `compare.first-difference` | First comparison difference | `Alt+Home` | Comparison content |
| `compare.last-difference` | Last comparison difference | `Alt+End` | Comparison content |
| `viewer.next-file` | View next captured file | `Space`, `Alt+Right` | Viewer content |
| `viewer.previous-file` | View previous captured file | `Backspace`, `Alt+Left` | Viewer content |
| `viewer.next-link` | Focus the next link in the viewed document | `Tab` | Viewer content |
| `viewer.previous-link` | Focus the previous link in the viewed document | `Shift+Tab` | Viewer content |
| `viewer.follow-link` | Follow the focused link | `Enter` | Viewer content |
| `viewer.link-back` | Back to the document a link was followed from | `Alt+Shift+Left` | Viewer content |
| `viewer.link-forward` | Forward to the document a link opened | `Alt+Shift+Right` | Viewer content |
| `viewer.reload` | Reload the viewed file | `F5`, `Ctrl+R` | Viewer content |
| `viewer.mute` | Mute or unmute the video's sound | `M` | Viewer content |
| `viewer.find` | Find in viewed file | `Ctrl+F` | Viewer and comparison windows, including search |
| `viewer.wrap` | Wrap viewer text | `Ctrl+W` | Viewer and comparison windows, including search |
| `viewer.zoom-in` | Zoom viewed image in | `Ctrl+Plus`, `Ctrl+Equal` | Viewer and comparison content |
| `viewer.zoom-out` | Zoom viewed image out | `Ctrl+Minus` | Viewer and comparison content |
| `viewer.fit` | Fit viewed image to window | `Ctrl+Digit0` | Viewer and comparison content |
| `viewer.actual-pixels` | View actual image pixels | `Ctrl+Digit1` | Viewer and comparison content |
| `viewer.zoom-to` | Type a zoom percentage | `Ctrl+Y` | Viewer content |
| `viewer.find-next` | Next match in viewed file | `F3` | Viewer and comparison windows, including search |
| `viewer.find-previous` | Previous match in viewed file | `Shift+F3` | Viewer and comparison windows, including search |
| `viewer.copy` | Copy viewer selection | `Ctrl+C` | Viewer and comparison content |
| `viewer.select-page` | Select whole viewed file | `Ctrl+A` | Viewer and comparison content |
| `viewer.up` | Scroll viewer up | `Up` | Viewer and comparison content |
| `viewer.down` | Scroll viewer down | `Down` | Viewer and comparison content |
| `viewer.left` | Scroll viewer left | `Left` | Viewer and comparison content |
| `viewer.right` | Scroll viewer right | `Right` | Viewer and comparison content |
| `viewer.page-up` | Previous viewer screen | `PageUp` | Viewer and comparison content |
| `viewer.page-down` | Next viewer screen | `PageDown` | Viewer and comparison content |
| `viewer.first` | Beginning of viewed file | `Ctrl+Home` | Viewer and comparison content |
| `viewer.document-previous` | Previous document page | `Ctrl+PageUp` | Viewer and comparison content |
| `viewer.document-next` | Next document page | `Ctrl+PageDown` | Viewer and comparison content |
| `viewer.last` | End of viewed file | `Ctrl+End` | Viewer and comparison content |
| `panel.filter` | Edit panel filter | `Ctrl+F12` | File list and filename search |
| `panel.drive-info` | Show drive information | `Ctrl+F1`, `Ctrl+L` | File list and filename search |
| `panel.folder-sizes` | Calculate or cancel folder sizes | `Ctrl+Shift+S`, `Ctrl+Shift+F10` | File list and filename search |
| `selection.toggle-size` | Toggle selection and calculate focused folder size | `Space` | File list outside filename search |
| `selection.toggle` | Toggle selection and advance | `Insert` | File list and filename search |
| `selection.all` | Select files and folders | `Ctrl+Shift+A` | File list and filename search |
| `files.make-list` | Make File List | `Ctrl+M` | File list and filename search |
| `selection.files` | Select files only | `Ctrl+A` | File list and filename search |
| `selection.clear-files` | Deselect files only | None | File list and filename search |
| `selection.clear` | Deselect files and folders | `Ctrl+D` | File list and filename search |
| `selection.invert` | Invert files only | `Ctrl+I` | File list and filename search |
| `selection.invert-all` | Invert top-level files and folders | `Ctrl+Shift+I` | File list and filename search |
| `selection.mask-select` | Select by mask… | `Ctrl+Equal`, `NumpadAdd`, `Ctrl+Plus` | File list outside filename search |
| `selection.mask-deselect` | Deselect by mask… | `Ctrl+Minus`, `NumpadSubtract` | File list outside filename search |
| `selection.mask-invert` | Invert selection (folders as in Select by mask) | `NumpadMultiply` | File list outside filename search |
| `panel.hidden-files` | Show or hide hidden files | `Ctrl+H` | File list and filename search |
| `sort.name` | Sort by name | `Ctrl+F3` | File list and filename search |
| `sort.extension` | Sort by extension | `Ctrl+F4` | File list and filename search |
| `sort.modified` | Sort by modification time | `Ctrl+F5` | File list and filename search |
| `sort.size` | Sort by size | `Ctrl+F6` | File list and filename search |
| `search.clear` | Leave filename search | `Escape` | Filename search |
| `search.erase` | Erase previous search grapheme | `Backspace` | Filename search |
| `files.rename` | Rename focused item | `F2` | File list and filename search |
| `files.attributes` | Edit attributes and times | `Ctrl+F2` | File list and filename search |
| `files.case` | Change filename case | `Ctrl+F7` | File list and filename search |
| `files.create-file` | Create empty file | `Shift+F4` | File list and filename search |
| `files.create-directory` | Create folder | `F7` | File list and filename search |
| `files.trash` | Move selection to Trash | `Delete`, `F8` | File list and filename search |
| `files.delete` | Permanently delete selection | `Shift+Delete`, `Shift+F8` | File list and filename search |
| `files.properties` | Properties of selected or focused items | `Alt+Enter`, `Ctrl+Alt+Enter` | File list and filename search |
| `files.copy-to` | Copy selected files to folder | `F5` | File list and filename search |
| `files.copy` | Copy files to clipboard | `Ctrl+C` | File list outside filename search |
| `files.move-to` | Move selected files to folder | `F6` | File list and filename search |
| `files.cut` | Cut native files to clipboard | `Ctrl+X` | File list outside filename search |
| `files.paste` | Paste files into active folder | `Ctrl+V` | File list outside filename search |
| `tab.new` | New tab | `Ctrl+T` | File list and filename search |
| `tab.close` | Close active tab | `Ctrl+W` | File list and filename search |
| `tab.previous` | Previous tab | `Ctrl+Shift+Tab` | File list and filename search |
| `bookmark.add` | Bookmark current folder | `Ctrl+Shift+Plus`, `Ctrl+Shift+Equal` | File list and filename search |
| `bookmark.assign.10` | Assign current folder to Hot Path 10 | `Ctrl+Shift+Digit0` | File list and filename search |
| `command.focus` | Toggle command editor focus | `Ctrl+Tab` | Application window |
| `command.insert-name` | Append focused basename to command | `Ctrl+Enter` | File list, filename search and command editor |
| `command.insert-full-path` | Append focused full path to command | `Ctrl+Shift+P` | File list, filename search and command editor |
| `command.insert-path` | Insert active panel path into command | `Ctrl+P` | File list, filename search and command editor |
| `command.insert-left-path` | Insert left panel path into command | `Ctrl+BracketLeft`, `Ctrl+Shift+BracketLeft` | Application window |
| `command.insert-right-path` | Insert right panel path into command | `Ctrl+BracketRight`, `Ctrl+Shift+BracketRight` | Application window |
| `command.history-older` | Recall older command | `Alt+Up` | Command editor |
| `command.history-newer` | Recall newer command | `Alt+Down` | Command editor |
| `command.execute` | Execute command draft | `Enter` | Command editor |
| `command.focus-files` | Return to file list | `Escape` | Command editor |
| `command.open-terminal-elevated` | Open terminal as administrator (Windows) / Open root terminal (sudo) (Linux) | None | Application window |
| `drive.left` | Choose left panel volume | `Alt+F1` | Application window |
| `drive.right` | Choose right panel volume | `Alt+F2` | Application window |
| `app.help` | Help | `F1` | Application window |
| `app.user-menu` | User menu | `F9` | Application window |
| `app.main-menu` | Main menu | `F10` | Application window |
| `app.options` | Options | None | Application window |
| `app.jobs` | Operation jobs | None | Application window |
| `app.about` | About Midnight Explorer | None | Application window |
| `app.custom-commands` | Configure Custom Commands | None | Application window |
| `bookmark.manage` | Manage Hot Paths | None | File list and filename search |
| `view.brief` | Brief panel view | `Alt+Digit2` | File list and filename search |
| `view.detailed` | Detailed panel view | `Alt+Digit3` | File list and filename search |
| `view.thumbnails` | Thumbnail panel view | `Alt+Digit5` | File list and filename search |
| `app.exit` | Exit application | `Alt+F4` | Application window |
| `network.connect` | Connect network drive | `F11` | Application window |
| `network.disconnect` | Disconnect network drive | `F12` | Application window |
| `tab.1` to `tab.9` | Switch to tab 1 to 9 | `Ctrl+Shift+Digit1` to `Ctrl+Shift+Digit9` | File list and filename search |
| `bookmark.open.1` to `bookmark.open.10` | Open Hot Path 1 to 10 | `DigitN`, `Ctrl+DigitN`, `Shift+DigitN`, with `Digit0` for Hot Path 10 | File list and filename search |
| `bookmark.other.1` to `bookmark.other.10` | Open Hot Path 1 to 10 in other panel | `Ctrl+Alt+DigitN`, `Alt+Shift+DigitN`, with `Digit0` for Hot Path 10 | File list and filename search |
| `drive.a` to `drive.z` | Switch to drive A to Z | `Shift+A` to `Shift+Z` | File list and filename search |
| `user-command.<id>` | The Custom Command's name | None | File list outside filename search |

The catalog is `shortcut_catalog` in
[`crates/midnight-core/src/shortcuts/catalog.rs`](../crates/midnight-core/src/shortcuts/catalog.rs);
the syntax and the reserved chords are in
[`crates/midnight-core/src/shortcuts.rs`](../crates/midnight-core/src/shortcuts.rs).
