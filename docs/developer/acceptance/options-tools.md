# Acceptance coverage: Options, appearance and keyboard

The inventory of user-visible Options pages and controls and of the keyboard shortcut
catalog and its hints, with the acceptance scenario and check that covers each item and
its last verified status. The F10 main menu, the F9 User menu, Checksums, Help, Drive
information, About, the Jobs window's list and notice dialogs are in
[tools.md](tools.md). The suite is described in [acceptance.md](../acceptance.md);
scenarios live in `crates/midnight-acceptance/tests/acceptance/scenarios/`.

Status: **PASS** (the check passed in the run named at the end), **FAIL** (a product
bug, with its finding), **PENDING** (not verified yet; the reason is given), **SKIP**
(the environment cannot run it; the reason is given).

## Options dialog

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-D01 | The toolbar's gear opens Options when its settings are ready | every `options_*` | `options_open`, `preferences_ready`, not busy | PASS |
| O-D02 | F10 › Options opens it | `tools_main_menu` | `options_open` | PASS |
| O-D03 | The page list switches pages | every `options_*` | topic index per page | PASS |
| O-D04 | A narrow window shows a topic picker instead of the list, and works | `options_themes`, `options_visual` | picker present, list absent; a theme chosen and saved through it; its list in the displayed order; Shift+Tab from OK reaches it; Down and Up walk every page with the focus kept, stopping at the ends | PASS (Windows, Linux) |
| O-D13 | The page list by keyboard (dialog-ergonomics.md, "Options page order"): Options opens on General with the focus on OK; Shift+Tab from OK reaches the list at its last row; Space and Enter choose the focused row's page (Enter keeps Options open); Up walks to General and Down to Plugins in the displayed order (topics 1, 8, 6, 7, 4, 10, 9, 3, 5), the focus following and both stopping at the ends | `options_general` | topic index, the focused row, its checked state, saves | PASS (Windows, Linux) |
| O-D14 | Options reopens on the page shown last in a session; a restart opens General | `options_general` | topic index after Escape and reopening, and after a restart | PASS (Windows, Linux) |
| O-D15 | Enter on a focused combo box (Disk I/O concurrency) opens its list and neither confirms nor saves Options; Escape closes only the list | `options_general` | popups, Options open, saves, the combo's value | PASS (Windows, Linux) |
| O-D16 | A low narrow window (Options compact) puts the topic picker in the header: the displayed order, Shift+Tab reaches it, Down opens it on the shown page, Down moves, Enter chooses (Keyboard) and closes it, the focus back on the picker | `options_themes` | listed rows, topic index, popups, focus | PASS (Windows, Linux) |
| O-D17 | Up and Down act on the focused row of the page list, not on the page shown (decided 22 November, U-D29 in [ui-polish.md](ui-polish.md)): with Plugins shown, Shift+Tab focuses Keyboard without choosing it, and Up there shows Custom Commands with the focus on it. A failure here is reported after the scenario's other checks have run | `options_general` | topic index, the focused row | PASS on Windows (`20261122-174336`) and Linux (`20261122-201240`), d96dec97; its red variant, which expects Keyboard, FAIL (see "Red runs"). Before d96dec97 Up showed Keyboard (recorded in Windows runs `20261122-092401` and `20261122-124508`) |
| O-D05 | OK saves and closes | every `options_*` | settings store saves | PASS |
| O-D06 | Cancel discards previews and closes | `options_general`, `options_appearance`, `options_colors`, `options_keyboard`, `options_themes` | previews restored, no save | PASS |
| O-D07 | Escape discards like Cancel | `options_general`, `options_buttons` | preview restored, no save | PASS |
| O-D08 | Enter confirms like OK | `options_general` | one save, closed | PASS |
| O-D09 | The header's close button discards and closes | `options_buttons` | Options closes, nothing saved | PASS |
| O-D10 | Reset to defaults restores every page's defaults and keeps tabs, Hot Paths and Custom Commands | `options_general`, `options_appearance` | shown values; saved values, the tabs, a seeded Hot Path (slot 2) and a seeded Custom Command (history is not checked: the scenario cleared it before) | PASS (Windows, Linux) |
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
| O-G07 | Remember command history after restart | `options_general` | positive control: the first start loads the seeded history as a restart loads remembered history (the "Executed command history" picker enabled, Alt+Up recalls the newest command); after clearing, remembering off and a restart the picker is disabled and Alt+Up, pressed for 2 s, recalls nothing | PASS (Windows, Linux) |
| O-G08 | Clear command history (enabled only with history) | `options_general` | enabled states; saved history empty | PASS |

## Settings file (Export and Import)

Through the scripted file chooser (every answer queued first, inside the scenario's
folder) and the Engine's own settings-file workers.

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-I01 | Export configuration writes what OK would save now, a previewed change included: "Exported settings to <path>." | `options_settings_file` | the status line; the file holds the previewed byte sizes and the Hot Path | PASS (Windows, Linux) |
| O-I02 | Exporting onto an existing file asks "… already exists. Replace it?": Keep leaves it byte for byte ("… was kept. Settings were not exported."), Replace writes the current settings and keeps the replaced file for recovery | `options_settings_file` | status lines; file bytes; the recovery copy's bytes | PASS (Windows, Linux) |
| O-I03 | Import configuration previews the file's settings ("Imported <name>. … Cancel keeps your current settings."): Cancel restores the theme and saves nothing; OK saves them and applies the file's Hot Paths (the number row's 4 opens the imported one) | `options_settings_file` | status line; theme; saved settings; the panel's folder | PASS (Windows, Linux) |
| O-I04 | Refused files leave the settings unchanged with the reason: not JSON, no settings version, a newer version | `options_settings_file` | "Could not import <name>: <reason> Your settings are unchanged."; saves | PASS (Windows, Linux) |
| O-I05 | A file from the other operating system keeps its Hot Path and its editor inactive ("Kept inactive because they use <system> paths: 1 hot path and the editor."): after OK the Hot Path's number says why it does not open, the Hot Paths manager lists it and removes it; General lists the editor, and Remove then OK drops it | `options_settings_file` | status line; saved inactive Hot Paths and settings; the notice; the manager's and the General page's rows | PASS (Windows, Linux) |

## Appearance

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-A01 | The active appearance previews immediately and Cancel restores it | `options_appearance`, `options_themes` | active palette, panel luminance and saved choices | PASS |
| O-A02 | The dark and light menus offer every palette of their appearance. The last pick, dark or light, is previewed whatever the host appearance; OK, Cancel and a restart show the choice matching the host | `options_themes` | accessible popup lists, rendered panel colors, previews on the other appearance, OK, Cancel and restart | Not run since the preview change (passed in its earlier form, where an inactive pick waited for its host appearance) |
| O-A03 | Spacing (desktop, touch) | `options_appearance` | row height, density | PASS |
| O-A04 | Font family | `options_appearance` | rendered panel text changes | PASS |
| O-A05 | Desktop and touch font sizes | `options_appearance` | row height, rendered text | PASS |
| O-A06 | Text rendering (Sharp, Balanced, Smooth) | `options_appearance` | stored and shown after a restart (the software renderer ignores it) | PASS |

## Colors, Previews, Archives, Editors, Comparators, Custom Commands

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-L01 | Colors: an added rule colours matching names; priority by Move up; Cancel; OK; restart | `options_colors` | row colours, rendered pixels, saved rules | PASS |
| O-L02 | Colors stay on their rows sorted by size and filtered by a panel filter | `options_colors` | row colours, rendered pixels | PASS |
| O-L03 | Colors: the colour picker keeps OK disabled while it is open; a swatch (`#ef4444`) and Use color set the foreground; Escape closes only the picker and keeps the value | `options_colors` | OK's enabled state, the hex field, the row colour, Options open | PASS (Windows, Linux) |
| O-L04 | Colors: each rule field changes what the panel shows after Apply colors: Regex (`^val.*\.dat$`), Case sensitive (`*.DAT`), the Folders target, Enable rule off, a background colour; Discard changes returns an unapplied edit to the applied rule; Move down; Cancel discards all of it | `options_colors` | row foreground and background colours, the expression, rule labels, saved rules | PASS (Windows, Linux) |
| O-L05 | The filename colours as shown: `.rs` sky blue `#23AAF2` (since 23 November 2026; cyan `#06A4C7` before), `.docx`, `.md` and `.txt` blue `#3B82F6` (since f153478c), each rendered as the shared readability policy (`midnight_core::readable_rgb`) makes its base colour on the row it is drawn on, at least 4.5:1, in Blue, Dark and Light (filename-color-options.md) | `options_palette` | each name's model colour; its rendered glyph colour against the policy's value for the measured row | Not run with `#23AAF2` yet. With cyan `#06A4C7`: PASS (Windows, Linux; rendered as the policy gives them: Blue `#06A4C7`/`#3B82F6`, `#3E84F6` on its alternate rows; Dark `#3D84F6` and `#4689F6` for the blue; Light `#3473DB`/`#326ED1` and `#057B96` for the cyan, all 4.50:1 or more) |
| O-L06 | Saved lists on loading (the real settings store, in a private child): the palette shipped until 22 November (blue source, neutral documents and text), the interim all-cyan one and the one shipped until 23 November (cyan source `#06A4C7`, blue documents and text) move to the defaults; a customised list is kept, except its document rule still exactly as the interim build set it, which turns blue (its cyan source rule, green text rule and changed font colour stay) | `options_palette` | model colours after each restart | Not run in this form yet (the 23 November step and the cyan source rule of the customised list are new). Before, without them and with a blue source rule in the customised list: PASS (Windows, Linux) |
| O-V01 | Previews: switches, thumbnail size seen in the Thumbnails view; Cancel, OK, restart | `options_previews_plugins` | row height; saved values | PASS |
| O-V02 | Previews: Clear thumbnail cache | – | the thumbnail service exists only in staged viewer processes; the button would clear a real cache | SKIP |
| O-R01 | Archives: custom extension chips (add, fold, remove, invalid, built-in conflict), an association; Cancel, OK, restart; Escape in the field | `options_archives` | chips, errors, saved values | PASS |
| O-E01 | Editors: a fallback and a `*.md` rule; F4 runs the matching one; restart | `options_editors` | file written by the editor command | PASS |
| O-E02 | Editors: a rule without `{file}` is refused | `options_editors` | error line | PASS |
| O-C01 | Comparators: built-in and external overrides saved, listed after a restart | `options_comparators` | rule rows, saved values | PASS |
| O-C02 | Comparators: an invalid regular expression is refused | `options_comparators` | error line | PASS |
| O-U01 | Custom Commands: create, edit, drag, delete, separators, placeholders; runs; failure notice; Preview; key; restart | `options_custom_commands` | files written, notices, saved commands | PASS |
| O-U02 | F9 lists the Custom Commands in the saved order | `options_custom_commands` | the User menu's command buttons | PASS (Windows, Linux) |
| O-U03 | A right click on a toolbar command offers "Custom Commands settings…", which opens Options › Custom Commands with that command selected | `options_custom_commands` | the context menu's text; the page; the form's name | PASS (Windows, Linux) |
| O-U04 | The command list's keys: Alt+Up and Alt+Down move the selected command, Delete removes it, Insert starts a new one; Cancel discards them | `options_custom_commands` | list order, the form, saved commands | PASS (Windows, Linux) |
| O-U05 | Escape during a drag in the command list cancels only the drag; the next Escape closes Options | `options_custom_commands` | order, Options open, saves | PASS (Windows, Linux) |
| O-U06 | A terminal-mode command: its dry run names the shell and the literal script | `options_custom_commands` | preview text. Running it is SKIP: the harness installs the native launcher, which would open a real terminal window | PASS (Windows: the dry run; running it SKIP. Linux in WSL, without a supported terminal: the preview reports "No supported system terminal is installed", SKIP of the dry run) |
| O-U07 | A disabled command's toolbar button is disabled and a click runs nothing | `options_custom_commands` | enabled state; no file, no notice within 2 s (the enabled per-item command wrote its files at once) | PASS (Windows, Linux) |
| O-H01 | Hot Paths manager: add, exchange numbers, remove; Ctrl+digit; restart | `options_hot_paths`, `files_hot_paths` (panels) | panel folders, saved Hot Paths | PASS |

## Keyboard

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-K01 | Search commands and select one | `options_keyboard`, `shortcuts_hints` | the command's id | PASS |
| O-K02 | The selected command's shortcuts picker | `options_keyboard` | aliases shown | PASS |
| O-K03 | Capture a key; Assign | `options_keyboard`, `shortcuts_hints` | captured text; aliases | PASS |
| O-K04 | Remove an alias | `options_keyboard`, `shortcuts_hints` | aliases | PASS |
| O-K05 | An occupied key names its owner and asks (Replace, Cancel) | `options_keyboard` | "Shortcut currently used by"; Replace shortcut dialog | PASS |
| O-K06 | Restore command | `options_keyboard` | default back; override dropped | PASS |
| O-K07 | Restore all defaults asks first: Keep custom shortcuts changes nothing, Reset all shortcuts restores the default map, which OK saves (no override left; F5 opens Copy and F7 Create folder again) | `options_keyboard`, `options_buttons` | the question, the command's shortcuts, saved overrides, the keys' effect | PASS (Windows, Linux) |
| O-K08 | The rebound key works, the old one does not, also after a restart | `options_keyboard`, `shortcuts_hints` | Create folder dialog | PASS |
| O-K09 | Hints follow a rebinding: F10 menu, F1 help, footer | `shortcuts_hints` | shown keys | PASS |
| O-K10 | Tab or Escape in the capture field is recorded, not run | `options_keyboard` | captured text; Options stays open | PASS |
| O-K11 | Reserved keys (Ctrl+Space) and a fifth alias are refused with a reason | `options_keyboard` | "reserved" and "At most four" reasons; shortcuts unchanged; Cancel discards | PASS |
| O-K12 | The Replace shortcut question opens with the keyboard on its Cancel; Enter there and Escape decline it, closing only the question | `options_keyboard` | focus, the question, Options open, the command's shortcuts | PASS (Windows, Linux) |
| O-K13 | Replace by keyboard (Tab to Replace, Enter) then OK saves: F5 moves from Copy to Create folder, which keeps F7 | `options_keyboard` | shortcuts shown and saved; F5 opens Create folder, not Copy | PASS (Windows, Linux) |
| O-K14 | Clear empties a captured key and is disabled again (Assign too) | `options_keyboard` | the capture field, enabled states | PASS (Windows, Linux) |

## Plugins

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-P01 | Plugins: every section's empty state without staged plugins (asserted for Viewers, Filesystems, Archives and Thumbnailers); Rescan; the Archives link | `options_previews_plugins` | texts; page switch | PASS (Windows, Linux) |
| O-P02 | Plugins with the installed packages (a staged child of the same scenario): every section lists them instead of its empty state, at least one row per viewer and per thumbnailer package and the filesystem packages across Filesystems and Archives; Rescan reports "Rescanned: found …" with the counts the sections list and no unavailable package | `options_previews_plugins` | rows per section against the staged packages; the rescan report | PASS (Windows, Linux) |

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
| O-S19 | Clear comparison (`panel.compare-clear`, b70cf8df): Escape and Ctrl+Shift+F11 after Compare directories with subfolders; Escape with nothing compared does nothing; Escape during a filename search ends the search and keeps the badges; the footer's Ctrl+Shift+F11 cell | both panels' badges, the result line, the selection; `shortcuts_footer` the same probe | PASS on Linux (`20261124-183335`) and Windows (`20261124-183236`) |

## Shortcut hints and the footer

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| O-F01 | F1 help lists every catalog action with all its keys or "Unassigned" | `shortcuts_hints` | one line per action | PASS |
| O-F02 | F10 shows every key of each item's action in binding order, as F1 lists them (since 22f46eab): "Ctrl+= / Num + / Ctrl++" beside Select by mask, "Delete / F8" beside Move to Trash, "Num *" beside the invert, nothing beside Options | `shortcuts_hints` | the key texts of every item against the catalog, and nine written out | PASS on Windows (`20261122-200459`) and Linux (`20261122-201240`), d96dec97; in Windows run `20261122-192050` the check still expected one key per item, as the menu showed before 22f46eab |
| O-F03 | Footer F1–F12 in the plain, Ctrl, Shift, Alt and Ctrl+Shift layers: a click does what the key does | `shortcuts_footer` | the key's probe after the click | PASS |
| O-F04 | Footer cells without an action are disabled and do nothing | `shortcuts_footer` | disabled; state unchanged | PASS (after the fix for finding O-2) |
| O-F05 | Footer F11/F12 (network) hidden on Linux | `shortcuts_footer` | no cell | PASS |
| O-F06 | After a rebinding the footer cell becomes unassigned | `shortcuts_hints` | "F7 — unavailable" | PASS |
| O-F07 | The panel's View and Sort menus show every key of each item's action (as F1 lists them, "Ctrl+F6 / …"); after Sort by size gets a second key they list both | `shortcuts_hints` | each row's key text against the catalog, before and after the change | PASS (Windows, Linux) |

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

Findings O-3 (About) and O-5 (notices) are in [tools.md](tools.md#findings).

| ID | Finding | Check |
| --- | --- | --- |
| O-1 | On Linux, Shift+letter outside a filename search is taken by the Windows drive-letter action, finds no drive and is swallowed: typing an uppercase letter starts no search (during a search the same key is typed). | `shortcuts_catalog`, scope probe "Shift+R" |
| O-2 | A click on an unavailable (disabled) F-key cell of the footer takes the keyboard focus from the file list, so arrow keys and typing reach no panel until the list is clicked; enabled cells give the focus back. | `shortcuts_footer` |
| O-4 | Options buttons that are enabled but change nothing: Keyboard › Clear with nothing captured, Keyboard › Restore command for a command at its defaults, Colors › Discard changes without changes, Plugins › Rescan all plugins (no feedback without plugins). | `options_buttons` |

## Host runs

Windows run `20261125-160213`, on the theme changes based on 0.6.0, passed
`options_appearance` and `options_themes`. The two scenarios selected all 46
palettes, switched the host appearance during preview, cancelled edits, saved
both choices and reopened Options on the other appearance.

Runs (UTC) on the Linux host with the smoke-scale fixture, the debug acceptance binary of the
shared build and a complete plugin set:
`MIDNIGHT_ACCEPTANCE_RUN_DIR=<run folder> MIDNIGHT_ACCEPTANCE_PLUGINS=<plugin set>
_build/<target>/debug/deps/acceptance-<hash> --scale smoke <scenario>...`

Several of these runs also ran tools scenarios; their results are in
[tools.md](tools.md#host-runs).

| Run | Scenarios | Result |
| --- | --- | --- |
| 2026-11-20 14:25 | `options_general`, `options_appearance`, `options_custom_commands`, `options_keyboard`, `options_colors`, `options_archives`, `options_editors`, `options_comparators`, `options_previews_plugins`, `options_hot_paths` | all PASS (16.8, 17.7, 49.6, 21.5, 13.3, 14.5, 11.4, 14.1, 10.6, 17.4 s; 315 s in all with the run's six tools scenarios) |
| 2026-11-20 14:09 | `shortcuts_hints`, `options_themes` | PASS (20.9, 34.3 s) |
| 2026-11-20 14:30 | `shortcuts_catalog` | FAIL: O-1 only; the other 197 keys and probes pass (336 s) |
| 2026-11-20 14:30 | `shortcuts_footer` | FAIL: O-2 only; every enabled cell does what its key does (89 s) |
| 2026-11-20 14:30 | `options_buttons` | FAIL: O-4 only; every other enabled button on every page has an effect (64 s) |
| 2026-11-20 14:41 | `files_filter_search`, `files_hot_paths` (cited above) | PASS (16.1, 5.5 s) |
| 2026-11-20 14:41 | `files_copy_options` (cited for O-G04) | FAIL on its own destination check (transfers record); its verification checks pass |
| 2026-11-20 14:58 | `options_buttons` (with the fix for O-4) | PASS (53.0 s) |
| 2026-11-20 15:21 | `shortcuts_catalog`, `shortcuts_footer` with the fixes for O-1 and O-2 | PASS (350.8, 81.6 s) |
| `20261121-144246` (Linux, commit 3f6fff8a, WSL 2 Ubuntu 24.04, every scenario of the suite; report in `_build/acceptance-linux/20261121-144246/`) | every scenario of this record: the ten `options_*` page scenarios, `options_buttons`, `options_themes`, `shortcuts_catalog`, `shortcuts_footer`, `shortcuts_hints` | all PASS (6.3–26.4 s for the pages; 45.6, 21.1, 224.8, 46.2, 11.2 s) |

Runs of 22 November with this record's checks: Windows 11 at scale 1 with the smoke
fixture, the debug acceptance binary through `scripts\acceptance.ps1 --test-binary`,
plugin packages of the release build of d7ff8860 through `--plugins`; Linux in WSL 2
(Ubuntu 24.04) through `scripts/acceptance.sh debug` with the shared debug plugin
packages, after the workspace clean step of the shared Cargo target. The base is the
main commit the record's files were applied to; 5ccd0eb7 has the corrected filename
colours (f153478c), and the earlier bases still had the interim all-cyan palette, which
`options_palette` checked then.

| Run | Base | Scenarios | Result |
| --- | --- | --- | --- |
| `20261122-050833` (Windows) | 05710878 | `options_general`, `options_settings_file`, `options_custom_commands`, `options_keyboard`, `options_colors`, `options_previews_plugins` (its staged part with the installed packages included), `options_visual`, `shortcuts_hints`, `options_buttons`, `options_themes` | all PASS (17.0, 20.0, 51.2, 30.3, 28.4, 35.4, 43.0, 21.1, 87.0, 37.5 s) |
| `20261122-050833` (Windows) | 05710878 | `options_palette` | FAIL in the migration part, a harness defect: the rules list makes only the rows near its viewport, and the scenario looked for rule 11 without scrolling; its colour checks passed (18.4 s). Fixed: the wheel brings the row in |
| `20261122-074313` (Windows) | 05710878 | `options_palette` with that fix | PASS (22.1 s) |
| `20261122-073609` (Linux) | 05710878 | the ten scenarios of the first row and `options_palette` | PASS except `options_custom_commands`, whose terminal-mode preview reported "No supported system terminal is installed" (WSL has none; the check now records that as SKIP of the dry run), and `options_palette`, which read the panel before its colours after a restart (the scenario now waits for them) |
| `20261122-092401` (Windows) | 26c18895 | the ten scenarios of the first row and `options_palette` | all PASS (18.0, 20.9, 55.8, 28.9, 25.3, 26.5, 36.7, 22.5, 96.8, 39.8, 49.6 s) |
| `20261122-123625` (Linux) | 5ccd0eb7 | the ten scenarios of the first row and `options_palette` | all PASS (18.8, 20.6, 57.7, 38.0, 47.4, 49.3, 77.5, 38.5, 140.3, 40.5, 31.8 s); `options_custom_commands` records SKIP of the terminal dry run (no supported terminal in WSL) |
| `20261122-124508` (Windows) | 5ccd0eb7 | the ten scenarios of the first row and `options_palette` | all PASS (19.1, 22.0, 52.6, 31.6, 27.5, 34.5, 43.8, 20.7, 90.3, 40.7, 39.5 s) |
| `20261122-174336` (Windows) | d96dec97 | `options_general` | PASS (21.2 s), with O-D17 |
| `20261122-192050` (Windows) | d96dec97 | the other nine scenarios of the first row and `options_palette` | PASS except `shortcuts_hints` (O-F02: the F10 menu lists every chord of an item since 22f46eab, which the check did not expect yet; updated, rerun due): `options_settings_file` 17.7, `options_custom_commands` 57.4, `options_keyboard` 28.1, `options_colors` 27.8, `options_previews_plugins` 28.8 (its staged part with the installed packages), `options_visual` 41.1, `options_buttons` 84.9, `options_themes` 29.5, `options_palette` 33.9 s |
| `20261122-200459` (Windows) | d96dec97 | `shortcuts_hints` with O-F02 updated | PASS (17.6 s) |
| `20261122-201240` (Linux) | d96dec97 | the ten scenarios of the first row, `options_palette`, `shortcuts_hints` | all PASS (`options_general` 11.1, `options_custom_commands` 34.2, `options_keyboard` 19.6, `options_colors` 15.9, `options_previews_plugins` 15.8, `options_visual` 27.7, `shortcuts_hints` 25.4, `options_buttons` 80.4, `options_themes` 38.6, `options_settings_file` 23.8, `options_palette` 39.7 s) |
Red runs: every check added or changed on 22 November (O-D04, O-D10, O-D13–O-D16, O-G07,
O-I01–O-I05, O-L03–O-L06, O-U02–O-U07, O-K07, O-K12–O-K14, O-P01, O-P02, O-F07) was
run once against a wrong expectation: a temporary copy of its scenario with one expected
value or condition inverted, built into the same test binary and not kept in the
repository. In Windows run `20261122-074313` all 57 copies failed, each at the check
it inverted, except `red_commands_settings_menu`, which failed earlier (a grouped
command's output file did not appear within 15 s); run again on its own in
`20261122-121540`, it failed at its inverted check (the context menu's entry). The palette checks rewritten for f153478c got five new copies (source code expected blue, the base colours expected without the readability policy, a list one step off each shipped palette expected to migrate, a customised list expected to keep its interim document rule cyan): all five failed at their inverted checks in Windows run `20261122-140946` (base 5ccd0eb7). O-D17 got one copy, which expects Keyboard (the page above the shown one) instead of Custom Commands: it failed at that check in Windows run `20261122-193317` (base d96dec97; "it shows Custom Commands with the focus on Custom Commands").
