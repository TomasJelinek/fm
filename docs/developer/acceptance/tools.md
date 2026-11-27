# Acceptance coverage: tools

The inventory of the F10 main menu, the F9 User menu, Checksums, Help, Drive
information, About, the Jobs window's list and notice dialogs, with the acceptance
scenario and check that covers each item and its last verified status. Options pages,
appearance, the keyboard editor and the shortcut catalog are in
[options-tools.md](options-tools.md). The suite is described in
[acceptance.md](../acceptance.md); scenarios live in
`crates/midnight-acceptance/tests/acceptance/scenarios/`.

Status: **PASS** (the check passed in the run named at the end), **FAIL** (a product
bug, with its finding), **PENDING** (not verified yet; the reason is given), **SKIP**
(the environment cannot run it; the reason is given).

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
| O-M10 | A window narrower than 640 px shows the Custom Commands as one "Custom commands" toolbar button (the commands' own buttons are gone); its bar over the window runs a command (on the focused file) and closes; Escape and a click outside the bar close it | `tools_user_menu` | buttons; the popup; the command's file on disk | PASS |
| O-M11 | F10 lists every chord of an action in one text beside the item, joined with " / " in the order Options › Keyboard lists them ("Shift+Delete / Shift+F8" for Delete permanently…, "Ctrl+Shift+S / Ctrl+Shift+F10" for Calculate / cancel folder sizes), and the item's tooltip reads "<label> (<chords>)" (22f46eab) | `tools_main_menu` | the row's texts; the tooltip's first line | PASS |
| O-M12 | Tools › "Open terminal as administrator" ("Open root terminal (sudo)" on Linux; the terminal button's Ctrl+click, b2184fb2) asks for an elevated terminal of the selected shell in the active panel's folder; the harness's desktop stand-in records it and starts nothing (no consent prompt, no sudo) | `tools_main_menu` | the recorded launch: a terminal, elevated, no command, the left panel's folder | PASS |
| O-M13 | Tools › "Clear comparison marks" (b70cf8df) is disabled with nothing compared; after Compare directories with subfolders it removes both panels' badges and the result line and keeps the selection | `tools_main_menu` | the item's state; the rows' compare marks, the line, the selection | PASS on Linux (`20261124-181835`) and Windows (`20261124-183236`) |

## Tools

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-T01 | Checksums: Calculate, Save manifest, Replace, Verify; MD5, SFV, mismatch, missing | `checksums` | digests, files, results | PASS |
| O-T02 | Checksums: all five algorithms, a manifest each, Replace prompt, Include subfolders | `tools_checksum_algorithms` | independent digests; manifest lines | PASS |
| O-T03 | Checksums: Verify preselected for BSD, GNU, SFV, sidecars, UTF-16; mixed manifest error | `tools_checksum_manifests` | results | PASS |
| O-T04 | Checksums: progress and Stop on a large file | `tools_checksum_large_stop` | bytes grow; Cancelled; no manifest | PASS |
| O-T05 | Checksums copies: on one row Ctrl+C its SHA-256, Ctrl+Shift+C its whole row, Shift+F10 and a right click open the menu of every algorithm and the full row (exactly those six), Copy MD5 copies the MD5. With two rows Ctrl+clicked: Copy reads "Copy 2 SHA-256" with its hint; Ctrl+C copies both shown checksums, one per line in the list's order; Ctrl+Shift+C both full rows (the path, then every checksum) with a blank line between; Shift+F10 on a selected row offers "Copy 2 SHA-256" … "Copy 2 full rows" and Copy 2 MD5 copies both; a right click on an unselected row offers that row's copies, copies it alone, keeps the selection and puts the cursor there; the footer's Copy copies the two checksum-file lines; Ctrl+D disables Copy with "Nothing to copy: … or press Ctrl+A to select all." and leaves the panel's marks; Ctrl+A reads "Copy 4 SHA-256" | `tools_checksum_algorithms` | the in-process clipboard's text per copy (counted copies, not the event trace); the selected rows; the cursor's row (its drawn edge); the footer's captions, states and hints | PASS |
| O-T06 | F1 Help: opens, lists the tools' shortcuts, Close and Escape | `tools_help_drive_info` | window, texts | PASS |
| O-T07 | Ctrl+F1 Drive information: folder, volume, filesystem, sizes; Close | `tools_help_drive_info` | notice body | PASS |
| O-T08 | Copy is there and disabled before anything is selected ("Copy SHA-256", "Nothing to copy: …"); with every row selected (Ctrl+A) it reads "Copy 4 SHA-256" and copies exactly the saved checksums.sha256, byte for byte | `checksums` | caption, state and hint; the clipboard against the file | PASS |
| O-T09 | No size limits: 20,000 files in one folder and a chain of 70 nested folders, with Include subfolders and every algorithm: the run ends "Completed · 20,070 calculated · …" and no text speaks of a limit; Ctrl+A reads "Copy 20,070 SHA-256"; Ctrl+Shift+C then copies all 20,070 full rows whole (each the path and five checksums, a blank line between), more than the 4 MiB the clipboard used to take; the saved manifest has 20,070 lines (the 70th level's file and the last file among them) and verifies "Completed · 20,070 verified · 0 mismatched · 0 missing". Counts appear as the system writes numbers ("20070" on the Linux host, "20 070" on the Windows host) | `tools_checksum_unlimited` | status lines, window texts, Copy, the in-process clipboard's text (size, blocks, the first file's SHA-256), manifest lines | PASS |
| O-T10 | The results follow the row being calculated while nothing is selected: in five samples, each after the run went on by two screenfuls, the boundary between calculated and waiting rows is in view and the list has moved on; a click on a row stops it (the rows in view hold still while the run goes on); Ctrl+D resumes it; a scroll by the user pauses it | `tools_checksum_unlimited` | the rows in view (indices and statuses) against the run's progress | PASS |
| O-T11 | Partial results: Stop after two of three files, then Save manifest… asks how many files were left out ("1 file was excluded" since 17e56f3d; "1 files" before, O-6) and "Save the 2 completed checksums anyway?"; Cancel writes nothing; Save writes the two lines and the save line reads "Saved checksums.sha256 · 2 included · 1 excluded"; saving again asks again, then Replace, and the line adds " · the replaced file is kept for recovery"; Details and Copy details name the file and the `.midnight-recovery…` folder | `tools_checksum_large_stop` | question, file contents, save line, details, clipboard | PASS (wording: O-6) |
| O-T12 | Escape while a calculation runs closes the window and writes no manifest; Ctrl+F8 then opens a new, idle window | `tools_checksum_large_stop` | window state; files; the new window's status and rows | PASS |
| O-T13 | Browse… on the Verify page asks the file chooser and only fills the field with the answer; Verify then checks that file, one run | `tools_checksum_manifests` | the field; chooser requests; the run count; the status | PASS |
| O-T14 | Over a verification's results Copy is disabled and says "A verification's results are not copied as a checksum file. …" | `tools_checksum_manifests` | state and hint after each verification | PASS |
| O-T15 | Ctrl+F8 during a filename search ends the search and captures the focused match (gamma.txt for "gam"), or the marks when there are any | `tools_checksum_manifests` | the rows calculated; the search state | PASS |
| O-T16 | Four Checksums windows at most: a fifth Ctrl+F8 opens none and says "Close one of the 4 open checksum windows first."; after one closes, Ctrl+F8 opens a window again | `tools_checksum_manifests` | windows; notice | PASS |
| O-T17 | The Checksums footer below 460 px: Copy is "Copy" and Save manifest… "Save…"; an action that does not fit moves into More…, Save… first, then Copy; Calculate stays a button; each 8 px from the next (440 and 320 px) | `tools_checksum_algorithms` | buttons, More… entries, gaps | PASS |
| O-T18 | Compare directories (0.5.1, Salamander's rules): the toolbar's button opens the dialog; a comparison with contents or subfolders keeps the dialog open with "Comparing … with …", its counts and Cancel, and no line appears at the bottom of the window; Cancel, Escape and Alt+C each end a running comparison within a moment and select nothing; the result is a line below the active panel; with subfolders a differing folder is selected in both panels and every row (also inside an expanded folder) shows its match badge; the dialog at 8, 12 and 32 px and in the dark theme | `tools_compare_dirs` | the dialog's state and texts; the bottom line; both panels' selections and rows' marks; the left panel's result line | PASS (`20261123-171004`, Windows 11, debug; Cancel ended the run in 119 ms, Escape in 63 ms, Alt+C in 66 ms) |
| O-T19 | Folder sizes: a partial total's warning says "Completed with warnings; this is a partial total."; a cancelled folder shows "≈ N" or "—" and says it is unfinished; Ctrl+Shift+S during a filename search ends it and measures the marks, or every folder when none are marked (as without a search, and as its toolbar hint says; the work plan first expected the focused match, and on 22 November the product's behavior was kept) | `tools_folder_sizes` | the rows' size texts, states and hints; the search state | PASS |

## About

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-B01 | The toolbar's application icon opens About | `about_box` | `about_open` | PASS |
| O-B02 | About tab: version in the title strip and the Version row; a Copyright row reading exactly "© 2026 …" (the year and the owner's name), its value in the Version row's column; no Author row, and the name never alone (5ccd0eb7); Built with | `about_box` | texts and positions | PASS |
| O-B03 | About tab lines have room between them, the Copyright line included | `about_box` | at least 6 px between lines | PASS |
| O-B15 | Saturn in the sky: drawn (its gold planet in the rendered pixels), clear of the title and the application icon; no crescent moon | `about_box` | image rectangles, rendered pixels, image labels | PASS |
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
| O-J04 | Copy details reaches the clipboard | `tools_notices_jobs` | a copy counted by the in-process clipboard and its text naming the job; "Copied to clipboard" | PASS |
| O-J05 | Cancel job disabled for finished jobs | `tools_notices_jobs` | enabled state | PASS |
| O-J06 | Clear finished without errors keeps the failed job; Clear all finished empties the list | `tools_notices_jobs`, `tools_jobs_copy` | rows; "No background operations" | PASS |
| O-J07 | Close and Escape close Jobs | `tools_notices_jobs` | window gone | PASS |
| O-N01 | A folder notice: title, reason, folder box | `tools_notices_jobs` | texts | PASS |
| O-N02 | Details expands, Hide details collapses | `tools_notices_jobs` | details text, button caption | PASS |
| O-N03 | Copy details reaches the clipboard | `tools_notices_jobs` | a copy counted by the in-process clipboard (not the event trace, which keeps 32 lines); "Copied to clipboard"; the text starts with the title, reason and folder | PASS (finding O-5, of 20 November, fixed: PASS on Linux in run `20261121-144246` and on Windows in run 20261121-040741) |
| O-N04 | Close, Escape and the header's close button close it; no other notice follows | `tools_notices_jobs` | notice token for one second | PASS |
| O-N05 | A very long folder path: dialog inside the window, Close in view, the path box scrolls to its end | `tools_notices_jobs` | rectangles; wheel scrolling | PASS |

## Findings

The finding numbers are shared with [options-tools.md](options-tools.md#findings), which
lists O-1, O-2 and O-4.

| ID | Finding | Check |
| --- | --- | --- |
| O-3 | About › Licenses: each row's "›" open mark and its click area lie in the row's last 26 px, under the list's 14 px overlay scrollbar; a click in the middle of the mark does nothing, only its left edge opens the library. | `about_box` |
| O-5 | A folder notice's Copy details copies the title, folder and raw error but not the reason the dialog shows ("Access to this folder was denied."), although its hint promises the complete message. Fixed: the check passes on Linux (run `20261121-144246`) and Windows (O-N03). | `tools_notices_jobs` |
| O-6 | Checksums' partial-save question does not match its count with the noun: with one file left out it reads "1 files were excluded. Save the 2 completed checksums anyway?" (`presenter/checksums/output.rs`). Wording only; the check asserts the count, " excluded" and the question, not the noun. Fixed on main in 17e56f3d ("1 file was excluded", "1 completed checksum"); the check stays noun-agnostic, so this record does not verify the wording. | `tools_checksum_large_stop` (O-T11) |

## Host runs

Runs (UTC) on the Linux host with the smoke-scale fixture, the debug acceptance binary of the
shared build and a complete plugin set:
`MIDNIGHT_ACCEPTANCE_RUN_DIR=<run folder> MIDNIGHT_ACCEPTANCE_PLUGINS=<plugin set>
_build/<target>/debug/deps/acceptance-<hash> --scale smoke <scenario>...`

Windows runs use `scripts\acceptance.ps1 --scale smoke --only <names> --plugins <release plugin packages> --test-binary <debug acceptance executable>` (run ids are UTC as well).

Several of these runs also ran Options scenarios; their results are in
[options-tools.md](options-tools.md#host-runs).

| Run | Scenarios | Result |
| --- | --- | --- |
| 2026-11-20 14:25 | `checksums`, `tools_checksum_algorithms`, `tools_checksum_manifests`, `tools_checksum_large_stop`, `tools_help_drive_info`, `tools_main_menu` | all PASS (9.3, 12.2, 13.8, 3.5, 5.5, 76.9 s) |
| 2026-11-20 14:09 | `tools_user_menu` | PASS (15.2 s) |
| 2026-11-20 14:30 | `about_box` | FAIL: O-3, and End's selection not counted as in view (being checked) (24 s) |
| 2026-11-20 14:30 | `tools_notices_jobs` | PASS (about 60 s) |
| 2026-11-20 14:41 | `tools_jobs_copy` (cited for O-J06) | PASS (9.2 s) |
| 2026-11-20 14:58 | `about_box` (with the fix for O-3) | PASS (21.0 s) |
| 2026-11-20 15:21 | `tools_checksum_algorithms` with the results' copy checks (clipboard text) | PASS (43.4 s) |
| 2026-11-20 15:21 | `tools_notices_jobs` with the clipboard text checks | FAIL: O-5 only (22.7 s) |
| `20261121-144246` (Linux, commit 3f6fff8a, WSL 2 Ubuntu 24.04, every scenario of the suite; report in `_build/acceptance-linux/20261121-144246/`) | every scenario of this record: `checksums`, `tools_checksum_algorithms`, `tools_checksum_manifests`, `tools_checksum_large_stop`, `tools_help_drive_info`, `tools_main_menu`, `about_box`, `tools_user_menu`, `tools_notices_jobs` | all PASS (4.6, 8.1, 6.5, 2.1, 2.1, 75.4, 10.8, 8.6, 13.7 s). O-5 is fixed (O-N03) |
| `20261122-100257` (Linux, origin/main 26c18895 with the Find and tools changes of this record, WSL 2 Ubuntu 24.04, plugin packages built from b1b89343, the shared Cargo target cleaned of every path package before the build) | every scenario of this record and of [find.md](find.md), with `tools_jobs_copy` and `journeys_find_to_checksums`: 26 scenarios | all PASS (`checksums` 10.2, `tools_checksum_algorithms` 16.7, `tools_checksum_manifests` 19.9, `tools_checksum_large_stop` 13.2, `tools_checksum_unlimited` 119.2, `tools_compare_dirs` 37.3, `tools_folder_sizes` 10.3, `tools_help_drive_info` 2.4, `tools_main_menu` 73.7, `about_box` 17.7, `tools_user_menu` 15.9, `tools_notices_jobs` 19.4 s; 895 s in all) |
| `20261122-093311`, `20261122-094218`, `20261122-095108` (Linux, the same tree and build setup; red runs) | the scenarios with new checks, each new check edited to expect a wrong value | every edited check failed with its own message; unedited scenarios passed |
| `20261122-112449` (Windows 11, origin/main 26c18895 with the Find and tools changes of this record, the release plugin packages built from d7ff8860) | the same 26 scenarios | all PASS (`checksums` 6.8, `tools_checksum_algorithms` 15.3, `tools_checksum_manifests` 17.3, `tools_checksum_large_stop` 10.3, `tools_checksum_unlimited` 248.2, `tools_compare_dirs` 24.4, `tools_folder_sizes` 47.6, `tools_help_drive_info` 3.4, `tools_main_menu` 71.4, `about_box` 21.7, `tools_user_menu` 33.1, `tools_notices_jobs` 23.4 s; 1166 s in all) |
| `20261122-165028` (Windows 11, origin/main 66a02637 with the Find and tools changes of this record, the release plugin packages built from d7ff8860) | the same 26 scenarios, with the About tab's Copyright row (O-B02, O-B03), the joined chords (O-M11) and the elevated terminal (O-M12) | 25 PASS (`about_box` 20.5, `tools_main_menu` 89.6, `tools_help_drive_info` 4.0, `checksums` 11.2, `tools_checksum_algorithms` 19.0, `tools_checksum_manifests` 18.6, `tools_checksum_large_stop` 9.9, `tools_checksum_unlimited` 295.4, `tools_compare_dirs` 31.7, `tools_folder_sizes` 79.6, `tools_user_menu` 24.8, `tools_notices_jobs` 23.3 s); `find_feed_panel` FAIL on a stale expectation (the Copy dialog's summary line changed in 142aa74b; see [find.md](find.md)), passed in the rerun below |
| `20261122-191856` (Windows 11, the same tree with the updated `find_feed_panel` check) | `find_feed_panel` | PASS (73.4 s) |
| `20261122-182452` (Linux, origin/main 66a02637 with the Find and tools changes of this record, WSL 2 Ubuntu 24.04, plugin packages built from b1b89343, the shared Cargo target cleaned of every path package before the build) | the same 26 scenarios | all PASS (`about_box` 31.4, `tools_main_menu` 82.4, `tools_help_drive_info` 3.9, `checksums` 16.8, `tools_checksum_algorithms` 17.7, `tools_checksum_manifests` 15.8, `tools_checksum_large_stop` 10.8, `tools_checksum_unlimited` 218.5, `tools_compare_dirs` 75.9, `tools_folder_sizes` 12.1, `tools_user_menu` 32.5, `tools_notices_jobs` 23.4 s; 1335 s in all) |
| `20261122-175928`, `20261122-180347`, `20261122-181712` (Linux, the same tree and build setup; red runs) | `about_box`, `tools_main_menu`, `find_feed_panel`, each new check edited to expect a wrong value (the Copyright text, the Author scan, the label column; the chord order, the terminal's mode; the Copy dialog's bytes) | every edited check failed with its own message |
