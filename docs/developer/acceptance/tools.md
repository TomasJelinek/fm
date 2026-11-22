# Acceptance coverage: tools

The inventory of the F10 main menu, the F9 User menu, Checksums, Help, Drive
information, About, the Jobs window's list and notice dialogs, with the acceptance
scenario and check that covers each item and its last verified status. Options pages,
appearance, the keyboard editor and the shortcut catalog are in
[options-tools.md](options-tools.md). The suite is described in
[ACCEPTANCE.md](../ACCEPTANCE.md); scenarios live in
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
| O-N03 | Copy details reaches the clipboard | `tools_notices_jobs` | command sent; "Copied to clipboard" | PASS on Linux in run `20261121-144246` and on Windows in run 20261121-040741 ([Windows results](windows.md#results)): Copy details starts with the title, reason and folder (finding O-5, of 20 November, fixed) |
| O-N04 | Close, Escape and the header's close button close it; no other notice follows | `tools_notices_jobs` | notice token for one second | PASS |
| O-N05 | A very long folder path: dialog inside the window, Close in view, the path box scrolls to its end | `tools_notices_jobs` | rectangles; wheel scrolling | PASS |

## Findings

The finding numbers are shared with [options-tools.md](options-tools.md#findings), which
lists O-1, O-2 and O-4.

| ID | Finding | Check |
| --- | --- | --- |
| O-3 | About › Licenses: each row's "›" open mark and its click area lie in the row's last 26 px, under the list's 14 px overlay scrollbar; a click in the middle of the mark does nothing, only its left edge opens the library. | `about_box` |
| O-5 | A folder notice's Copy details copies the title, folder and raw error but not the reason the dialog shows ("Access to this folder was denied."), although its hint promises the complete message. Fixed: the check passes on Linux (run `20261121-144246`) and Windows (O-N03). | `tools_notices_jobs` |

## Host runs

Runs (UTC) on the Linux host with the smoke-scale fixture, the debug acceptance binary of the
shared build and a complete plugin set:
`MIDNIGHT_ACCEPTANCE_RUN_DIR=<run folder> MIDNIGHT_ACCEPTANCE_PLUGINS=<plugin set>
_build/<target>/debug/deps/acceptance-<hash> --scale smoke <scenario>...`

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
