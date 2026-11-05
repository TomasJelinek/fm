# Keyboard shortcuts — FM 0.1

These actions are implemented. File navigation, tab and bookmark shortcuts apply
in the focused file list, preserving typing in the path/command/options editors.
Native numbered tabs and hot paths use physical digit keys across keyboard layouts
and do not intercept RightAlt/AltGr. Number 0 refers to bookmark slot 10.

| Keys | Current action |
| --- | --- |
| Tab / Shift+Tab in files | Switch the active pane |
| ↑ / ↓, PageUp / PageDown, Home / End | Move the focused row |
| Plain typing in files | Start Unicode case-insensitive filename-prefix search |
| ↑ / ↓ during search | Visit previous / next matching name without wrapping |
| Escape during search | Leave search and restore the complete listing |
| Backspace during search | Remove one extended grapheme; an empty query exits |
| Enter | Enter a folder or open the focused file through its host association |
| Backspace / Ctrl+PageUp | Parent folder |
| Alt+← / Alt+→ | Back / forward history |
| Ctrl+Backspace / Shift+Backspace / Ctrl+Backslash | Native filesystem root |
| Ctrl+PageDown | Enter a known folder; never launch a regular file |
| Ctrl+G | Focus the path editor |
| Alt+F1 / Alt+F2 | Open left / right volume choices |
| Shift+drive letter | Switch volume on Windows, using its remembered path |
| Ctrl+F1 / Ctrl+L | Read-only cached drive information |
| Space | Toggle selection, advance one row and calculate the focused directory's size on the backend |
| Insert | Toggle selection and move to the next row |
| Ctrl+A / Ctrl+D / Ctrl+I | Select all / clear / invert selection in the focused file pane |
| Ctrl+R / Ctrl+F9 | Refresh |
| Ctrl+F3 / Ctrl+F4 / Ctrl+F5 / Ctrl+F6 | Sort by name / type-extension / modified time / size |
| Ctrl+Shift+S / Ctrl+Shift+F10 | Start or cancel all-folder size calculation in the active pane |
| Alt+Enter / Ctrl+Alt+Enter | Properties of selected items, or focused item |
| Ctrl+T | New tab at the current folder |
| Ctrl+W | Close active tab; keep the last tab |
| Ctrl+Tab / Ctrl+Shift+Tab in files | Next / previous tab in that pane |
| Ctrl+Shift+1…9 | Switch to that numbered tab in the focused pane; missing tabs do nothing |
| Ctrl+Shift+0 | Assign current folder to bookmark slot 10; other slots use the Hot Paths manager |
| Ctrl+1…9/0 or Shift+1…9/0 | Open the numbered bookmark in the focused pane |
| Ctrl+Alt+number or Shift+Alt+number | Open the numbered bookmark in the other pane |
| Ctrl+Shift+Plus | Add the current folder as an unnumbered bookmark |
| Ctrl+Enter in files or command editor | Append the focused basename to the end of the command draft |
| Ctrl+P in files or command editor | Insert the active pane path at the command selection |
| Ctrl+[ / Ctrl+] | Insert the left / right pane path at the command selection |
| Ctrl+Space | Available to the input method (IME); Ctrl+P inserts the path |
| Enter in command editor | Explicitly submit the command to the selected system terminal |
| Escape / Ctrl+Tab in command editor | Return to the active file list |
| F1 | Help |
| F9 | User menu; no custom command execution is configured in this milestone |
| F10 | Main menu, including Options |
| Alt+F4 | Exit |

Inserted paths/names use the draft's fixed shell grammar: simple paths remain
bare; special characters use escaped double quotes, never surrounding apostrophes. Insertion
never submits a command. Editor word deletion and ordinary text editing retain
their standard behavior. Menus also support arrows, Enter and Escape; unclear
controls have tooltips and accessible names.

Mouse: left click focuses, Ctrl+left click toggles selection, unmodified left
double-click activates. Clicking a column header switches sorting direction.
Insert and Space clamp at the last row; `..` is never selected or measured.
Wheel scrolling animates in pixels and settles on a full row when idle; native
touch/trackpad gestures retain their own inertia.

The drive toolbar includes Hot Paths; all saved paths also appear in the drive
dropdown. Back/Forward arrows have history dropdowns. Tabs sit below the drive
toolbar. Hovering a path segment highlights its prefix; clicking opens that raw
ancestor. The details strip below each panel copies a field's displayed value
when clicked (filename, exact bytes, date/time, attributes or Unix permissions).

The command directory ends with `>`. Command history recalls a draft without
running it; only Enter/Run submits it. Options has Panel, Appearance and Command
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

The bottom F2 Rename, F3 View, F4 Edit, F5 Copy, F6 Move, F7 New folder and
F8 Delete actions are reserved for later stages and are currently disabled.
Copy/move/delete queues, overwrite decisions and mandatory destination checksum
verification remain in the design. Other unlisted Salamander/Total Commander
shortcuts, such as Shift+F9 for a bookmark menu, are not implemented.

Bulk selection runs through the backend and excludes `..`. Text editors retain
their ordinary Ctrl+A behavior. The requested Ctrl+D takes precedence over
Total Commander's bookmark meaning. During progressive listing, all/invert may
wait for that listing to complete; input remains responsive.

Behavioral references: local Salamander `src/salamand.rc:61–102` and
`src/fileswn0.cpp:1220–1260`. Requested Ctrl+P/Ctrl+Enter behavior and folder-tab
navigation take precedence over an upstream shortcut with a different meaning.
