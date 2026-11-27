# Acceptance coverage: Find

The inventory of user-visible Find behavior (the Find window, its results, Search inside
archives and Feed to panel), with the acceptance scenario and check that covers each item
and its last verified status. It follows the Find window's code
(`crates/midnight-ui/ui/find*.slint`, `crates/midnight-ui/src/find_windows.rs`,
`find_feed.rs`, `presenter/find*.rs`), the engine (`crates/midnight-engine/src/find*.rs`)
and [find.md](../find.md). The suite is described in [acceptance.md](../acceptance.md);
scenarios live in `crates/midnight-acceptance/tests/acceptance/scenarios/`, and the Find
scenarios share `find_support.rs` there.

Status: **PASS** (the check passed in the run named below), **FAIL** (a product bug, with
its finding), **PENDING** (not checked; the reason is given), **SKIP** (the environment
cannot run it; the reason is given).

Run them with `bash scripts/acceptance.sh --only find`. `find_archives` and
`find_viewers` stage the installed plugin packages (`MIDNIGHT_ACCEPTANCE_PLUGINS`, see
acceptance.md); without them they fail at staging, not skip.

Last runs: the Linux run `20261122-182452` and the Windows runs `20261122-165028` and `20261122-191856` (smoke scale, debug test builds of origin/main 66a02637 with the Find and tools changes of this record; Linux: WSL 2, Ubuntu 24.04 on ext4, the plugin packages built from b1b89343, the shared Cargo target cleaned of every path package before the build; Windows: Windows 11, the release plugin packages built from d7ff8860). Every Find scenario passed in both; on Windows `find_feed_panel` first failed on a stale expectation (F-F19: the Copy dialog's summary line changed in 142aa74b) and passed with the updated check in `20261122-191856`. Statuses are from these runs unless another is named, and the Seconds column is the Linux run's. After d75e82b1 (no default visited-entries limit, 100,000 results, actions on a fed list) `tools_find_results`, `find_feed_panel`, `find_budgets` and `find_archives` were updated and rerun on 24 November: Linux `20261124-181835` and Windows `20261124-183236`, all PASS; their rows' Seconds are from the Linux run. The new checks were each made to expect a wrong value once and failed with their own message (Linux red runs `20261122-093311`, `20261122-094218`, `20261122-095108` and, for the updated F-F19, `20261122-181712`). Earlier: the runs `20261122-100257` (Linux) and `20261122-112449` (Windows) of 26c18895 with these checks, all PASS; the Linux run `20261121-144246` of commit 3f6fff8a (every scenario of the suite) and 2026-11-20 on Linux, smoke scale, with the plugin packages built from the tree of that day.

## Scenarios

| Scenario | Covers | Status | Seconds |
| --- | --- | --- | --- |
| `find_by_mask` | Alt+F7, a mask search, Go to file | PASS | 6.5 |
| `tools_find_criteria` | masks, Regex, Expression, content, Identical files, column widths | PASS | 75 |
| `tools_find_folders_help` | search folders, syntax help, status details | PASS | 23 |
| `tools_find_results` | 100,000 results, Stop, Feed to panel operations, no archive provider | PASS | 25 |
| `find_query_syntax` | masks, Regex and Expression in depth; content encodings; Modified within; Identical files | PASS | 72 |
| `find_roots_windows` | several search folders; ways to open Find; four windows; two searches at once; Escape and Close | PASS | 52 |
| `find_result_actions` | several results selected, Ctrl+C, the context menu, Copy details, sorting, collation, Go to file, View/Edit/Open, stale results, Stop, scrolling, a rebound View key | PASS | 142 |
| `find_feed_panel` | Feed to panel: list, the Find selection carried as marks, clipboard, marks, keys, refusals of the folder-only actions, the other actions on the results, viewers, file operations, right panel | PASS | 29 |
| `find_budgets` | folder depth, content per file, a group over the former result limit, duplicate candidates | PASS | 22 |
| `find_layout` | Blue, Dark and Touch at preferred, narrow and compact sizes; column drags; path tails; the fitted footer and More… (also with 32 px text) | PASS (FIND-COMPACT-STATUS-OVERLAP fixed) | 34 |
| `find_archives` | Search inside archives through the installed plugin, many and large archives, Stop, visited entries | PASS (also at full scale, run `20261121-171419`, 75 s) | 87 |
| `find_viewers` | viewer windows from the results and from a fed list; a viewer keeping what it captured | PASS | 13 |

## Findings

Product bugs the Find checks found. A finding stays listed after its fix, with the run
that verified it.

| Finding | Product bug | Check | State |
| --- | --- | --- | --- |
| FIND-FEED-SELECT-FILES | In a fed list, select files only (Ctrl+A until 27 November, Ctrl+Shift+A since) adds the files to the marks and keeps a marked folder marked; in a panel it replaces the marks with the files | `find_feed_panel` › marks | Fixed: the check passed in the last run |
| FIND-FEED-INVERT | In a fed list, invert files only (Ctrl+I until 27 November, Ctrl+Shift+I since) also unmarks a marked folder; in a panel it keeps folder marks | `find_feed_panel` › marks | Fixed: the check passed in the last run |
| FIND-LAYOUT-NARROW-LABEL | A narrow Find window drew the "Search folders" label with no height | `find_layout` › a narrow window | Fixed: the narrow layout checks and the sideways step pass (Linux run `20261121-144246`) |
| FIND-RESULTS-PATH-TAIL | A shortened Path cell in the results cuts the end of the path, hiding the last folder; the fed list keeps it | `find_layout` › shortened result paths | Fixed: the check passed in the last run |
| FIND-COMPACT-STATUS-OVERLAP | Found on Linux in run `20261121-144246`, in the fix of FIND-COMPACT-STATUS-HIDDEN (F-L04, WIN-7). In a Find window under 360 px high with text over 20 px (Touch, 700×340) the idle status line, pinned to the query area's last line, overlaps the bottom of the scrolled query form by 11 px: "Fewer options" and "Feed to panel" by 216×11 and 212×11 px, and the button's border runs through the status text ("Ready · 0 found · 0 examined"). An empty band of about the same height stays above the form at the query area's top. Suspected cause: `query-scroll` (`find-window.slint`, about line 181) sets `height: parent.height - root.query-status-height` but no `y`, so Slint centres it in the query area and moves it down by half the status line's height; the status line is placed at `y: parent.height - self.height`. The `midnight-ui` test `find_compact_status` checks that the status line is in view, not that the form ends above it. | `find_layout` › Touch compact: layout clean | Fixed: the Touch and Blue 700×340 checks (a clean layout, nothing overlapping; after a search the idle status in the query area with 24 px text, under the results otherwise, and in view; while a search runs, the query area hidden and the status under the results) pass on Linux (runs `20261122-100257`, `20261122-182452`) and Windows (runs `20261122-112449`, `20261122-165028`) |

## Opening Find and its windows

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| F-W01 | Alt+F7 opens Find on the focused panel's folder | `find_by_mask`, `find_roots_windows` | Search folders shows the panel's folder | PASS |
| F-W02 | Ctrl+F opens Find | `find_roots_windows` | a Find window titled "Find files · Midnight Explorer" on the folder | PASS |
| F-W03 | The toolbar's Find button opens Find | `find_roots_windows` | same | PASS |
| F-W04 | F10 › Tools › Find files… opens Find | `find_roots_windows` | same | PASS |
| F-W05 | Find from the right panel searches its folder; Go to file moves that panel | `find_roots_windows` | Search folders value; results; right panel cursor; left panel unmoved | PASS |
| F-W06 | At most four Find windows; a fifth is refused; closing one frees a place | `find_roots_windows` | notice "Find files" / "Close one of the 4 open Find windows first." | PASS |
| F-W07 | Several windows search at once with their own results; Stop ends only its own search | `find_roots_windows` | statuses and rows of both windows | PASS |
| F-W08 | At most two searches walk at once; a third waits until a worker is free | `find_roots_windows` | the third stays Searching with no rows, then completes after a Stop | PASS |
| F-W09 | A search run again with the kept draft finds the same | `find_roots_windows` | same rows twice | PASS |
| F-W10 | The window is modeless: the panels stay usable; the form keeps its draft | `find_roots_windows` | panel Enter/Backspace; masks text kept | PASS |
| F-W11 | Escape closes the Add entry, then the help, then the window | `find_roots_windows` | each Escape closes one | PASS |
| F-W12 | Close during a running search closes the window and frees its session | `find_roots_windows` | window hidden; four new windows fit | PASS |
| F-W13 | Find cannot start inside an archive; the toolbar button is disabled there | `find_archives` | notice "Find files" / "Recursive search is available in native folders.…" | PASS |

## Names: Masks, Regex and Expression

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| F-N01 | Masks `*`, `?`, several masks, `\|` exclusions, folder names, Subfolders off | `tools_find_criteria` | exact result sets | PASS |
| F-N02 | `;;` is a literal semicolon | `find_query_syntax` | `notes;;draft.txt` finds `notes;draft.txt` | PASS |
| F-N03 | `*.*` also matches extensionless names and folders | `find_query_syntax` | equals every entry on disk | PASS |
| F-N04 | Several exclusions after one `\|` | `find_query_syntax` | `*.*\|*.bak;*.tmp` | PASS |
| F-N05 | An excluded folder is still searched | `find_query_syntax` | `*.*\|excluded` lists its file, not the folder | PASS |
| F-N06 | Match name case for ASCII and Unicode letters | `tools_find_criteria`, `find_query_syntax` | `über*`, `ÜBER*` with and without the option | PASS |
| F-N07 | Regex mode with Match name case and alternatives | `tools_find_criteria`, `find_query_syntax` | exact result sets | PASS |
| F-N08 | `(?i)` overrides Match name case | `find_query_syntax` | `(?i)^readme$` with the option | PASS |
| F-N09 | Expression: kind, size, size range, dates, `name like`, exact size | `tools_find_criteria` | exact result sets | PASS |
| F-N10 | Expression: `kB`, `KiB` and `B`; `=~`; `not (…)`; a UTC offset at an inclusive and an exclusive bound; `present`/`missing` | `find_query_syntax` | exact result sets | PASS |
| F-N11 | Expression attributes on Linux (none reported) | `tools_find_criteria` | no match; `missing(attributes)` matches all | PASS |
| F-N12 | An invalid expression fails with its reason; the status line's tooltip and details | `tools_find_folders_help` | Failed, tooltip, details page with Back and Copy | PASS |
| F-N13 | Syntax help per mode, F1, an example fills the query without searching | `tools_find_folders_help` | titles, example text, status unchanged | PASS |

## Containing text

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| F-C01 | Literal text, Match case, Whole words | `tools_find_criteria`, `find_query_syntax` | exact result sets | PASS |
| F-C02 | UTF-8, UTF-8 with BOM, UTF-16 LE and BE with BOM; BOM-less UTF-16 is not decoded | `find_query_syntax` | exact result set; Completed | PASS |
| F-C03 | Unicode letter case | `find_query_syntax` | `KÖLN` finds `Köln`; not with Match case | PASS |
| F-C04 | Whole words: a combining mark and an underscore continue a word | `find_query_syntax` | `cafe`, `foo` not found; `baz` found | PASS |
| F-C05 | Malformed UTF-8 is reported per file, also after a match | `find_query_syntax` | Incomplete, nothing listed, details name both files and 2 errors | PASS |
| F-C06 | Regular expressions per line: anchors, a CR before LF, no match across lines | `find_query_syntax`, `tools_find_criteria` | exact result sets | PASS |
| F-C07 | An invalid content regular expression fails with its reason | `tools_find_criteria` | Failed; details "Invalid content regular expression" | PASS |

## Modified within

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| F-M01 | Minutes, hours and days; a future time never matches | `find_query_syntax` | exact result sets | PASS |
| F-M02 | 366 days at most in each unit | `find_query_syntax` | 8784 hours and 527040 minutes accepted; 367 days, 8785 hours, 527041 minutes refused | PASS |
| F-M03 | Invalid ages show why and disable Find; Enter does not search | `find_query_syntax` | message, Find disabled, status unchanged | PASS |
| F-M04 | The collapsed More options line summarizes the options | `find_query_syntax` | "Modified: last 2 hours · Inside archives" / "· Identical files" | PASS |

## Identical files

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| F-I01 | Groups of empty, small and binary files; a same-size different file and a hard-link pair are not grouped | `find_query_syntax` | group labels and members | PASS |
| F-I02 | Groups and candidates in the status and the details; the group column | `find_query_syntax`, `tools_find_criteria` | "3 groups · 10 candidates" | PASS |
| F-I03 | Groups stay together under Name and Size, both directions | `find_query_syntax` | exact orders | PASS |
| F-I04 | Search inside archives is disabled with Identical files | `find_query_syntax` | check box disabled | PASS |
| F-I05 | A group over the result limit is omitted whole; a later group is listed | `find_budgets` | the pair listed as Group 1; Incomplete "result count" | PASS |

## Search folders and Subfolders

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| F-R01 | Add a typed folder with Enter or OK; Browse… asks the folder chooser | `find_roots_windows`, `tools_find_folders_help` | "Search folders (3)"; entry closed | PASS |
| F-R02 | Tick boxes, the "(k of n)" label, nothing ticked refused | `find_roots_windows`, `tools_find_folders_help` | labels; error line with details | PASS |
| F-R03 | Adding a listed folder again ticks it instead of listing it twice | `find_roots_windows` | three ticked boxes | PASS |
| F-R04 | Relative and parent-step paths are refused; the entry stays open | `find_roots_windows` | the error text; entry open | PASS |
| F-R05 | A missing folder: the others are searched; Incomplete; the details name it | `find_roots_windows` | rows; status; details | PASS |
| F-R06 | Remove the chosen folder | `find_roots_windows`, `tools_find_folders_help` | label count | PASS |
| F-R07 | Sixteen folders at most | `find_roots_windows` | the 17th refused; a search over 16 | PASS |
| F-R08 | Subfolders on and off across several folders | `find_roots_windows` | exact result sets | PASS |

## Results

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| F-S01 | Status line, details page (Copy, Back), error lines that open the details | `tools_find_folders_help`, `find_query_syntax`, `find_budgets` | texts | PASS |
| F-S02 | Every column sorted both ways with its arrow; folders first and missing extensions last under Extension; equal keys by path | `find_result_actions` | exact orders and header arrows | PASS |
| F-S03 | A new search keeps the window's order | `find_result_actions` | exact order | PASS |
| F-S04 | Name order equals the panels' collation | `find_result_actions` | same names, same order | PASS |
| F-S05 | Large result lists; the 100,000 results | `tools_find_results` | every entry of `many/`; 100,000 rows | PASS |
| F-S06 | Every column resized by dragging; a drag never sorts; the width is kept for the next search | `tools_find_criteria`, `find_layout` | widths; order; arrow | PASS |
| F-S07 | Keys in the results; Left and Right scroll sideways | `find_result_actions`, `find_layout` | selected row; header position | PASS (the sideways part with the check adjusted on 21 November; also held on Windows in run 20261121-042048, [windows.md](windows.md)) |
| F-S08 | Go to file by Enter, double click and button, for the cursor's result, files and folders; the window stays open | `find_result_actions`, `find_by_mask` | panel folder and cursor | PASS |
| F-S09 | Open on a folder goes to it | `find_result_actions` | panel folder and cursor | PASS |
| F-S10 | View, F3 and Alt+F3 open the cursor's result in a viewer (Markdown rendered, or its source) | `find_viewers`, `find_result_actions` | the viewer's text and mode; the request's file and plain-text choice | PASS |
| F-S11 | The viewer of a Find result shows only that file: Space walks no further | `find_viewers` | caption and text after Space | PASS |
| F-S12 | Edit and F4 run the configured editor on the result | `find_result_actions` | the editor's copy has the file's bytes; "External editor started" | PASS |
| F-S13 | Open reaches the native launcher | `find_result_actions` | "Desktop action failed" naming it unavailable (no launcher in the harness) | PASS |
| F-S14 | View and Edit are disabled for folders; F3 on a folder goes to it | `find_result_actions` | button states; no launch | PASS |
| F-S15 | A result deleted or changed after the search is refused; the panel does not move | `find_result_actions` | error lines; no launch; no editor | PASS |
| F-S16 | The View and Edit tooltips name their keys | `find_result_actions` | tooltip texts | PASS |
| F-S17 | A new, shorter result list after scrolling a long one shows its rows | `find_result_actions` | rows shown after the new search | PASS |
| F-S18 | The result columns agree with the disk (extension, size or <DIR>, modified) | `find_result_actions` | every row of a search | PASS |
| F-S19 | Copy details in the footer and Copy on the details page put the search's status, errors and counts on the clipboard, the same text both times | `find_result_actions` › Copy details | the copied text starts with the status line and holds the details' counts line; both copies equal | PASS (SKIP before: the clipboard was not installed) |
| F-S20 | Several results selected: a click selects one, Ctrl+click toggles, Shift+click selects the range from the anchor (replacing), Ctrl+Shift+click adds the range; Up with Shift extends the range, Ctrl+Up moves the cursor alone; Space toggles the cursor's row, Insert toggles and moves down; Ctrl+A every result, Ctrl+D none (the cursor stays) | `find_result_actions` › several results selected | the selected rows (their accessible state) and the cursor's row (its drawn edge) after each step | PASS |
| F-S21 | The status line counts the selection and sums its files: " · 1 selected · 10 B", " · 2 selected · 310 B", " · 2 selected · 1 B" with a folder; no " · … selected" without a selection | `find_result_actions` › several results selected | the status line's selection part | PASS |
| F-S22 | Ctrl+C copies the selection's full paths, one per line in the order shown, when the cursor's row is in it, else the cursor's row alone (only that copy) | `find_result_actions` › several results selected | the in-process clipboard's text and its count of copies | PASS |
| F-S23 | F3 with five results selected views the cursor's file only | `find_result_actions` › several results selected | the viewer request's file | PASS |
| F-S24 | A re-sort keeps the selection (by Size and back to Name); a new search clears it | `find_result_actions` › several results selected | the selected names; the status line | PASS |
| F-S25 | The context menu: Go to file, View, Edit, Open, Copy names, Copy full paths, Copy row details, Select all and Deselect all, the copies naming the count (" (2)") and the keys shown (Enter, F3, F4, Ctrl+C, Ctrl+A, Ctrl+D); it opens with Shift+F10, the Menu key and a right click; Escape closes only the menu; a right click on a selected row keeps the selection, on an unselected one selects it alone; on a folder View and Edit are disabled | `find_result_actions` › the results' context menu | the menu's entries, states and texts; the selection | PASS |
| F-S26 | The menu's actions: Copy names, Copy full paths and Copy row details (name, extension, folder, size and modified, tab-separated) of the selection or of the row; View and Edit on its row; Select all, Deselect all; Go to file on a folder | `find_result_actions` › the results' context menu | the clipboard's text (one copy each), the viewer request, the editor's copy, the selection, the panel | PASS |
| F-S27 | View rebound in Options (O-X04, here Ctrl+Alt+V in the settings): in the results the new key views the cursor's file and F3 does nothing; the View tooltip and the context menu name the new key | `find_result_actions` › a rebound View key | tooltip; the menu's key text; the viewer requests after F3 on one result and the new key on another | PASS (O-X04) |

## Stop

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| F-T01 | Stop cancels; what was found is kept and sorted; Feed to panel enabled again | `find_result_actions`, `tools_find_results` | Cancelled · 20 found, rows e00…e19 | PASS |
| F-T02 | Feed to panel is disabled before a search and while one runs | `find_result_actions` | button state | PASS |
| F-T03 | A new search replaces the results | `find_result_actions` | rows e00…e09 | PASS |
| F-T04 | While a search runs the window names the folder it is reading; the name goes when the search ends | `find_result_actions` | a text naming the folder of the long reads while Searching, none after Stop | PASS (the corrected check: "Scanning: <folder>") |

## Budgets

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| F-B01 | Result count (100,000; 10,000 before d75e82b1) | `tools_find_results` | 100,050 files: Incomplete, 100,000 rows; "result count" | PASS |
| F-B02 | Folder depth (128 retained levels) | `find_budgets` | the files of 127 levels; "folder depth" | PASS |
| F-B03 | Content read per file (1 GiB) | `find_budgets` | the small file only; "content read per file"; 1 size-limited file | PASS |
| F-B04 | Duplicate candidates (100,000) | `find_budgets` | "duplicate candidates"; 100,000 candidates | PASS |
| F-B05 | No visited-entries limit by default (1,000,000 before d75e82b1), archive members included | `find_archives` | eleven archives of 99,910 members: Completed, 1,099,021 examined, no "visited entries" | PASS |
| F-B06 | Retained memory (64 MiB) | – | – | PENDING: no fixture reaches it in reasonable time and the window cannot lower it |
| F-B07 | Total content read (16 GiB) | – | – | PENDING: it needs 16 GiB actually read and the window cannot lower it |
| F-B08 | An identical group larger than the former 10,000-result limit is listed whole beside a later pair; at default limits no group can exceed the result limit, as the 100,000 duplicate candidates come first (the engine's `an_omitted_oversized_group_does_not_stop_later_groups_that_fit` lowers the limit to omit a group whole) | `find_budgets` | Completed; 2 verified groups, 10,003 candidates and rows; each group's rows under one label | PASS |

## Search inside archives (installed archive plugin)

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| F-A01 | Members of a ZIP, a tar and a tar.gz by name; the Path column shows the archive path | `find_archives` | exact result set; size; 4 archives in the details | PASS |
| F-A02 | Without the option nothing inside is found | `find_archives` | no rows | PASS |
| F-A03 | Unicode member names and member folders | `find_archives` | exact result sets | PASS |
| F-A04 | A member archive matches by name and is not opened | `find_archives` | exact result set | PASS |
| F-A05 | Content in members (literal, per-line regex) and in a gzip stream's member | `find_archives` | exact result sets | PASS |
| F-A06 | An expression over members and files | `find_archives` | exact result set | PASS |
| F-A07 | Modified within: tar times carry a zone, ZIP times do not | `find_archives` | exact result set | PASS |
| F-A08 | A broken archive is reported and the search goes on | `find_archives` | Incomplete; the other file found; details name it | PASS |
| F-A09 | Go to file opens the archive at the member; Backspace returns | `find_archives` | path bar, cursor; back in the folder at the archive | PASS |
| F-A10 | View, Edit and Open are disabled for members; F3 explains | `find_archives` | button states; error line | PASS |
| F-A11 | Feed to panel leaves members out and counts them; only members are refused (since d75e82b1 by the backend's feed preparation, whose details add "Operation: feed search results") | `find_archives` | list rows and status; error line | PASS |
| F-A12 | 300 ZIPs and one of 20,000 members, all searched | `find_archives` | 301 rows; 301 archives and 20,600 members in the details | PASS |
| F-A13 | Stop ends a search inside archives promptly while a panel opens an archive | `find_archives` | Cancelled within 5 s; the panel lists the archive | PASS |
| F-A14 | An archive over 100,000 entries is searched completely | `find_archives` | the last member found | PASS |
| F-A15 | Without an archive provider the search says so | `tools_find_results` | Incomplete; details | PASS |
| F-A16 | Subfolders limits ordinary folders only: with it off, an archive in a searched folder is searched completely and one in an ordinary subfolder is not reached | `find_archives` | exact result sets with Subfolders off and on | PASS |
| F-A17 | Several search folders with archives at once | `find_archives` | members of one folder's archives and the other folder's file; Incomplete for the broken archive | PASS |

## Feed to panel

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| F-F01 | The list: the Find window's order, title, status (the focused result's facts, then the counts), Folder column, folder rows | `find_feed_panel` | exact rows and texts | PASS |
| F-F02 | Cursor keys | `find_feed_panel` | cursor after Down, End, Up, Home, Page Down, Page Up | PASS |
| F-F03 | Alt+Right and Ctrl+R do nothing in the list | `find_feed_panel` | list unchanged, no notice | PASS |
| F-F04 | Insert marks and moves; Space marks and moves (since d75e82b1, measuring a folder too); the status counts and sums | `find_feed_panel` | marks, cursor, status | PASS |
| F-F05 | Ctrl+A, Ctrl+Shift+A, Ctrl+D, Ctrl+Shift+D, Ctrl+I, Ctrl+Shift+I with the keys' panel meaning; keypad * like Ctrl+Shift+I | `find_feed_panel` | marks after each key from a known state | Not run since the 27 November selection key change (Ctrl+Shift+D is new). With the earlier keys (plain key files only): PASS, the folders case included (keypad * as Ctrl+Shift+I once Select by mask last had Include folders, `find_feed.rs` since 4d788480); also PASS on Windows in run 20261121-024756 ([Windows results](windows.md#results)) |
| F-F06 | Ctrl+click, Shift+click, Alt+Down and Alt+Up | `find_feed_panel` | marks and cursor | PASS |
| F-F07 | Enter, Ctrl+Page Down and a double click go to a file or open a folder | `find_feed_panel` | panel folder and cursor; list closed | PASS |
| F-F08 | Backspace, Alt+Left, Ctrl+Backspace, Shift+Backspace and Back to folder leave the list; Ctrl+G navigates away; Tab keeps it | `find_feed_panel` | the folder shown equals the disk | PASS |
| F-F09 | F3 and Alt+F3 view the focused file with the list's files; F4 edits it; a folder is explained | `find_feed_panel` | the request's file, files and plain-text choice; editor copy; notices | PASS |
| F-F10 | Alt+Enter summarizes the marked results | `find_feed_panel` | a properties notice, not a refusal | PASS |
| F-F11 | F5 copies the marked results; F6 moves the focused one and the list keeps its row | `find_feed_panel`, `tools_find_results` | files on disk; rows | PASS |
| F-F12 | F8, Delete and Shift+Delete ask; Shift+F8 deletes a marked folder and the result inside it | `find_feed_panel`, `tools_find_results` | dialogs; disk; rows | PASS |
| F-F13 | The actions that need one folder explain themselves and change nothing: F7, Shift+F4, Ctrl+V, Ctrl+F10 (since d75e82b1 the others act on the results, F-F22) | `find_feed_panel` | notice title and text per key; folder unchanged | PASS |
| F-F14 | The toolbar, the function bar and the main menu refuse the same way (New empty file, F7, F10 Create folder…) | `find_feed_panel` | same | PASS |
| F-F15 | A custom command from the toolbar runs with the list's paths (refused before d75e82b1) | `find_feed_panel` › actions on the results | the marker file it creates | PASS |
| F-F16 | Ctrl+C and Ctrl+X put the marked results on the clipboard, to copy and to move | `find_feed_panel` › the Find selection carried to the list | the clipboard's files and intent | PASS (SKIP before: the clipboard was not installed) |
| F-F17 | A Find from the right panel feeds the right panel | `find_feed_panel` | rows and title in the right panel | PASS |
| F-F18 | In a viewer opened from the list, Space and Backspace walk the list's results; Alt+F3 shows the source | `find_viewers` | the viewer's text after each key | PASS |
| F-F19 | The Find selection arrives as the list's marks: three results Ctrl+clicked (" · 3 selected · 11 B"), all six listed, exactly those marked ("3 of 6 selected · …"), the list's cursor on the Find's cursor result; F5's Copy dialog reads "Copy 3 files (11 B) to:", its source "Search results" (cancelled; before 142aa74b it named "3 selected search results"); after a plain click only, nothing is marked and the cursor is on that result | `find_feed_panel` › the Find selection carried to the list | marks, cursor, status, the dialog's text | PASS |
| F-F20 | With no result chosen in the Find window, the list's cursor starts on its first row | `find_feed_panel` › the list | the list's cursor | PASS |
| F-F21 | A viewer opened from the results keeps the content it captured when the file changes afterwards; a new View of the changed result is refused and opens nothing | `find_viewers` › from the results | the open viewer's text; the error line; the viewer count | PASS |
| F-F22 | The other actions act on the list's rows, here the focused `src/sub/g.txt` that the folder underneath does not show: F2 renames it, Ctrl+F2, Ctrl+F7 and the toolbar's Attributes button capture it, Ctrl+F8 and F10 Checksums… calculate it alone, Ctrl+M previews it, Ctrl+Enter inserts its full path, Ctrl+Shift+S measures a folder result, Ctrl+F6 twice, F10 Sort by name and Ctrl+F3 sort the list, Ctrl+= and Ctrl+− mark by mask and Num + and Num − open those dialogs, Ctrl+F12 opens the filter, Ctrl+H toggles the list's own Show hidden files, Ctrl+Shift+C is not refused; each is cancelled and the folders are unchanged | `find_feed_panel` › actions on the results | the rename dialog's name, the Attributes summary, the Checksums rows, the Make File List preview, the command line, the row's size, the order, the marks, the dialogs | PASS |
| F-F23 | Compare files from a fed list takes the focused result as File A and asks for File B | `viewers_combinations` › Find results and a fed list | the comparator's File A (`sub/c.txt`) and empty File B | PASS |

## Size, density and theme

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| F-L01 | No clipped or overlapping controls in Blue, Dark and Touch (24 px text) at the preferred, a narrow and a compact size | `find_layout` | the UI review's layout checks | PASS |
| F-L02 | Narrow: Remove under Add; Browse… and OK under the typed path | `find_layout` | control positions | PASS |
| F-L03 | Compact: the field is "Query"; a running search hides the query area; the idle status shows under the results | `find_layout` | label; field hidden; status under the results and in view | PASS (Blue compact) |
| F-L04 | Compact with text over 20 px: the idle status moves into the query area and shows there | `find_layout` | status position; a status line in view after the search | PASS (Linux `20261122-100257`, `20261122-182452`; Windows `20261122-112449`, `20261122-165028`). Before: FAIL with FIND-COMPACT-STATUS-OVERLAP (Linux run `20261121-144246`, Touch 700×340), and earlier FIND-COMPACT-STATUS-HIDDEN (Windows run 20261121-042048, [windows.md](windows.md)) |
| F-L05 | A shortened Path cell keeps the last folder, as the fed list's Folder cells and other shortened paths do | `find_layout` | every shortened Path cell at 560 px shows its `/folder` tail | PASS |
| F-L06 | The fitted footer: the buttons and the More… entries are the page's whole set once each (Find, Go to file, View, Edit, Open, Copy details; on the details page Find, Copy, Back); the actions in More… are the first ones of the order Copy details, Open, Edit, View, Go to file (details: Copy, Find); every button, More… and Close included, is whole in the window and 8 px from the next. At 1000×620 every action is a button; at 560×420 some move; at 320×200 the query page shows Find and More…, the details page Back and More… | `find_layout` › the footer and More… | the footer's buttons, the menu's entries (also those its short list scrolls out of view), rectangles; layout findings naming them | PASS |
| F-L07 | More… from the keyboard: Tab reaches it, Space opens it, Enter runs its first action (Go to file, with a result under the cursor), Escape closes only the menu | `find_layout` › the footer and More… | focus; the panel at the result; the menu and window state | PASS |
| F-L08 | The largest text (32 px) at 320×200: the query page shows Find and More…, the details page Back and More…, each whole and 8 px apart (on Linux Back used to be cut there; fitted footers since be086db9) | `find_layout` › the footer with 32 px text at 320×200 | as F-L06 | PASS (with be086db9) |

## Not offered by Find

- A history of earlier queries: each Find window keeps its own draft, and a new search
  starts from it (F-W09).

## Harness notes

- **Viewers from Find.** The staged `find_viewers` scenario opens the viewer windows that
  View, F3 and Alt+F3 ask for, through the same `refresh-find-viewer` hook production
  installs, and checks their text and mode. The scenarios that run without plugins
  (`find_result_actions`, `find_feed_panel`) record each request through that hook
  (`find_support::record_launches`) and check its file, its plain-text choice and, for a
  fed list, the files it walks.
- **Archive plugin limits.** Find sets no limits of its own on archives. The archive
  plugin set used for the record has no entry limits, so F-A14 searches an archive of
  120,000 members completely and requires its last member. Archives over 65,535 members
  are written with ZIP64 end records (`find_archives.rs`), as the format requires.
- **Clipboard.** `find_result_actions` and `find_feed_panel` install the in-process
  clipboard (`app::MemoryClipboard`, `Config::clipboard`) where the application installs
  the desktop one. A copy is told apart from the one before it by the clipboard's count
  of copies (`tools_support::clipboard_writes`, `wait_copied`), not by its text, so the
  same text copied again still counts; "only this copy" checks a known copy made after
  the action (the Engine takes the presenter's copies in order) instead of waiting a
  fixed time.
- **The cursor's row.** The results expose each row's selected state to accessibility,
  not the cursor. `tools_support::cursor_row` reads it from the drawn list: the cursor's
  row alone has the full-height stroke at its left edge (a selected row's square sits
  in the middle of the edge only). Each check that reads it first confirms it on a row
  just clicked.
