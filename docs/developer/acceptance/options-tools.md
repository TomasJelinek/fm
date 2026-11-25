# Acceptance coverage: Options, appearance, keyboard and tools

The inventory of user-visible Options pages and controls, the keyboard shortcut catalog
and its hints, the F10 main menu, the F9 User menu, Checksums, Help, Drive
information, About, the Jobs window's list and notice dialogs, with the acceptance
scenario and check that covers each item and its last verified status. The suite is
described in [ACCEPTANCE.md](../ACCEPTANCE.md); scenarios live in
`crates/midnight-acceptance/tests/acceptance/scenarios/`.

Status: **PASS** (the check passed in the run named at the end), **FAIL** (a product
bug, with its finding), **PENDING** (not verified yet; the reason is given), **SKIP**
(the environment cannot run it; the reason is given).

## Options dialog

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-D01 | The toolbar's gear opens Options when its settings are ready | every `options_*` | `options_open`, `preferences_ready`, not busy | PASS |
| O-D02 | F10 › Options opens it | `tools_main_menu` | `options_open` | PASS |
| O-D03 | The page list switches pages | every `options_*` | topic index per page | PASS |
| O-D04 | A narrow window shows a topic picker instead of the list, and works | `options_themes`, `options_visual` | picker present, list absent; a theme chosen and saved through it | PASS |
| O-D05 | OK saves and closes | every `options_*` | settings store saves | PASS |
| O-D06 | Cancel discards previews and closes | `options_general`, `options_appearance`, `options_colors`, `options_keyboard`, `options_themes` | previews restored, no save | PASS |
| O-D07 | Escape discards like Cancel | `options_general`, `options_buttons` | preview restored, no save | PASS |
| O-D08 | Enter confirms like OK | `options_general` | one save, closed | PASS |
| O-D09 | The header's close button discards and closes | `options_buttons` | Options closes, nothing saved | PASS |
| O-D10 | Reset to defaults restores every page's defaults and keeps tabs, history, Hot Paths and commands | `options_general`, `options_appearance` | shown values and saved values | PASS |
| O-D11 | A page's invalid entry keeps Options open with its reason | `options_editors`, `options_comparators` | error line text | PASS |
| O-D12 | Every enabled button on every page has a visible effect | `options_buttons` | text, control, popup, window, chooser or command changes | PASS (after the fix for finding O-4) |

## General

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-G01 | Show only filename search matches | `options_general` | typing hides non-matching rows; saved; restart | PASS |
| O-G02 | Show hidden files | `files_filter_search` (panels) | dot-names hidden; saved; restart | PASS |
| O-G03 | Always show file sizes in bytes | `options_general` | the size column; preview, Cancel, OK, restart, reset | PASS |
| O-G04 | Verify entire copied file with SHA-256, and the quick-verification note | `options_general`, `files_copy_options` (transfers) | note shown; saved; the job's verification | PASS (the job side in `files_copy_options`, whose other check has a transfers finding) |
| O-G05 | Disk I/O concurrency picker | `options_general` | shown, saved, restart, reset to Automatic (4) | PASS |
| O-G06 | Disk I/O concurrency's effect on a job | – | no visible effect to observe (worker count is not shown) | PENDING |
| O-G07 | Remember command history after restart | `options_general` | saved; after clearing and a restart Alt+Up recalls nothing | PASS |
| O-G08 | Clear command history (enabled only with history) | `options_general` | enabled states; saved history empty | PASS |

## Appearance

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-A01 | Theme previews; Cancel restores | `options_appearance` | theme index, mean luma | PASS |
| O-A02 | Every theme of the picker previews distinctly; Light themes bright, Dark ones dark; OK saves; restart | `options_themes` | theme index and panel pixels per theme; saved theme; restart | PASS |
| O-A03 | Spacing (desktop, touch) | `options_appearance` | row height, density | PASS |
| O-A04 | Font family | `options_appearance` | rendered panel text changes | PASS |
| O-A05 | Desktop and touch font sizes | `options_appearance` | row height, rendered text | PASS |
| O-A06 | Text rendering (Sharp, Balanced, Smooth) | `options_appearance` | stored and shown after a restart (the software renderer ignores it) | PASS |

## Keyboard

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-K01 | Search commands and select one | `options_keyboard`, `shortcuts_hints` | the command's id | PASS |
| O-K02 | The selected command's shortcuts picker | `options_keyboard` | aliases shown | PASS |
| O-K03 | Capture a key; Assign | `options_keyboard`, `shortcuts_hints` | captured text; aliases | PASS |
| O-K04 | Remove an alias | `options_keyboard`, `shortcuts_hints` | aliases | PASS |
| O-K05 | An occupied key names its owner and asks (Replace, Cancel) | `options_keyboard` | "Shortcut currently used by"; Replace shortcut dialog | PASS |
| O-K06 | Restore command | `options_keyboard` | default back; override dropped | PASS |
| O-K07 | Restore all defaults asks first | `options_buttons` | the confirmation appears | PASS |
| O-K08 | The rebound key works, the old one does not, also after a restart | `options_keyboard`, `shortcuts_hints` | Create folder dialog | PASS |
| O-K09 | Hints follow a rebinding: F10 menu, F1 help, footer | `shortcuts_hints` | shown keys | PASS |
| O-K10 | Tab or Escape in the capture field is recorded, not run | `options_keyboard` | captured text; Options stays open | PASS |
| O-K11 | Reserved keys (Ctrl+Space) and a fifth alias are refused with a reason | `options_keyboard` | "reserved" and "At most four" reasons; shortcuts unchanged; Cancel discards | PASS |

## Editors, Comparators, Plugins, Previews, Archives, Colors, Custom Commands

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-E01 | Editors: a fallback and a `*.md` rule; F4 runs the matching one; restart | `options_editors` | file written by the editor command | PASS |
| O-E02 | Editors: a rule without `{file}` is refused | `options_editors` | error line | PASS |
| O-C01 | Comparators: built-in and external overrides saved, listed after a restart | `options_comparators` | rule rows, saved values | PASS |
| O-C02 | Comparators: an invalid regular expression is refused | `options_comparators` | error line | PASS |
| O-P01 | Plugins: empty states without staged plugins; Rescan; the Archives link | `options_previews_plugins` | texts; page switch | PASS |
| O-P02 | Archive plugin details fit above Archive associations | `options_archive_plugin_layout` | Installed plugin, wrapped formats and package path, button bounds and navigation at 1024×700 and 560×560 | PASS (Windows, 2026-11-25) |
| O-V01 | Previews: switches, thumbnail size seen in the Thumbnails view; Cancel, OK, restart | `options_previews_plugins` | row height; saved values | PASS |
| O-V02 | Previews: Clear thumbnail cache | – | the thumbnail service exists only in staged viewer processes; the button would clear a real cache | SKIP |
| O-R01 | Archives: custom extension chips (add, fold, remove, invalid, built-in conflict), an association; Cancel, OK, restart; Escape in the field | `options_archives` | chips, errors, saved values | PASS |
| O-L01 | Colors: an added rule colours matching names; priority by Move up; Cancel; OK; restart | `options_colors` | row colours, rendered pixels, saved rules | PASS |
| O-L02 | Colors stay on their rows sorted by size and filtered by a panel filter | `options_colors` | row colours, rendered pixels | PASS |
| O-U01 | Custom Commands: create, edit, drag, delete, separators, placeholders; runs; failure notice; Preview; key; restart | `options_custom_commands` | files written, notices, saved commands | PASS |
| O-H01 | Hot Paths manager: add, exchange numbers, remove; Ctrl+digit; restart | `options_hot_paths`, `files_hot_paths` (panels) | panel folders, saved Hot Paths | PASS |

## Keyboard shortcut catalog

Every default binding of `shortcut_catalog()`, alternates included, is pressed in the
context its scope names (`shortcuts_catalog`). Number-row digits, the keypad's + − *
and modified bracket keys go through the window's physical-shortcut entry, as the
native key hook delivers them. The probe compares the whole visible state: anything
besides the action's own effect fails it.

| ID | Actions | Check | Status |
| --- | --- | --- | --- |
| O-S01 | Panel navigation: switch, open, up, down, page up/down, first, last, left/right (Brief), parent ×2, root ×3, enter folder, back, forward, refresh ×2, path, swap, zoom | cursor, focus, folder, listing, flags | PASS |
| O-S02 | Selection: Space, Insert, Ctrl+Shift+A, Ctrl+A, Ctrl+D, Ctrl+I, Ctrl+Shift+I, Alt+Up/Down between marks | marks and cursor | PASS |
| O-S03 | Select/Deselect by mask: Ctrl+=, Num +, Ctrl++; Ctrl+−, Num −; Num * | mask dialog (select/deselect); inverted marks | PASS |
| O-S04 | Sorting Ctrl+F3–F6; Ctrl+H hidden files | sort column and direction; hidden row gone | PASS |
| O-S05 | File dialogs: F2, Shift+F4, F7, Delete/F8, Shift+Delete/Shift+F8, Ctrl+F2, Ctrl+F7, F5, F6, Alt+Enter, Ctrl+Alt+Enter | the right dialog (title, dangerous, move, case page) | PASS |
| O-S06 | Clipboard: Ctrl+C, Ctrl+X, Ctrl+V | in-process clipboard; a pasted copy | PASS |
| O-S07 | Tools: Ctrl+F, Alt+F7, Ctrl+F8, Ctrl+M, Ctrl+F10, Ctrl+F12, Ctrl+F1, Ctrl+L, Ctrl+Shift+S, Ctrl+Shift+F10, F4 | windows, dialogs, notice, sizes | PASS |
| O-S08 | Tabs: Ctrl+T, Ctrl+W, Ctrl+Shift+Tab, Ctrl+Shift+1…9 | tab count, active tab, folder | PASS |
| O-S09 | Hot Paths: 1…0, Ctrl+1…0, Shift+1…0 (this panel), Ctrl+Alt+ and Alt+Shift+1…0 (other panel), Ctrl+Shift+Plus/Equal (add), Ctrl+Shift+0 (slot 10) | folders; saved Hot Paths | PASS |
| O-S10 | Command line: Ctrl+Tab, Ctrl+Enter, Ctrl+Shift+P, Ctrl+P, Ctrl+[ and ] (also with Shift), Alt+Up/Down, Escape | draft text and focus | PASS |
| O-S11 | Command line Enter: submitted to a terminal | this harness has no desktop services (they would start a real terminal): "Terminal unavailable" is checked instead | PASS |
| O-S12 | Application: F1, F9, F10, Alt+F1, Alt+F2, the drives popup's ".", Alt+2/3/5 view modes, Alt+F4 | windows, menus, popups, view mode, window hidden | PASS |
| O-S13 | Network F11/F12 on Linux change nothing | whole state unchanged | PASS |
| O-S14 | Shift+letter drive keys on Linux switch nothing | folders unchanged | PASS |
| O-S15 | Keys outside their context: Space and a digit typed into filename search, Insert ends it, F7 and Ctrl+A in the command editor, a digit typed there | search text, marks, draft | PASS |
| O-S16 | An uppercase letter starts a filename search | search text | PASS (after the fix for finding O-1) |
| O-S17 | Viewer and comparison keys; F3, Alt+F3, Ctrl+Shift+C | viewer windows exist only in staged processes; covered by `viewers_files`, `viewers_controls`, `viewers_compare` | SKIP |
| O-S18 | Actions without a default key (Options, Jobs, About, Custom Commands, Manage Hot Paths, Deselect files only) | reached from F10 (`tools_main_menu`) and Options | PASS |

## Shortcut hints and the footer

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-F01 | F1 help lists every catalog action with all its keys or "Unassigned" | `shortcuts_hints` | one line per action | PASS |
| O-F02 | F10 shows each item's key (Ctrl+= beside Select by mask, Num * beside the invert, none beside Options) | `shortcuts_hints` | key text per item | PASS |
| O-F03 | Footer F1–F12 in the plain, Ctrl, Shift, Alt and Ctrl+Shift layers: a click does what the key does | `shortcuts_footer` | the key's probe after the click | PASS |
| O-F04 | Footer cells without an action are disabled and do nothing | `shortcuts_footer` | disabled; state unchanged | PASS (after the fix for finding O-2) |
| O-F05 | Footer F11/F12 (network) hidden on Linux | `shortcuts_footer` | no cell | PASS |
| O-F06 | After a rebinding the footer cell becomes unassigned | `shortcuts_hints` | "F7 — unavailable" | PASS |
| O-F07 | Panel View and Sort menus show their keys | – | not driven yet | PENDING |

## Menus

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-M01 | F10 categories list exactly their items | `tools_main_menu` | item lists per category | PASS |
| O-M02 | Every F10 item does what it says (cancelled or undone); Exit last | `tools_main_menu` | dialogs, windows, effects | PASS |
| O-M03 | F9 lists the built-in items, the commands (separator as a rule, disabled shown disabled) and Close | `tools_user_menu` | button order and states | PASS |
| O-M04 | F9 items: Command line, Bookmarks and Hot Paths, Properties, folder sizes, Checksums, Close | `tools_user_menu` | focus, manager, notice, sizes, window | PASS |
| O-M05 | F9 keyboard: Tab then Enter runs an item; Escape closes; a click outside closes | `tools_user_menu` | effect; menu closed | PASS |
| O-M06 | F9 commands on marked files: grouped and per item | `tools_user_menu` | files written | PASS |
| O-M07 | F9 commands on a folder shown only in an expanded tree branch | `tools_user_menu` | files written | PASS |
| O-M08 | F9 disabled command runs nothing | `tools_user_menu` | no file | PASS |
| O-M09 | F9 without commands: the empty state; Set up opens Options › Custom Commands | `tools_user_menu` | text; Options page | PASS |

## Tools

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-T01 | Checksums: Calculate, Save manifest, Replace, Verify; MD5, SFV, mismatch, missing | `checksums` | digests, files, results | PASS |
| O-T02 | Checksums: all five algorithms, a manifest each, Replace prompt, Include subfolders | `tools_checksum_algorithms` | independent digests; manifest lines | PASS |
| O-T03 | Checksums: Verify preselected for BSD, GNU, SFV, sidecars, UTF-16; mixed manifest error | `tools_checksum_manifests` | results | PASS |
| O-T04 | Checksums: progress and Stop on a large file | `tools_checksum_large_stop` | bytes grow; Cancelled; no manifest | PASS |
| O-T05 | Checksums: Ctrl+C, Ctrl+Shift+C, the Shift+F10 and right-click copy menu in the results | `tools_checksum_algorithms` | the clipboard's text: SHA-256, the whole row, MD5 | PASS |
| O-T06 | F1 Help: opens, lists the tools' shortcuts, Close and Escape | `tools_help_drive_info` | window, texts | PASS |
| O-T07 | Ctrl+F1 Drive information: folder, volume, filesystem, sizes; Close | `tools_help_drive_info` | notice body | PASS |

## About

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-B01 | The toolbar's application icon opens About | `about_box` | `about_open` | PASS |
| O-B02 | About tab: version in the title strip and the Version row, Built with | `about_box` | texts | PASS |
| O-B03 | About tab lines have room between them | `about_box` | at least 6 px between lines | PASS |
| O-B04 | Left and Right switch the tabs | `about_box` | shown tab | PASS |
| O-B05 | Licenses: the count matches the list | `about_box` | summary and rows | PASS |
| O-B06 | Rows look selectable: pointer cursor, hover background and "›" mark on the hovered row only | `about_box` | cursor, pixels, mark | PASS |
| O-B07 | A click selects a row; its mark stays; View license enables | `about_box` | selected state, mark, button | PASS |
| O-B08 | View license, Enter and a double-click open the detail (title, License, Used by, authors, text) | `about_box` | detail texts | PASS |
| O-B14 | A click on a row's "›" mark opens its detail | `about_box` | detail opens after a click in the middle of the shown mark | PASS (after the fix for finding O-3) |
| O-B09 | Back and Escape return to the list with the row and filter kept | `about_box` | selection, filter | PASS |
| O-B10 | Up, Down, Page Down, End, Home move the selection | `about_box` | selected row | PASS |
| O-B11 | Filter: "K of N", only matching rows, no match, Down into the list, Enter opens the first | `about_box` | summary, rows, detail | PASS |
| O-B12 | Reopening About keeps the filter and the selection | `about_box` | filter, selection | PASS |
| O-B13 | Escape in the list, Close and a click outside close About; dragging the sky moves it | `about_box` | `about_open`, position | PASS |

## Jobs window list and notices

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-J01 | Jobs lists create, rename, delete and failed jobs with their states; header count | `tools_notices_jobs` | rows, states, header | PASS |
| O-J02 | A click and Up/Down select rows | `tools_notices_jobs` | selected row | PASS |
| O-J03 | Open and Enter show the selected job's window | `tools_notices_jobs` | window opens, closes | PASS |
| O-J04 | Copy details reaches the clipboard | `tools_notices_jobs` | command sent; "Copied to clipboard" | PASS |
| O-J05 | Cancel job disabled for finished jobs | `tools_notices_jobs` | enabled state | PASS |
| O-J06 | Clear finished without errors keeps the failed job; Clear all finished empties the list | `tools_notices_jobs`, `tools_jobs_copy` | rows; "No background operations" | PASS |
| O-J07 | Close and Escape close Jobs | `tools_notices_jobs` | window gone | PASS |
| O-N01 | A folder notice: title, reason, folder box | `tools_notices_jobs` | texts | PASS |
| O-N02 | Details expands, Hide details collapses | `tools_notices_jobs` | details text, button caption | PASS |
| O-N03 | Copy details reaches the clipboard | `tools_notices_jobs` | command sent; "Copied to clipboard" | FAIL: the copied report lacks the reason the notice shows ("Access to this folder was denied."); it holds the title, folder, raw error, path and operation (finding O-5, to be triaged) |
| O-N04 | Close, Escape and the header's close button close it; no other notice follows | `tools_notices_jobs` | notice token for one second | PASS |
| O-N05 | A very long folder path: dialog inside the window, Close in view, the path box scrolls to its end | `tools_notices_jobs` | rectangles; wheel scrolling | PASS |

## Combinations

| ID | Behavior | Scenario | Status |
| --- | --- | --- | --- |
| O-X01 | Theme × density × window size: the narrow touch layout's topic picker | `options_themes`, `options_visual` | PASS |
| O-X02 | Disk I/O and verification options with a copy | `files_copy_options` (verification); concurrency not observable | PENDING |
| O-X03 | Colors with sorting and a panel filter | `options_colors` | PASS |
| O-X04 | A rebound key used inside dialogs (Find results' F3/F4) | – | PENDING: not driven yet |
| O-X05 | Custom commands on multi-selections and tree rows | `tools_user_menu` | PASS |
| O-X06 | Restart persistence of every page | each `options_*` scenario | PASS |

## Findings

| ID | Finding | Check |
| --- | --- | --- |
| O-1 | On Linux, Shift+letter outside a filename search is taken by the Windows drive-letter action, finds no drive and is swallowed: typing an uppercase letter starts no search (during a search the same key is typed). | `shortcuts_catalog`, scope probe "Shift+R" |
| O-2 | A click on an unavailable (disabled) F-key cell of the footer takes the keyboard focus from the file list, so arrow keys and typing reach no panel until the list is clicked; enabled cells give the focus back. | `shortcuts_footer` |
| O-3 | About › Licenses: each row's "›" open mark and its click area lie in the row's last 26 px, under the list's 14 px overlay scrollbar; a click in the middle of the mark does nothing, only its left edge opens the library. | `about_box` |
| O-5 | A folder notice's Copy details copies the title, folder and raw error but not the reason the dialog shows ("Access to this folder was denied."), although its hint promises the complete message. | `tools_notices_jobs` |
| O-4 | Options buttons that are enabled but change nothing: Keyboard › Clear with nothing captured, Keyboard › Restore command for a command at its defaults, Colors › Discard changes without changes, Plugins › Rescan all plugins (no feedback without plugins). | `options_buttons` |

## Host runs

Runs (UTC) on the Linux host with the smoke-scale fixture, the debug acceptance binary of the
shared build and a complete plugin set:
`MIDNIGHT_ACCEPTANCE_RUN_DIR=<run folder> MIDNIGHT_ACCEPTANCE_PLUGINS=<plugin set>
_build/<target>/debug/deps/acceptance-<hash> --scale smoke <scenario>...`

| Run | Scenarios | Result |
| --- | --- | --- |
| 2026-11-20 14:25 | `checksums`, `tools_checksum_algorithms`, `tools_checksum_manifests`, `tools_checksum_large_stop`, `tools_help_drive_info`, `tools_main_menu`, `options_general`, `options_appearance`, `options_custom_commands`, `options_keyboard`, `options_colors`, `options_archives`, `options_editors`, `options_comparators`, `options_previews_plugins`, `options_hot_paths` | all PASS (9.3, 12.2, 13.8, 3.5, 5.5, 76.9, 16.8, 17.7, 49.6, 21.5, 13.3, 14.5, 11.4, 14.1, 10.6, 17.4 s; 315 s in all) |
| 2026-11-20 14:09 | `tools_user_menu`, `shortcuts_hints`, `options_themes` | PASS (15.2, 20.9, 34.3 s) |
| 2026-11-20 14:30 | `shortcuts_catalog` | FAIL: O-1 only; the other 197 keys and probes pass (336 s) |
| 2026-11-20 14:30 | `shortcuts_footer` | FAIL: O-2 only; every enabled cell does what its key does (89 s) |
| 2026-11-20 14:30 | `about_box` | FAIL: O-3, and End's selection not counted as in view (being checked) (24 s) |
| 2026-11-20 14:30 | `tools_notices_jobs` | PASS (about 60 s) |
| 2026-11-20 14:30 | `options_buttons` | FAIL: O-4 only; every other enabled button on every page has an effect (64 s) |
| 2026-11-20 14:41 | `files_filter_search`, `files_hot_paths`, `tools_jobs_copy` (cited above) | PASS (16.1, 5.5, 9.2 s) |
| 2026-11-20 14:41 | `files_copy_options` (cited for O-G04) | FAIL on its own destination check (transfers record); its verification checks pass |
| 2026-11-20 14:58 | `about_box`, `options_buttons` (with the fixes for O-3 and O-4) | PASS (21.0, 53.0 s) |
| 2026-11-20 15:21 | `tools_checksum_algorithms` with the results' copy checks (clipboard text) | PASS (43.4 s) |
| 2026-11-20 15:21 | `shortcuts_catalog`, `shortcuts_footer` with the fixes for O-1 and O-2 | PASS (350.8, 81.6 s) |
| 2026-11-20 15:21 | `tools_notices_jobs` with the clipboard text checks | FAIL: O-5 only (22.7 s) |
