# Panels and files: acceptance coverage

This record lists every user-visible feature of the file panels (the inventory), the
acceptance scenario and check that exercises it through the real main window, and the
status of that check in the latest recorded run. The inventory follows the panel code
(`crates/midnight-ui/ui/file-pane.slint` and the controls it uses, `main.slint`,
`shared-toolbar.slint`, the embedded dialogs, `crates/midnight-ui/src/presenter/`), the
shortcut catalog (`crates/midnight-core/src/shortcuts/catalog.rs`) and the documents
[KEYBOARD_SHORTCUTS.md](../../KEYBOARD_SHORTCUTS.md), [FEATURE_STATUS.md](../../FEATURE_STATUS.md),
[MAKE_FILE_LIST.md](../../MAKE_FILE_LIST.md), [TREE_PANEL.md](../TREE_PANEL.md),
[SELECTION.md](../SELECTION.md) and [PANEL_DIAGNOSTICS.md](../PANEL_DIAGNOSTICS.md). The
suite itself is described in [ACCEPTANCE.md](../ACCEPTANCE.md).

Statuses:

- **PASS**: the check ran and held.
- **FAIL**: the check ran and found a product bug, named by its finding (P-n below).
- **PENDING**: not verified yet, for the reason given.
- **SKIP**: the acceptance environment or platform cannot provide what the check needs.

Every default key of the catalog is also pressed once in its context by the separate
`shortcuts_catalog` sweep (Options and tools record); the panel scenarios here check the
exact effect on rows, cursor, marks, sort and view state, the pointer paths and the
combinations. Helpers of these scenarios are in
`tests/acceptance/scenarios/panels_support.rs`.

## Findings

| Finding | Product bug | Checks |
| --- | --- | --- |
| P-1 | After a dialog opened from a toolbar or drive-bar button (Attributes, Make File List, Properties, the funnel, Hot Paths › Manage) closed with Escape or ×, keyboard focus stayed on that button: arrow keys did nothing until the list was clicked. Fixed (focus returns to the active list); the check passes. | D-13 |
| P-2 | After the Hot Paths manager opened from the toolbar's Hot Paths menu closed, the first click on a file row moved the cursor but did not give the list keyboard focus. Fixed; the check passes. | D-14 |
| P-3 | At the narrowest split (286 px) seven-character names were elided (46 of 55 px) while Size, Modified and Time kept about 120 px: the name did not get the width first. Fixed; the check passes. | V-12 |
| P-4 | An empty folder showed only `..`: the empty-folder message never appeared because `..` counts as a row, and listings emptied by hidden files or a filter did not say so. Decided and fixed: the message appears below `..`, with a distinct message when everything is hidden or filtered. | N-35, N-36 |
| P-5 | Fixed; the check passes. Select by mask accepted the invalid masks `a\|b\|c` (two `\|`) and `*.txt\|` (nothing after `\|`): the dialog closes, nothing is marked and no reason was shown anywhere. | S-07 |
| P-6 | Inside an archive the path bar is cut at its end ("…/staged/r…"), hiding the archive's name and the folders inside it, while native folders show whole segments with one middle marker. Fixed; the check passes (Linux run `20261121-144246`). | N-45 |

## Startup and navigation

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| N-01 | Both panels open on their folders and list them | `startup` | PASS |
| N-02 | Enter and Backspace into every fixture folder; each listing equals the disk | `navigation` | PASS |
| N-03 | Backspace returns the cursor to the folder left | `navigation`; `panels_tree_keys` › Enter on a nested folder | PASS |
| N-04 | Alt+Left and Alt+Right history | `files_navigation` | PASS |
| N-05 | Toolbar Back and Forward: disabled without history, step through it | `panels_paths` › toolbar | PASS |
| N-06 | Back and Forward history menus list the folders newest first and jump several steps | `panels_paths` › history menus | PASS |
| N-07 | Toolbar Parent folder, cursor on the folder left, keyboard back in the list | `panels_paths` › toolbar | PASS |
| N-08 | Toolbar Refresh lists again and keeps the cursor | `panels_paths` › toolbar | PASS |
| N-09 | Ctrl+R lists an entry made behind the panel's back, keeps cursor and marks | `files_navigation` | PASS |
| N-10 | Ctrl+PageUp goes to the parent with the cursor on the folder left | `panels_paths` › keys | PASS |
| N-11 | Ctrl+PageDown enters a folder and never opens a file | `panels_paths` › keys | PASS |
| N-12 | Ctrl+Backspace, Shift+Backspace and Ctrl+Backslash open the filesystem root | `files_navigation`; `panels_paths` › keys | PASS |
| N-44 | Shift+Backspace goes to the focused panel's root: left and right deep folders (focused by Tab and by a click), after a search ended with Escape, in a second tab (the first stays), and inside an archive to the archive's own root | `panels_root_keys` | PASS |
| N-45 | Inside an archive the path bar shows the archive's and the current folder's names whole (a native folder is the control) | `panels_root_keys` › path bar | PASS (P-6 fixed) |
| N-13 | A double click opens a folder; on `..` goes up with the cursor on the folder left | `panels_paths` › keys | PASS |
| N-14 | Tab and Shift+Tab switch the active panel and its keyboard focus | `panels_paths` › keys | PASS |
| N-15 | Ctrl+G opens a typed path | `files_navigation`; `panels_paths` | PASS |
| N-16 | Ctrl+G to a missing folder is reported with the whole path | `files_navigation`; `panels_notices` › missing folder | PASS |
| N-17 | A path segment click opens that ancestor | `files_navigation` | PASS |
| N-18 | Hovering a path segment highlights its prefix | `panels_paths` › path bar | PASS |
| N-19 | Right-clicking a segment copies its raw prefix without navigating | `panels_paths` › path bar (clipboard content) | PASS |
| N-20 | Pasting into the path editor appends a separator | `panels_paths` › path bar | PASS |
| N-21 | A long path shows whole segments, one middle marker and its last folder; the marker opens the whole path for editing | `panels_paths` › long path | PASS |
| N-22 | The blank space after the path opens the editor (at least 32 px); Enter opens; Escape cancels and returns to the list | `panels_paths` › path bar | PASS |
| N-23 | Alt+F1 and Alt+F2 open each panel's drive menu; Escape closes it without navigating | `panels_paths` › drives | PASS |
| N-24 | `.` in the drive menu opens the other panel's folder (As Other Panel) | `panels_paths` › drives | PASS |
| N-25 | The drive menu's filesystem root by pointer | `panels_paths` › drives | PASS |
| N-26 | The drive bar's quick location buttons | none | SKIP: the acceptance machine shows a single volume and no special location, whose button keeps the current folder (each volume remembers its folder) |
| N-27 | Ctrl+U swaps the panels and back, folders kept | `files_navigation` | PASS |
| N-28 | Ctrl+U moves each panel's tabs with it | `panels_tabs` › swap | PASS |
| N-29 | Alt+Enter Properties of a folder: type, permissions, time and its contents counted as on disk | `files_navigation` › Properties | PASS |
| N-29a | Properties' owner, group, created and accessed lines | none | SKIP: they come from the desktop service's property details, which the headless harness does not install (it would also launch programs) |
| N-30 | Alt+Enter Properties of a file: type, size, permissions, time | `panels_dialogs` › after the cancelled rounds | PASS |
| N-31 | Properties of several marked items: "3 selected items", their kinds and the files' bytes | `panels_dialogs` › afterwards | PASS |
| N-32 | Ctrl+F1 drive information: opens, shows the folder's volume, closes | `tools_help_drive_info` (Options and tools); `panels_dialogs` (open and close) | PASS |
| N-33 | Start with `--left`/`--right` and a saved tab session | `startup_paths` (harness owner) | see that scenario |
| N-34 | Startup listing shows no notice for an unreadable child folder | `panels_notices` › startup | PASS |
| N-35 | An empty folder says so below `..`, and `..` still goes up | `panels_paths` › empty states | PASS |
| N-36 | A listing emptied by hidden files or by the panel filter says why | `panels_paths` › empty states | PASS |
| N-37 | A quick-filtered search without matches says "No matching names"; Escape restores | `panels_paths` › quick filter | PASS |
| N-38 | The details strip copies the clicked field: name, Ctrl+click the whole path, size as shown | `panels_paths` › details strip | PASS |
| N-39 | Free space at the right of the details strip | none yet | PENDING: no check written yet |
| N-40 | Loading notice only after 500 ms; refreshes keep rows without it | `files_busy_folder` (no loading state while busy) | PASS |
| N-41 | Shift+letter switches the drive | not on Linux | SKIP: Windows only |
| N-42 | F11/F12 network drive dialogs | not on Linux | SKIP: Windows only; the footer hides them |
| N-43 | The Trash location opens the desktop Trash | not installed headless | SKIP: the harness installs no desktop opener |

## Tabs

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| T-01 | Ctrl+T, Ctrl+W, the last tab stays | `files_tabs` | PASS |
| T-02 | Ctrl+Shift+Tab, a click on a tab | `files_tabs` | PASS |
| T-03 | The close button shows only on hover; the + button | `files_tabs` | PASS |
| T-04 | A middle click closes a tab, never the last | `panels_tabs` › pointer | PASS |
| T-05 | A double click after the tabs opens one at the panel's folder | `panels_tabs` › pointer | PASS |
| T-06 | Ctrl+Shift+1…9 switch tabs; a missing number does nothing | `panels_tabs` › numbered | PASS |
| T-07 | Scroll arrows agree with the strip; the active tab is scrolled into view | `panels_tabs` › many tabs | PASS |
| T-08 | A panel stops at 32 tabs: + disabled with its reason, Ctrl+T adds nothing | `panels_tabs` › many tabs | PASS |
| T-09 | The right panel's tabs are its own | `panels_tabs` › second tabs | PASS |
| T-10 | F7, Shift+F4 and F2 in a second tab act on that tab's folder, in both panels | `panels_tabs` › second tabs | PASS |
| T-11 | Tabs and the active tab come back after a restart | `panels_tabs` › restart | PENDING: the harness's `Config::restore_tabs` start is not confirmed yet (still so in the Linux run `20261121-144246`) |

## View, columns, sorting and layout

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| V-01 | Ctrl+F3–F6 sort by name, extension, time and size both ways; folders first; extensionless last | `files_sorting`; `panels_view_sort` | PASS |
| V-02 | Every column header sorts (a new column ascending, again reverses); the header states its direction | `panels_view_sort` › headers | PASS |
| V-03 | The sort menu's columns and Reverse order | `panels_view_sort` › sort menu | PASS |
| V-04 | Sorting keeps the marks and the cursor's file, scrolled into view | `panels_view_sort` › every sort step | PASS |
| V-05 | Each panel sorts on its own | `panels_view_sort` › right panel | PASS |
| V-06 | Detailed, Brief and Thumbnails from the View menu keep cursor and marks | `panels_view_sort` › views | PASS |
| V-07 | Alt+2, Alt+3 and Alt+5 (physical keys) switch the view | `panels_view_sort` › views | PASS |
| V-08 | Brief: Left and Right move by column | `panels_view_sort` › Brief | PASS |
| V-09 | Thumbnails: arrows move by cell and grid row | `panels_view_sort` › Thumbnails | PASS |
| V-10 | Dragging the splitter resizes the panels | `panels_view_sort` › splitter | PASS |
| V-11 | The splitter stops at the minimum panel width | `panels_view_sort` › splitter | PASS |
| V-12 | At the narrowest split file names stay readable | `panels_view_sort` › splitter | PASS |
| V-13 | A double click centers the splitter; its Left/Right/Enter keys | `panels_view_sort` › splitter | PASS |
| V-14 | The split ratio is restored after a restart | `panels_view_sort` › restart | PASS |
| V-15 | Ctrl+F11 enlarges the active panel; Tab moves it; Ctrl+F11 restores the split | `panels_view_sort` › zoom | PASS |
| V-16 | A small window shows one panel and a rail that activates the other | `panels_view_sort` › compact | PASS |
| V-17 | Touch density: header sorting, the 12 px splitter drag | `panels_view_sort` › touch | PASS |
| V-18 | Column resizing | not a panel feature | SKIP: panel columns have no resize handles; their widths are measured (Find's result columns resize, see the Find record) |

## Selection

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| S-01 | Insert toggles and advances; Space toggles, advances and measures a folder | `files_selection` | PASS |
| S-02 | Ctrl+A, Ctrl+Shift+A, Ctrl+D, Ctrl+I, Ctrl+Shift+I | `files_selection` | PASS |
| S-03 | Ctrl+click toggles; Shift+click selects a range | `files_selection` | PASS |
| S-04 | Ctrl+Shift+click adds a range | `panels_selection_more` | PASS |
| S-05 | Keypad + − * (physical keys) with masks, Include folders, Escape | `files_selection` | PASS |
| S-06 | Ctrl+=, Ctrl++ and Ctrl+− open Select/Deselect files from the remembered mask and Include folders | `panels_selection_more` | PASS |
| S-07 | An invalid mask (two `\|`, nothing after `\|`) changes nothing and says why | `panels_selection_more` | PASS (P-5 fixed) |
| S-08 | F10 › Select by mask… | `files_selection` | PASS |
| S-09 | Alt+Down and Alt+Up visit the marked rows without wrapping, marks kept | `panels_selection_more` | PASS |
| S-10 | Insert and Space stop at the last row | `panels_selection_more` | PASS |
| S-11 | `..` is never marked (Insert, Ctrl+Shift+A, Ctrl+Shift+I) | `panels_selection_more` | PASS |
| S-12 | The rubber band from the gutter: plain replaces, Shift adds, Ctrl toggles | `panels_selection_more` | PASS |
| S-13 | The details strip counts marked files and folders and their bytes | `panels_selection_more` › every step | PASS |
| S-14 | Marks with hidden files hidden and with a filter: Ctrl+A marks only what is shown | `files_filter_search`; `panels_search_filter` | PASS |
| S-15 | F5 of a multi-selection deselects the completed roots | `files_selection` | PASS |
| S-16 | The number row's + starts a search | `files_selection` | PASS |

## Filename search, hidden files and the panel filter

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| Q-01 | Typing searches by prefix; the cursor goes to the first match | `files_filter_search`; `panels_search_filter` | PASS |
| Q-02 | Up and Down visit matches without wrapping; Home, End, Page Up and Page Down follow match order | `panels_search_filter` | PASS |
| Q-03 | Backspace removes one character; an empty query ends the search | `panels_search_filter` | PASS |
| Q-04 | Space extends the query; Insert ends the search and marks the match | `panels_search_filter` | PASS |
| Q-05 | Unicode names match without case | `panels_search_filter` | PASS |
| Q-06 | A query survives sorting and Ctrl+R; opening a folder ends it | `panels_search_filter` | PASS |
| Q-07 | Escape ends the search and restores the listing | `files_filter_search` | PASS |
| Q-08 | Show only filename search matches hides the others | `panels_paths` › quick filter; `options_general` | PASS |
| H-01 | Ctrl+H, the View menu, F10 and Options hide and show dot-names in both panels; saved across a restart | `files_filter_search` | PASS |
| H-02 | Hiding moves the cursor to a shown row, drops marks on hidden rows, counts "N hidden", leaves hidden tree children out | `files_filter_search` | PASS |
| F-01 | Ctrl+F12 Masks and Expression filters; Show all | `files_filter_search` | PASS |
| F-02 | The drive bar's funnel opens the filter; Regex | `panels_search_filter` | PASS |
| F-03 | Case sensitive | `panels_search_filter` | PASS |
| F-04 | The indicator above the list names the filter and opens it with its draft | `panels_search_filter` | PASS |
| F-05 | The indicator's × turns the filter off, keeps the expression and the keyboard in the list | `panels_search_filter` | PASS |
| F-06 | The filter history lists applied filters and fills the draft | `panels_search_filter` | PASS |
| F-07 | Cancel keeps the applied filter | `panels_search_filter` | PASS |
| F-08 | An invalid regular expression is refused in the dialog; the listing stays | `panels_search_filter` | PASS |
| F-09 | Ctrl+A, Ctrl+Shift+A, sorting and search inside a filtered list | `panels_search_filter` | PASS |
| F-10 | Each panel has its own filter | `panels_search_filter` | PASS |
| F-11 | A filtered tree keeps a match's ancestor as context that cannot be marked | `panels_tree_keys` | PASS |
| F-12 | The filter's Help and F1 show the syntax; Escape closes only the help | `panels_search_filter` | PASS |

## Inline folder branches

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| B-01 | The expander expands and collapses a branch | `files_tree_ops`; `panels_tree_keys` | PASS |
| B-02 | Ctrl+click expands a whole branch | `tree_expand_all` | PASS |
| B-03 | Arrows, Home and End through nested rows | `panels_tree_keys` | PASS |
| B-04 | Enter on a nested folder opens it; Backspace returns with the cursor on it | `panels_tree_keys` | PASS |
| B-05 | Collapsing a branch with the cursor inside moves the cursor to the branch | `panels_tree_keys` | PASS |
| B-06 | Insert and Space (measuring) on nested rows; Ctrl+Shift+I keeps nested marks; Ctrl+A and Ctrl+D across levels | `panels_tree_keys` | PASS |
| B-07 | Each branch is ordered by the panel's sort (name both ways, time) | `panels_tree_keys` | PASS |
| B-08 | F5, F2 and Shift+F8 of a folder three levels deep | `files_tree_ops` | PASS |
| B-09 | A branch that cannot be listed shows its error icon; the icon opens the notice with the whole path | `panels_notices` › failed branch | PASS |
| B-10 | An expanded branch stays expanded while its folder changes | `panels_busy_ops` | PASS |
| B-11 | Retry lists a failed branch once its folder is readable; the error icon goes | `panels_notices` › failed branch | PASS |
| B-12 | The branch capacity limit (10,000 rows) | none yet | PENDING: no check written yet |

## Busy folders

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| W-01 | While the folder churns: rows keep identity, cursor and marks stay, no loading state | `files_busy_folder` | PASS |
| W-02 | A rename and a copy finished meanwhile are listed within 1.5 s | `files_busy_folder` | PASS |
| W-03 | F7 and Shift+F4 while busy: done, listed at once, nothing left open; Ctrl+F2 while busy | `panels_busy_ops` | PASS (Linux run `20261121-144246`: Ctrl+F2 on the two marked files, as the check expects since 21 November; F7 and Shift+F4 listed 45 and 47 ms after their reports). The earlier FAIL was a test error, not a product bug |
| W-04 | Once quiet the listing equals the disk | `files_busy_folder`; `panels_busy_ops` | PASS |

## Create, rename, attributes and times

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| O-01 | Shift+F4 creates an empty file, Unicode name, cursor on it | `create_rename` | PASS |
| O-02 | Shift+F4 on an existing name asks; a second Enter does not replace; Replace file empties it and names the recovery copy | `create_rename`; `files_create` | PASS |
| O-03 | F7 one name, nested segments, `..`, an existing name, Escape | `files_create` | PASS |
| O-04 | F2 with the stem preselected; Unicode, case only, collision (Skip, Replace), `/`, empty, 255/256 bytes, a folder; listed at once with the cursor on it | `create_rename`; `files_rename` | PASS |
| O-05 | Ctrl+F2 permission check boxes and octal | `files_attributes` | PASS |
| O-06 | Ctrl+F2 typed date, time (nanoseconds) and offset for one file, three files and a folder | `files_attributes` | PASS |
| O-07 | Up/Down change the date, time and offset part under the cursor | `panels_times` | PASS |
| O-08 | The calendar: Alt+Down, arrows, Page Up/Down, Enter, Previous/Next month, a day, Today | `panels_times` | PASS |
| O-09 | The time picker: Alt+Down, arrows, Page Up/Down, ▲/▼, Escape, Enter | `panels_times` | PASS |
| O-10 | Now fills date, time and offset and ticks the row; Apply sets that moment | `panels_times` | PASS |
| O-11 | Escape closes a picker only; the next Escape closes the editor without applying | `panels_times` | PASS |
| O-12 | Ctrl+F7 opens the Case page; lowercase renames the file | `panels_times` | PASS |
| O-13 | Keys typed right after F2's Enter wait until the new name is listed, then act from it | `files_rename` › held keys | PASS (Linux run `20261121-144246`) |
| O-14 | Include subfolders sets the time on a folder and everything below it; Back to editing keeps the draft | `panels_times` › Include subfolders | PASS (Linux run `20261121-144246`) |

## Folder sizes, Hot Paths and Make File List

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| Z-01 | Ctrl+Shift+S measures every folder as the disk's logical bytes | `folder_sizes` | PASS |
| Z-02 | Marked folders only; links not followed; folder links measured as their targets; an unreadable subfolder gives a partial size with a warning; cancel | `tools_folder_sizes` | PASS |
| Z-03 | Space measures a nested folder | `panels_tree_keys` | PASS |
| Z-04 | The toolbar's Folder sizes button measures the shown folders | `panels_paths` › toolbar | PASS |
| P-01 | Ctrl+Shift+Plus adds the folder; the Hot Paths menu opens it; the manager renumbers, removes and adds | `files_hot_paths` | PASS |
| P-02 | 1–9 and 0, Shift+ and Ctrl+digit open Hot Path N; Ctrl+Alt+ and Alt+Shift+digit in the other panel | `panels_hot_digits` | PASS |
| P-03 | An unassigned number is swallowed; a digit during a search stays text | `panels_hot_digits` | PASS |
| P-04 | Ctrl+Shift+0 makes the folder number 10 | `panels_hot_digits` | PASS |
| P-05 | The star adds under the first free number; Add path beyond ten has none | `panels_hot_digits` | PASS |
| P-06 | The Hot Paths menu and the drive menu open saved folders | `panels_hot_digits` | PASS |
| P-07 | Hot Paths survive a restart; numbers exchanged in the manager | `options_hot_paths` (Options and tools) | see that scenario |
| L-01 | Ctrl+M to a file with a tab template; the preview expands tabs; the marks are cleared | `files_file_list` | PASS |
| L-02 | Clipboard output | `panels_file_list` | PASS |
| L-03 | FileNamePart, FileExtension, FileSize, `$$`, `:max`, `<DIR>` | `panels_file_list` | PASS |
| L-04 | Browse… sets the destination from the chooser | `panels_file_list` | PASS |
| L-05 | An existing file asks first: Cancel keeps it; Replace and Append do so, and a notice names the recovery copy holding the original bytes | `panels_file_list` | PASS |
| L-06 | The focused entry when nothing is marked; a panel filter limits the list | `panels_dialogs`; `panels_file_list` | PASS |
| L-07 | The template history | `panels_file_list` | PASS |
| L-08 | Internal viewer output shows exactly the list | `panels_root_keys` (staged with the viewers) | PASS |

## Dialogs and notices

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| D-01 | An unreadable folder's notice: title, reason and its whole 400-character path readable without scrolling | `panels_notices` | PASS |
| D-02 | Details and Hide details; Copy details copies the reason and the path | `panels_notices` | PASS |
| D-03 | Escape, Close and × close it; the panel keeps its listing without a second banner | `panels_notices` | PASS |
| D-04 | The same at touch density in a 900×640 window | `panels_notices` | PASS |
| D-05 | The whole-path check is able to fail (a 480×260 window) | `panels_notices` › control | PASS |
| D-06 | Every panel dialog has one × in its header and one Cancel/Close button | `panels_dialogs` | PASS |
| D-07 | Escape, ×, Cancel/Close close every panel dialog; no notice, window or popup remains; nothing changes on disk | `panels_dialogs` | PASS |
| D-08 | After a dialog opened by its key closes, the keyboard is back in the list | `panels_dialogs` | PASS |
| D-09 | Each dialog works after cancelled rounds (F7, Shift+F4, F2, Ctrl+F2, Ctrl+F12, Ctrl+=/−, Ctrl+M, the manager, Alt+Enter) | `panels_dialogs` › afterwards | PASS |
| D-13 | After a dialog opened from the toolbar closes, the keyboard is back in the list | `panels_dialogs` | PASS |
| D-14 | After the Hot Paths manager closes, a click on a row focuses the list | `panels_dialogs` | PASS |

## Latest recorded run

Linux run `20261121-144246` of commit 3f6fff8a (WSL 2, Ubuntu 24.04 on ext4, debug test
profile, smoke scale, every plugin package built from that commit for the staged
`panels_root_keys`; every scenario of the suite ran; report in
`_build/acceptance-linux/20261121-144246/`): 32 scenarios, 31 passed, 1 pending
(`panels_tabs`: the tab restore waits for the harness). `startup_paths` belongs to the
harness owner and passed in the same run (6.0 s).

| Scenario | Status | Duration |
| --- | --- | --- |
| `startup`, `navigation`, `create_rename`, `tree_expand_all`, `folder_sizes`, `files_rename`, `files_create`, `files_attributes`, `files_selection`, `files_sorting`, `files_filter_search`, `files_navigation`, `files_tabs`, `files_hot_paths`, `files_file_list`, `files_tree_ops`, `files_busy_folder`, `tools_folder_sizes` | PASS | 0.8–31 s each (`navigation` 31.3 s) |
| `panels_dialogs` | PASS | 84.7 s |
| `panels_notices`, `panels_times`, `panels_view_sort`, `panels_paths`, `panels_search_filter`, `panels_tree_keys`, `panels_selection_more`, `panels_hot_digits`, `panels_file_list` | PASS | 3.7–29.5 s each |
| `panels_tabs` | PENDING (T-11) | 16.6 s |
| `panels_busy_ops` | PASS (W-03) | 8.7 s |
| `panels_root_keys` | PASS (P-6 fixed) | 12.6 s |

The earlier host run of 2026-11-20 (coordinated build of 16:05): 29 passed, 1 pending
(`panels_tabs`), 2 failed (`panels_busy_ops`, the W-03 test error; `panels_root_keys`,
P-6).

Commands of the Linux run (from the checkout, after `./build.sh debug`; the run was started
in a delegated systemd user scope, `systemd-run --user --scope -p Delegate=yes`, so that
the HTML viewer's browser finds a cgroup-v2 delegation):

```text
MIDNIGHT_ACCEPTANCE_RUN_DIR=<checkout>/_build/acceptance bash scripts/acceptance.sh debug --scale smoke
```

The 2026-11-20 host run ran the built executable directly:

```text
cargo test -p midnight-acceptance --locked --features midnight-acceptance/acceptance --test acceptance --no-run
MIDNIGHT_ACCEPTANCE_RUN_DIR=<absolute run folder> MIDNIGHT_ACCEPTANCE_PLUGINS=<complete plugin set> \
  <target>/debug/deps/acceptance-<hash> --scale smoke startup panels_ navigation create_rename \
  tree_expand_all folder_sizes files_rename files_create files_attributes files_selection \
  files_sorting files_filter_search files_tabs files_hot_paths files_file_list files_tree_ops \
  files_busy_folder
```

`files_rename` › held keys (O-13) and `panels_times` › Include subfolders (O-14), written
after that run, passed in the Linux run.
