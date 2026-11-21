# Acceptance on Windows

The acceptance suite on Windows 11 with `scripts/acceptance.ps1`: what ran, what it
found and what Windows cannot cover. How the runner and the harness differ on Windows
is described in [ACCEPTANCE.md](../ACCEPTANCE.md) ("Windows" under "Running it"); the
area records list the checks themselves.

Status: **PASS** (every check held in the run named), **FAIL** (a product bug, named by
its finding below or by the area record's open item, or a result this environment
cannot attribute, said so), **PENDING** (only checks of announced behavior failed, as
on Linux), **SKIP** (Windows or this environment cannot provide what the scenario
needs; the reason is given), **NOT RUN** (not run on Windows with the current harness).

## How it ran

Windows 11 Enterprise (10.0.26200), Czech regional settings (dates `21.11.2026`,
decimal comma, Czech collation, where "ch" sorts after "h"), the checkout on drive D:
and the private installation on C:, smoke scale, debug test build. The test executable
was built from the shared working tree on 21 November between about 03:00 and 06:00
local time (HEAD up to 3d42cb1f plus the roles' uncommitted changes of the moment), so
a run tests the product source of its build; the harness changed between the runs, and
each scenario is listed with the newest run that ran it. Two or three runs and other
roles' builds shared the machine (20 logical processors), which slows the debug
build's timed steps.

Plugins: the packages `scripts\validate.ps1` left in `_build\validate`
(`--validation-packages`): the Basic, Image, Markdown, PDF and Source viewers built on
20 November between 10:37 and 10:43, the native Windows filesystem built on 19 November
and the archive filesystem built on 20 November at 23:17. The viewers predate d9cafd45
(20 November 16:52), which changed them, so viewer checks of behavior that landed since
then cannot pass with them; those failures are marked "not attributable". There is no
HTML viewer and no thumbnailer package.

```text
pwsh -File scripts\acceptance.ps1 --test-binary <copy of acceptance-*.exe> --validation-packages
     --scale smoke --run-dir <checkout>\_build\acceptance-win --only <scenarios>
```

Run folders are below `_build\acceptance-win\` (named in UTC; a staged scenario's own
run is in its `work\<scenario>\staged\run\` folder, its screenshots are copied to the
outer run's `shots\`).

## Results

| Scenario | Area | Status | Run |
| --- | --- | --- | --- |
| `startup` | panels | PASS | `20261121-043913` |
| `startup_paths` | panels | PASS (a private profile: `USERPROFILE` and `LOCALAPPDATA`) | `20261121-043913` |
| `navigation` | panels | PASS | `20261121-043913` |
| `copy_move_delete` | transfers | PASS | `20261121-043913` |
| `create_rename` | operations | PASS | `20261121-043913` |
| `tree_expand_all` | tree | PASS | `20261121-043913` |
| `folder_sizes` | tree | PASS | `20261121-043913` |
| `find_by_mask` | find | PASS | `20261121-043913` |
| `checksums` | checksums | PASS | `20261121-043913` |
| `files_copy_kinds` | transfers | PASS | `20261121-043913` |
| `files_move_kinds` | transfers | PASS | `20261121-043913` |
| `files_conflicts` | transfers | PASS | `20261121-043913` |
| `files_links` | transfers | PASS (directory junctions: dangling, to a folder, to itself) | `20261121-043913` |
| `files_delete` | operations | PASS (F8 cancelled, the Recycle Bin not read; a read-only-attribute folder with contents deleted by Shift+F8) | `20261121-022248` |
| `files_rename` | operations | PASS | `20261121-043913` |
| `files_create` | operations | PASS | `20261121-043913` |
| `files_attributes` | operations | PASS (Hidden, Archive and Read-only set and cleared; times to 100 ns) | `20261121-024756` |
| `files_selection` | selection | PASS | `20261121-043913` |
| `files_sorting` | panels | PASS | `20261121-024756` |
| `files_filter_search` | panels | PASS (dot-names with the hidden attribute) | `20261121-022248` |
| `files_navigation` | panels | PASS | `20261121-043913` |
| `files_tabs` | panels | PASS | `20261121-022248` |
| `files_hot_paths` | panels | PASS | `20261121-043913` |
| `files_file_list` | panels | PASS | `20261121-043913` |
| `files_tree_ops` | tree | PASS | `20261121-043913` |
| `files_busy_folder` | panels | PASS | `20261121-043913` |
| `files_copy_options` | transfers | PASS | `20261121-043913` |
| `files_transfer_jobs` | jobs | FAIL: WIN-1 (Details shows no progress of the current file while paused) | `20261121-024756` |
| `files_transfer_errors` | transfers | PENDING: the two announced behaviors pending on Linux too (a Move out of a write-protected folder offers no Skip; Skip of an unreadable selected folder while the transfer is prepared); the cross-volume Copy and Move ran to the private installation's drive | `20261121-024756` |
| `files_clipboard_drag` | transfers | PASS | `20261121-022248` |
| `files_archive_transfers` | transfers | PASS (staged) | `20261121-022248` |
| `files_trash` | operations | SKIP: Windows recycles into the account's own Recycle Bin, one per volume, which no variable redirects to a private folder | `20261121-022248` |
| `tools_compare_dirs` | compare | PASS (its unreadable-metadata part noted as not run: no Windows equivalent) | `20261121-022908` |
| `tools_find_criteria` | find | PASS (`attributes has readonly` finds only the read-only file, `attributes has hidden` nothing) | `20261121-022908` |
| `tools_find_folders_help` | find | PASS | `20261121-022908` |
| `tools_find_results` | find | PASS | `20261121-022908` |
| `find_query_syntax` | find | PASS | `20261121-040741` |
| `find_roots_windows` | find | PASS | `20261121-022908` |
| `find_result_actions` | find | PASS (the harmless editor is the `cp.exe` stand-in) | `20261121-022908` |
| `find_feed_panel` | find | PASS (keypad * as Ctrl+I, and as Ctrl+Shift+I after Select by mask with Include folders) | `20261121-024756` |
| `find_budgets` | find | PASS | `20261121-022908` |
| `find_layout` | find | FAIL: WIN-7 (FIND-COMPACT-STATUS-HIDDEN, the find record's F-L04); every other check held | `20261121-042048` |
| `find_archives` | find | PASS (staged) | `20261121-024756` |
| `find_viewers` | find | PASS (staged) | `20261121-022908` |
| `tools_checksum_algorithms` | checksums | PASS | `20261121-024756` |
| `tools_checksum_manifests` | checksums | PASS (pages Calculate and Verify) | `20261121-024756` |
| `tools_checksum_large_stop` | checksums | PASS | `20261121-024756` |
| `tools_folder_sizes` | tree | PASS (directory junctions; Ctrl+G into one) | `20261121-043913` |
| `tools_jobs_copy` | jobs | PASS | `20261121-024756` |
| `tools_help_drive_info` | tools | PASS | `20261121-024756` |
| `tools_main_menu` | tools | PASS | `20261121-024756` |
| `ui_review` | ui | FAIL: 46 flags, as in run 20261121-031212. At 320×200 (Blue Desktop, Dark Touch) About does not close and the dialogs after it cannot open (14 steps), with the cut-off, clipped and zero-size texts of About, Hot Paths and a notice's Details: the open minimum-size finding U-D08 (ui-polish record); the command line's working folder (WIN-2, 15 flags) and the path bar's Edit folder path (WIN-3, 2 flags) | `20261121-045727` |
| `viewers_files` | viewers | FAIL, not attributable to the current source: the ten failed checks (CSS, Makefile, GNUmakefile, rules.mk and rules.mak not in Source mode; CSV and TSV not shown as tables; truncated and zero-byte PNG refused without the decoder's reason) are behaviour that landed in d9cafd45 (20 November 16:52), after the validation packages were built (10:43). 17 slow-open review flags of the debug build under load (staged) | `20261121-035050` |
| `viewers_controls` | viewers | PASS (staged) | `20261121-035050` |
| `viewers_compare` | viewers | PASS (staged) | `20261121-035050` |
| `viewers_thumbnails` | viewers | SKIP: the validation packages hold no thumbnailers | `20261121-035050` |
| `viewers_archives` | viewers | FAIL: WIN-8 (the copy-out of 100,001 files from flat.tar ran out of its 600 s budget); every other check held (staged) | `20261121-035050` |
| `viewers_scroll` | viewers | FAIL, not attributable (stale Source viewer package, debug build under load): Rust source only: five wheel notches and twenty Down presses in a row moved 256 px (300 and 560 expected) while one notch, one Down (28 px) and PageDown (712 px) moved as expected; 45 of 47 checks held (staged) | `20261121-035050` |
| `viewers_selection` | viewers | FAIL, not attributable (stale Source viewer package): selected Rust source text below 4.5:1 in GruvboxDark, CatppuccinLatte, RosePineDawn and GraphiteContrast; CatppuccinLatte's CSV selection shows no text; Blue, Dark and Light hold (staged) | `20261121-035050` |
| `viewers_presentation` | viewers | PASS (staged; the HTML zoom, policy and Find case skipped: no HTML viewer package) | `20261121-045833` |
| `viewers_combinations` | viewers | PASS (staged) | `20261121-035050` |
| `options_general` | options | PASS | `20261121-032203` |
| `options_appearance` | options | PASS | `20261121-032203` |
| `options_custom_commands` | options | PASS (the `sh.exe` stand-in; the preview shows arguments Debug-escaped) | `20261121-042048` |
| `options_keyboard` | options | PASS | `20261121-032203` |
| `options_colors` | options | PASS | `20261121-032203` |
| `options_archives` | options | PASS | `20261121-032203` |
| `options_editors` | options | PASS (the `cp.exe` stand-in) | `20261121-032203` |
| `options_comparators` | options | PASS | `20261121-032203` |
| `options_previews_plugins` | options | PASS | `20261121-032203` |
| `options_hot_paths` | options | PASS | `20261121-032203` |
| `options_visual` | options | PASS (with the UI role's swatch tooltip, uncommitted then and committed since in 93e7f43a: WIN-4 resolved) | `20261121-040741` |
| `ui_polish_checks` | ui | PASS (with the final gate: every red case flagged, its corrected twin clean) | `20261121-051501` |
| `ui_themes` | ui | PASS | `20261121-031212` |
| `ui_matrix_full` | ui | SKIP: runs with `--all-themes` only | `20261121-031212` |
| `ui_polish_dialogs` | ui | FAIL: WIN-2 only (1 flag: the command line's working folder overlapping its start overlay in the main window); every dialog's Tab, Escape, Enter, buttons, hover and tooltips held (staged) | `20261121-045655` |
| `ui_polish_jobs` | ui | PASS | `20261121-051501` |
| `ui_polish_viewers` | ui | FAIL: WIN-9 (Zoom image out and Zoom image in, − and +, without tooltip) | `20261121-035050` |
| `shortcuts_catalog` | keyboard | PASS (198 keys) | `20261121-040741` |
| `panels_dialogs` | panels | PASS (Ctrl+F2 sets Read-only where Linux types octal 600) | `20261121-043911` |
| `panels_notices` | panels | PASS (Copy details starts with the title, the reason and the folder) | `20261121-042110` |
| `panels_times` | panels | PASS | `20261121-042110` |
| `panels_view_sort` | panels | PASS | `20261121-042110` |
| `panels_paths` | panels | PASS (the drive menu; Attributes in the details strip) | `20261121-043911` |
| `panels_tabs` | panels | PENDING: the harness's own pending check of tabs restored after a restart (as on Linux) | `20261121-042110` |
| `panels_search_filter` | panels | PASS | `20261121-042110` |
| `panels_tree_keys` | tree | PASS | `20261121-042110` |
| `panels_selection_more` | selection | PASS | `20261121-042110` |
| `panels_hot_digits` | panels | PASS | `20261121-042110` |
| `panels_busy_ops` | panels | PASS (Ctrl+F2 sets Read-only on the two marked files) | `20261121-044947` |
| `panels_file_list` | panels | PASS | `20261121-042110` |
| `panels_root_keys` | panels | PASS | `20261121-042110` |
| `about_box` | tools | PASS | `20261121-035053` |
| `shortcuts_footer` | keyboard | PASS | `20261121-035053` |
| `tools_user_menu` | tools | PASS | `20261121-035053` |
| `tools_notices_jobs` | tools | PASS (Copy details starts with the title, the reason and the folder) | `20261121-040741` |
| `shortcuts_hints` | keyboard | PASS | `20261121-035053` |
| `options_buttons` | options | PASS | `20261121-032203` |
| `options_themes` | options | PASS | `20261121-032203` |
| `journeys_find_to_checksums` | journeys | PASS ("1 archive member not shown", finding J-1 fixed) | `20261121-043913` |
| `journeys_archive_roundtrip` | journeys | PASS | `20261121-032850` |
| `journeys_filtered_conflicts` | journeys | PASS (the recovery folder, without a hidden attribute, listed while hidden files are hidden) | `20261121-043911` |
| `journeys_concurrent_jobs` | journeys | PASS (the delete waits queued until the paused copy ends, as designed; WIN-5 withdrawn) | `20261121-042048` |
| `journeys_options_restart` | journeys | PASS (staged) | `20261121-032203` |
| `journeys_links_readonly` | journeys | SKIP: Linux link and mode semantics (see below) | `20261121-032850` |
| `journeys_keyboard_only` | journeys | PASS (Ctrl+U swaps the panels' places, each keeping its folder) | `20261121-042048` |
| `journeys_pointer_only` | journeys | FAIL: WIN-6 (the drag's Move "Completed with warnings"); the pending quick drop could not be tested: the drag took 683 ms (staged) | `20261121-043911` |

## Findings

Product findings seen on Windows (the area records hold the Linux ones).

| Finding | What happens | Scenario › check | Evidence |
| --- | --- | --- | --- |
| WIN-1 | A Copy paused in the middle of a 1 GiB file shows no progress of the current file under Details: no "Current file written: X / Y bytes" line, although the job window says "Paused · copying" with 257,687,552 bytes copied. On Linux the line is there (transfers record T-J11). After Resume, without Details, the file's own bar appears ("90%"), so the per-file meter works while the copy runs. The meter (`job_file_progress.rs`) needs `CopyProgress.current_file_size`, which the copy worker derives from its current file copy (`copy_worker.rs`, `store_progress`); the paused Windows job reports none. | `files_transfer_jobs` › Details shows the current file's progress | `20261121-024756\shots\files_transfer_jobs\progress-details.png` (also `20261121-022248`) |
| WIN-2 | The command line's prompt with a working folder longer than it can show (here about 120 characters) is flagged as overlapping text: the full path, scrolled to its end (`full-cwd` in `main.slint`), lies under the start-of-path overlay; at 320×200 Touch the full path is also flagged as elided to nothing. Seen at Dark Touch 960×600, HighContrastDark Touch 1340×820, both minimum sizes and the interactive gate's main window (Blue Desktop 1340×820). Either the overlay design needs a gate exemption with its reason, or the scrolled text must be clipped where the overlay starts. | `ui_review`, `ui_polish_dialogs` › overlapping text, label elided to nothing | `20261121-045727\shots\ui_review\HighContrastDark-Touch-Normal-x1-main.png` (also `20261121-031212`); `20261121-045655\shots\ui_polish_dialogs\dialogs-main.png` |
| WIN-3 | At Light Desktop 1600×1000 the path bar's "Edit folder path" button reaches 6 px outside the path bar's clipping container, which does not scroll (a long path, about 120 characters). | `ui_review` › content cut off without scrolling | `20261121-045727\shots\ui_review\Light-Desktop-Large-x1-main.png` (also `20261121-031212`) |
| WIN-4 | Resolved: Options › Colors' swatch buttons "Choose Foreground" and "Choose Background" showed no text and had no tooltip (run `20261121-032203`); the UI role's tooltip change (committed since in 93e7f43a) was in the build of run `20261121-040741`, where `options_visual` passed. | `options_visual` › icon-only button without tooltip | `20261121-032203\shots\options_visual\Light-Desktop-1340x820-colors.png` |
| WIN-5 | Withdrawn: not a product bug. A permanent delete of a folder beside a paused copy stays "Queued" until the copy ends, as designed: a Delete holds the whole namespace and every transfer keeps its namespace share until it ends, paused or not (`OPERATION_PROGRESS.md`, "Admission policy and waits"). The journey expected otherwise; it now checks the designed order and passed in run `20261121-042048`. | `journeys_concurrent_jobs` | `20261121-032850\shots\journeys_concurrent_jobs\jobs.png` |
| WIN-6 | Every same-volume Move on Windows ends "Completed with warnings" with an attention mark in Jobs, and its window stays open with "Source changes the system could not confirm as saved to disk: 1". The Windows native Move always reports `durability_confirmed: false` (`midnight-platform-windows/src/transfer_native_move.rs`), which counts `source_removal_durability_unconfirmed` (`move_native.rs`), gives the job a closure receipt (`copy_delivery.rs`, `closure`) and so the warning state (`job_presentation.rs`). Linux confirms it by syncing the parent folder. Since nothing on Windows can confirm it, every Move warns; whether Windows should report it at all is a design decision (Architect). | `journeys_pointer_only` › Jobs lists the move as completed | `20261121-043911\shots\journeys_pointer_only\move-outcome.png`, `jobs.png`; `20261121-042048\shots\ui_polish_dialogs\dialogs-jobs.png` |
| WIN-7 | FIND-COMPACT-STATUS-HIDDEN (the find record's F-L04, also on Linux): a Find window under 360 px high with text over 20 px puts the idle status into the query area, as FIND.md says, but at the end of the query form, scrolled out of the short query area's view: after the search the user sees no "Completed · 46 found · 46 examined". The status line (`find-window.slint`, `!root.inline-status`) is the last child of the scrolled `query-form`. | `find_layout` › Touch compact: a status line in view after the search | `20261121-042048\shots\find_layout\touch-700x340-idle.png` |
| WIN-8 | F5 of the folder of 100,001 files out of `flat.tar` copied about 59,000 files in its 600 s budget (about 100 files per second, each synced and read back for verification); the pending copy of a 100,001-member selection likewise ran out of time. Debug test build on a shared machine, archive package of 20 November 23:17: a performance result for the Perf role to measure in a release build, not yet a product bug. | `viewers_archives` › F5 of the folder of 100,001 files out of flat.tar | `20261121-035050`, the scenario's report (`work\viewers_archives\staged\run\20261121-035618\index.html`) |
| WIN-9 | The image viewer's zoom buttons "Zoom image out" (−) and "Zoom image in" (+) have no tooltip (Dark Desktop and Light Touch): `crates/midnight-viewer-ui/ui/viewer.slint`, the two `ActionButton`s without a hint. Fixed in source on 21 November: they, and the HTML viewer's same two buttons, show "Zoom image out (Ctrl+-)" and the like (`ui_polish::zoom_buttons_name_their_action_and_key_in_a_tooltip`); the gate has not been run again. | `ui_polish_viewers` › icon-only button without tooltip | `20261121-035050\shots\ui_polish_viewers\Dark-Desktop-viewer-image.png`, `Light-Touch-viewer-image.png` |

Also seen: a Jobs row counts in the plural whatever the number ("1 files · 0 skipped ·
0 errors", `job_presentation.rs`, the summary's format string; the row of a Copy of one
file in `20261121-042048\shots\ui_polish_dialogs\dialogs-jobs.png`), the way finding
J-1 was for archive members.

## Harness defects found on Windows

Fixed in the acceptance crate; those marked "all platforms" made the scenario fail on
Linux too.

- `journeys_concurrent_jobs` (all platforms): expected the delete to complete beside the
  paused copy; it now expects the designed wait (WIN-5).
- `journeys_keyboard_only` (all platforms): expected Ctrl+U to change the panels'
  folders; Ctrl+U swaps their places and each keeps its folder, as `files_navigation`
  checks. The journeys record's "not triaged" Ctrl+U failure was this.
- `journeys_filtered_conflicts` (all platforms): read the job window before sizing it, so
  only its title and Close were in view.
- `journeys_pointer_only` (all platforms): expected a copy with Replace · keep original to
  end without warnings and left that job's window open; a kept original is a warning by
  design.
- `panels_busy_ops` (all platforms): expected Ctrl+F2 to change the focused file while two
  files were marked (the panels record's W-03 test error).
- `find_layout` (all platforms): placed the compact window's status line by window
  coordinates, where a line scrolled out of the query area lies below the results; it now
  goes by the area that clips the line, which turned the finding into WIN-7.
- `viewers_presentation` (all platforms): measured Find matches with the tooltip of the
  control under the pointer ("Next match (F3)", "Find whole words") over the page's
  corner.
- The UI gate (all platforms): the button whose label repeats an active page tab (the
  Checksums footer's Verify) was clicked as the tab; Jobs' Clear all finished was clicked
  first and emptied the list for Open, Copy details and Clear finished without errors;
  copies to the clipboard were not observed; a surface still settling hid a click's
  effect; text scrolled to a sliver under a list's focus ring was measured for contrast;
  a dialog's only Tab stop (Help's Close) was compared with itself for a focus look.
- Windows only: an unreadable folder still accepted new entries (`fixtures::deny_read`
  now denies writing too, as mode 000 does); `shortcuts_catalog` hid its dot-file by
  name only; `options_custom_commands` compared the run's raw path with the preview's
  escaped one; `panels_dialogs`, `panels_busy_ops` and `panels_paths` expected octal
  permissions and `/` where Windows has attributes and `\`.

## Linux reruns needed

The harness changes of this Windows work that apply to every platform have not run on
Linux yet, and its Unix code was first compiled by CI (which lints the suite on both
platforms). These scenarios need a Linux run before their Linux records change:
`journeys_concurrent_jobs`, `journeys_keyboard_only`, `journeys_filtered_conflicts`,
`journeys_pointer_only`; `panels_busy_ops` (W-03), `panels_dialogs`, `panels_paths`;
`find_layout` (it should now report F-L04 on Linux too), `find_feed_panel`,
`find_query_syntax` and the other find scenarios; `viewers_presentation`,
`viewers_selection`, `viewers_combinations`; `options_custom_commands`,
`files_transfer_jobs`, `tools_notices_jobs`, `panels_notices`; and every UI gate
scenario (`ui_review`, `ui_themes`, `ui_polish_checks`, `ui_polish_dialogs`,
`ui_polish_jobs`, `ui_polish_viewers`, `options_visual`, `ui_matrix_full`).
`scripts/acceptance.sh` and its Linux runner tests are unchanged.

## Not covered on Windows

- The Recycle Bin: `files_trash` (`SKIP`); `files_delete` never confirms F8 and does not
  read the Recycle Bin; `ui_polish_dialogs` does not press Enter or Move to Trash in
  the Trash confirmation; `journeys_pointer_only` cancels it.
- Symbolic links: creating one needs administrator rights or Developer Mode, and Windows
  copies never recreate links, so the fixture's `links/` is empty and
  `journeys_links_readonly` reports `SKIP`. `files_links` and `tools_folder_sizes` use
  directory junctions instead; the link notes of `files_transfer_errors` and the "with
  notes" job window state of `ui_polish_jobs` are noted as not available.
- A listable folder whose entries cannot be reached (mode 0444 on a folder):
  `tools_compare_dirs` notes its unreadable-metadata part as not run.
- The HTML viewer and the thumbnailers (the validation packages hold neither): the HTML
  cases report themselves skipped, `viewers_thumbnails` reports `SKIP`.
- The viewer checks of behavior newer than the validation packages (see "How it ran");
  `viewers_files`, `viewers_scroll` and `viewers_selection` need a run with packages
  built from the current source.
- `ui_matrix_full` (`--all-themes`), and the full scale.
- A drop within 400 ms of the press (`journeys_pointer_only`, pending on Linux too): the
  synthetic drag took 683 ms on this loaded machine, so the check could not observe it.
