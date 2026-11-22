# Acceptance on Windows

The acceptance suite on Windows 11 with `scripts/acceptance.ps1`: what ran, what it
found and what Windows cannot cover. How the runner and the harness differ on Windows
is described in [ACCEPTANCE.md](../ACCEPTANCE.md) ("Windows" under "Running it"); the
area records list the checks themselves.

Status: **PASS** (every check held in the run named), **WARN** (every check held; the
UI review flagged something, a warning unless `--strict-review`), **FAIL** (a product
bug, named by its finding below), **PENDING** (only checks of announced behavior failed,
as on Linux), **SKIP** (Windows or this environment cannot provide what the scenario
needs; the reason is given), **NOT RUN** (not run on Windows with the current harness).

## How it ran

Windows 11 Enterprise (10.0.26200), Czech regional settings (dates `21.11.2026`,
decimal comma, Czech collation, where "ch" sorts after "h"), the checkout on drive D:
and the private installation on C:, debug test build, on 21 November between 15:18 and
20:30 local time.

- Source: commit 3f6fff8a (main when the run started; CI green on Windows and Linux).
  The whole-suite run's test executable was built at 15:43 from the clean worktree with
  one harness change (`viewers_stage::installed_packages`) and ran under the runner's
  thumbnailer staging fix (see "Harness defects").
  From 16:12 other roles edited product sources in the shared worktree and later
  commits landed (e9f51bff, 2b901122 and after; not tested here), so every rerun's
  executable was built from a `git archive` export of 3f6fff8a with only harness changes
  applied, in its own target directory: the fixes below, and for the last run
  (`20261121-192806`) also the Linux harness fixes of 21 November and the pending marks
  removed.
- Packages: one `build.bat debug` at 3f6fff8a (15:18–15:38, 1,178 s, offline, before any
  other role's edit): the Basic, Source, Markdown, Image, PDF and HTML viewers, the native
  Windows and archive filesystems, and the image and video thumbnailers (an FFmpeg 9.0.2
  MSVC prefix and its checksum-verified source archive prepared on this machine earlier,
  libclang 18.1.1, the verified CEF SDK under `_build\html-viewer`; nothing downloaded).
  Its deployment step's runtime validation (`package-release.py validate-runtime`) found
  6 viewers, 3 filesystems and 2 thumbnailers; the native filesystem package was current
  from 19 November (no source change since). The runner staged them from `_build\debug`
  (network providers left out).
- Commands: `pwsh -File scripts\acceptance.ps1 --scale smoke --offline --run-dir
  <checkout>\_build\acceptance-win2` (the whole suite, run `20261121-144422`, 6,966 s);
  reruns with `--test-binary <export build> --only <scenarios>` (`20261121-164408`,
  `20261121-170053`, `20261121-173345`, and `20261121-192806` with `--keep-fixture`);
  and `--scale full` for the transfer, archive and viewer scenarios (`20261121-175553`).
- Load: the Linux suite ran in WSL meanwhile (its VM held 10–16 GB), and other roles'
  Cargo builds (a release build, workspace test builds) ran for most of the afternoon:
  30–90 % CPU, which slows the debug build's timed steps (WIN-8's copy rate followed it).

Run folders are below `_build\acceptance-win2\` (named in UTC; a staged scenario's own
run is in its `work\<scenario>\staged\run\` folder, its screenshots are copied to the
outer run's `shots\`; fixtures and scratch folders of finished runs were deleted, their
reports and screenshots kept). The previous Windows runs (`_build\acceptance-win\`,
`20261121-015653` to `20261121-051501`) used the packages `scripts\validate.ps1` had
left, which predated d9cafd45 and held no HTML viewer and no thumbnailer; their results
are superseded.

## Results

| Scenario | Area | Status | Run |
| --- | --- | --- | --- |
| `startup` | panels | PASS | `20261121-144422` |
| `startup_paths` | panels | PASS (a private profile: `USERPROFILE` and `LOCALAPPDATA`) | `20261121-144422` |
| `navigation` | panels | PASS | `20261121-144422` |
| `copy_move_delete` | transfers | PASS; full scale PASS | `20261121-144422`; `20261121-175553` |
| `create_rename` | operations | PASS | `20261121-144422` |
| `tree_expand_all` | tree | PASS | `20261121-144422` |
| `folder_sizes` | tree | PASS | `20261121-144422` |
| `find_by_mask` | find | PASS | `20261121-144422` |
| `checksums` | checksums | PASS | `20261121-144422` |
| `files_copy_kinds` | transfers | PASS; full scale PASS | `20261121-144422`; `20261121-175553` |
| `files_move_kinds` | transfers | PASS; full scale PASS | `20261121-144422`; `20261121-175553` |
| `files_conflicts` | transfers | PASS; full scale PASS | `20261121-144422`; `20261121-175553` |
| `files_links` | transfers | PASS (directory junctions: dangling, to a folder, to itself); full scale PASS | `20261121-144422`; `20261121-175553` |
| `files_delete` | operations | PASS (F8 cancelled, the Recycle Bin not read; a read-only-attribute folder with contents deleted by Shift+F8) | `20261121-144422` |
| `files_rename` | operations | PASS | `20261121-144422` |
| `files_create` | operations | PASS | `20261121-144422` |
| `files_attributes` | operations | PASS (Hidden, Archive and Read-only set and cleared; times to 100 ns) | `20261121-144422` |
| `files_selection` | selection | PASS | `20261121-144422` |
| `files_sorting` | panels | PASS | `20261121-144422` |
| `files_filter_search` | panels | PASS (dot-names with the hidden attribute) | `20261121-144422` |
| `files_navigation` | panels | PASS | `20261121-144422` |
| `files_tabs` | panels | PASS | `20261121-144422` |
| `files_hot_paths` | panels | PASS | `20261121-144422` |
| `files_file_list` | panels | PASS | `20261121-144422` |
| `files_tree_ops` | tree | PASS | `20261121-144422` |
| `files_busy_folder` | panels | PASS | `20261121-144422` |
| `files_copy_options` | transfers | PASS; full scale PASS | `20261121-144422`; `20261121-175553` |
| `files_transfer_jobs` | jobs | PASS (WIN-1 resolved); full scale PASS; PASS with Wait off a strict check (T-J01) | `20261121-192806` (also `20261121-144422`); `20261121-175553` |
| `files_transfer_errors` | transfers | PENDING on one announced behavior, pending on Linux too: a Move out of a write-protected folder offers Retry, Cancel job and Decide later, no Skip (T-E06); T-J15, T-E05 and T-E10 held as strict checks; the cross-volume Copy and Move ran to the private installation's drive (C:), where the Move failed and kept its source ("Move needs a protected source parent"), an outcome the check accepts. With the harness of 3f6fff8a (`20261121-144422`, full scale `20261121-175553`) also T-E10 was pending: the harness read the question's buttons before the decision was up | `20261121-192806` |
| `files_clipboard_drag` | transfers | PASS; full scale PASS | `20261121-144422`; `20261121-175553` |
| `files_archive_transfers` | transfers | PASS (staged); full scale PASS | `20261121-144422`; `20261121-175553` |
| `files_trash` | operations | SKIP: Windows recycles into the account's own Recycle Bin, one per volume, which no variable redirects to a private folder | `20261121-144422` |
| `tools_compare_dirs` | compare | PASS (its unreadable-metadata part noted as not run: no Windows equivalent) | `20261121-144422` |
| `tools_find_criteria` | find | PASS | `20261121-144422` |
| `tools_find_folders_help` | find | PASS | `20261121-144422` |
| `tools_find_results` | find | PASS | `20261121-144422` |
| `find_query_syntax` | find | PASS | `20261121-144422` |
| `find_roots_windows` | find | PASS | `20261121-144422` |
| `find_result_actions` | find | PASS (the harmless editor is the `cp.exe` stand-in) | `20261121-144422` |
| `find_feed_panel` | find | PASS | `20261121-144422` |
| `find_budgets` | find | PASS | `20261121-144422` |
| `find_layout` | find | FAIL: WIN-10 (the compact Touch window's status line overlaps Fewer options and Feed to panel by 11 px); every other check held, WIN-7's status check among them | `20261121-170053` (also `20261121-144422`) |
| `find_archives` | find | PASS (staged); full scale PASS | `20261121-144422`; `20261121-175553` |
| `find_viewers` | find | PASS (staged) | `20261121-144422` |
| `tools_checksum_algorithms` | checksums | PASS | `20261121-144422` |
| `tools_checksum_manifests` | checksums | PASS (pages Calculate and Verify) | `20261121-144422` |
| `tools_checksum_large_stop` | checksums | PASS | `20261121-144422` |
| `tools_folder_sizes` | tree | PASS (directory junctions; Ctrl+G into one) | `20261121-144422` |
| `tools_jobs_copy` | jobs | PASS; full scale PASS | `20261121-144422`; `20261121-175553` |
| `tools_help_drive_info` | tools | PASS | `20261121-144422` |
| `tools_main_menu` | tools | PASS | `20261121-144422` |
| `ui_review` | ui | PASS (260 screenshots, no flag: WIN-2, WIN-3 and the earlier minimum-size flags are gone; the one flag of `20261121-144422`, Find's "Path" heading at 320×200, was a harness defect, fixed) | `20261121-173345` |
| `viewers_files` | viewers | WARN (staged): every check held, the HTML cases included; 6 slow-open flags of the debug build (`page.html` 3.8 s, `report.pdf` 3.6 s, `plain.htm` 905 ms, `plain.txt` 654 ms, `page.xhtml` 572 ms, `broken.html` 553 ms); full scale WARN (15 slow-open flags) | `20261121-144422`; `20261121-175553` |
| `viewers_controls` | viewers | PASS (staged); full scale PASS | `20261121-144422`; `20261121-175553` |
| `viewers_compare` | viewers | PASS (staged); full scale PASS | `20261121-144422`; `20261121-175553` |
| `viewers_thumbnails` | viewers | FAIL: WIN-11 (fixed in source on 22 November, rerun pending), in two of three runs (staged; the image formats, the PDF's first page, BMP, ICO and TIFF held; AVIF skipped: no encoder for a fixture); full scale PASS | `20261121-170053` (also `20261121-144422`); `20261121-175553` |
| `viewers_archives` | viewers | FAIL: WIN-8, every other check held (staged); 7-Zip, `.tar.zst`/`.tzst`/`.zst` and the deflated ZIP skipped: no `7z`, `zstd` or `zip` program on this machine; full scale FAIL: WIN-8 only | `20261121-170053` (also `20261121-164408`, `20261121-144422`); `20261121-175553` |
| `viewers_scroll` | viewers | PASS (staged; HTML wheel travel and kinetic coasting skipped: the harness delivers no native wheel events or touchpad phases); full scale PASS | `20261121-144422`; `20261121-175553` |
| `viewers_selection` | viewers | WARN (staged): selected text reaches 4.5:1 in every theme once the probe's defect was fixed; one flag: PaperContrast's unselected Rust source shows a colour #b3bdcf at 1.87:1 against #fdfefe (below 3:1, a warning); full scale WARN (the same flag) | `20261121-170053`; `20261121-175553` |
| `viewers_presentation` | viewers | PASS (staged; the HTML zoom, policy and Find cases ran); full scale PASS | `20261121-144422`; `20261121-175553` |
| `viewers_combinations` | viewers | PASS (staged); full scale PASS | `20261121-144422`; `20261121-175553` |
| `options_general` | options | PASS | `20261121-144422` |
| `options_appearance` | options | PASS | `20261121-144422` |
| `options_custom_commands` | options | PASS (the `sh.exe` stand-in; the preview shows arguments Debug-escaped) | `20261121-144422` |
| `options_keyboard` | options | PASS | `20261121-144422` |
| `options_colors` | options | PASS | `20261121-144422` |
| `options_archives` | options | PASS | `20261121-144422` |
| `options_editors` | options | PASS (the `cp.exe` stand-in) | `20261121-144422` |
| `options_comparators` | options | PASS | `20261121-144422` |
| `options_previews_plugins` | options | PASS | `20261121-144422` |
| `options_hot_paths` | options | PASS | `20261121-144422` |
| `options_visual` | options | PASS (WIN-4 resolved) | `20261121-144422` |
| `ui_polish_checks` | ui | PASS (every red case flagged, its corrected twin clean) | `20261121-144422` |
| `ui_themes` | ui | PASS | `20261121-144422` |
| `ui_matrix_full` | ui | SKIP: runs with `--all-themes` only | `20261121-144422` |
| `ui_polish_dialogs` | ui | PASS (staged; WIN-2 resolved; the one flag of `20261121-144422`, Jobs' Clear all finished with nothing left to clear, was a harness defect, fixed) | `20261121-170053` |
| `ui_polish_jobs` | ui | PASS | `20261121-144422` |
| `ui_polish_viewers` | ui | PASS (WIN-9 resolved) | `20261121-144422` |
| `shortcuts_catalog` | keyboard | PASS | `20261121-144422` |
| `panels_dialogs` | panels | PASS (Ctrl+F2 sets Read-only where Linux types octal 600) | `20261121-144422` |
| `panels_notices` | panels | PASS (Copy details starts with the title, the reason and the folder) | `20261121-144422` |
| `panels_times` | panels | PASS | `20261121-144422` |
| `panels_view_sort` | panels | PASS | `20261121-144422` |
| `panels_paths` | panels | PASS (the drive menu; Attributes in the details strip) | `20261121-144422` |
| `panels_tabs` | panels | PENDING: the harness's own pending check of tabs restored after a restart (as on Linux) | `20261121-144422` |
| `panels_search_filter` | panels | PASS | `20261121-144422` |
| `panels_tree_keys` | tree | PASS | `20261121-144422` |
| `panels_selection_more` | selection | PASS | `20261121-144422` |
| `panels_hot_digits` | panels | PASS | `20261121-144422` |
| `panels_busy_ops` | panels | PASS (Ctrl+F2 sets Read-only on the two marked files) | `20261121-144422` |
| `panels_file_list` | panels | PASS | `20261121-144422` |
| `panels_root_keys` | panels | PASS (staged) | `20261121-144422` |
| `about_box` | tools | PASS | `20261121-144422` |
| `shortcuts_footer` | keyboard | PASS | `20261121-144422` |
| `tools_user_menu` | tools | PASS | `20261121-144422` |
| `tools_notices_jobs` | tools | PASS (Copy details starts with the title, the reason and the folder) | `20261121-144422` |
| `shortcuts_hints` | keyboard | PASS | `20261121-144422` |
| `options_buttons` | options | PASS | `20261121-144422` |
| `options_themes` | options | PASS | `20261121-144422` |
| `journeys_find_to_checksums` | journeys | PASS | `20261121-144422` |
| `journeys_archive_roundtrip` | journeys | PASS; full scale PASS | `20261121-144422`; `20261121-175553` |
| `journeys_filtered_conflicts` | journeys | PASS (the recovery folder, without a hidden attribute, listed while hidden files are hidden) | `20261121-144422` |
| `journeys_concurrent_jobs` | journeys | PASS (the delete waits queued until the paused copy ends, as designed; the copy that does not wait completes beside the paused one, a strict check) | `20261121-192806` (also `20261121-144422`) |
| `journeys_options_restart` | journeys | PASS (staged) | `20261121-144422` |
| `journeys_links_readonly` | journeys | SKIP: Linux link and mode semantics (see below) | `20261121-144422` |
| `journeys_keyboard_only` | journeys | PASS (Ctrl+U swaps the panels' places, each keeping its folder) | `20261121-144422` |
| `journeys_pointer_only` | journeys | PENDING: only the quick drop could not be tested (the synthetic drag was released after 612 ms; the pending check needs a drop within 400 ms); WIN-6 resolved: the drag's Move ends Completed | `20261121-144422` |

Totals of the whole-suite run `20261121-144422`: 94 passed, 1 warned, 3 pending, 3 skipped,
6 failed of 107 in 6,966 s. With the reruns: 96 PASS, 2 WARN, 3 FAIL (WIN-8, WIN-10,
WIN-11), 3 PENDING, 3 SKIP. Durations of the reruns: `20261121-164408` 733 s (one
scenario), `20261121-170053` 3,154 s (six), `20261121-173345` 352 s (one),
`20261121-192806` 102 s (three: 2 PASS, 1 PENDING); `20261121-175553` 4,713 s (22
scenarios at full scale: 18 PASS, 2 WARN, 1 PENDING, 1 FAIL).

## Findings

Product findings seen on Windows (the area records hold the Linux ones). The verdicts
of 21 November afternoon come first in each row; the rest is the finding's history.

| Finding | What happens | Scenario › check | Evidence |
| --- | --- | --- | --- |
| WIN-1 | **Resolved** (`20261121-144422`, PASS; full scale PASS): with Details open at 760×560 the paused copy of the 1 GiB file shows "Current file written: 245 366 784 / 1 073 741 824 bytes" and its bar, in view. History: not an engine defect: a Copy paused in the middle of a 1 GiB file showed no progress of the current file under Details, although the job window said "Paused · copying" with 257,687,552 bytes copied. The paused job did report it (`CopyProgress.current_file_copied`, `copy_worker.rs` `store_progress`; engine test `a_copy_paused_in_the_middle_of_a_file_reports_its_size_and_progress`). The line was out of view: with Details open the details box takes 38–45% of the 760×560 window, and with Windows' fonts the line, then the next item after the file's name in `job-progress-window.slint`, lay below the scrolled progress area's visible part. Fixed in source on 21 November: under Details a Copy or Move shows the line and its bar above the speed chart, after the time remaining (`native_job_windows::details_file_progress`). | `files_transfer_jobs` › Details shows the current file's progress | `20261121-144422\shots\files_transfer_jobs\progress-details.png`; before: `_build\acceptance-win\20261121-024756\shots\files_transfer_jobs\progress-details.png` |
| WIN-2 | **Resolved** (`20261121-144422`, `20261121-173345`): no overlapping-text or elided flag of the command line's working folder in `ui_review`'s 260 screenshots or in `ui_polish_dialogs`' main window. History: the prompt's working folder, longer than it can show (about 120 characters), lay under the start-of-path overlay (`full-cwd` in `main.slint`), flagged as overlapping text, and at 320×200 Touch as elided to nothing. Fixed in source on 21 November: the strip is clipped where the start overlay ends (`command_path_overlay`). | `ui_review`, `ui_polish_dialogs` › overlapping text, label elided to nothing | `20261121-173345\shots\ui_review\HighContrastDark-Touch-Normal-x1-main.png`; before: `_build\acceptance-win\20261121-045727\shots\ui_review\HighContrastDark-Touch-Normal-x1-main.png` |
| WIN-3 | **Resolved** (`20261121-144422`, `20261121-173345`): no cut-off flag at Light Desktop 1600×1000. History: the path bar's "Edit folder path" button reached 6 px outside the path bar's clipping container with a path of about 120 characters. Fixed in source on 21 November: the path bar keeps a pixel per drawn component in reserve (`path_bar_fit`). | `ui_review` › content cut off without scrolling | `20261121-173345\shots\ui_review\Light-Desktop-Large-x1-main.png` |
| WIN-4 | Resolved (unchanged; `options_visual` PASS again in `20261121-144422`): Options › Colors' swatch buttons "Choose Foreground" and "Choose Background" had no tooltip (93e7f43a). | `options_visual` › icon-only button without tooltip | `20261121-144422\shots\options_visual\` |
| WIN-5 | Withdrawn (unchanged; `journeys_concurrent_jobs` PASS in `20261121-144422`): a permanent delete beside a paused copy stays "Queued" until the copy ends, as designed (`OPERATION_PROGRESS.md`, "Admission policy and waits"). | `journeys_concurrent_jobs` | `20261121-144422\shots\journeys_concurrent_jobs\` |
| WIN-6 | **Resolved** (`20261121-144422`): the drag's same-volume Move ends "Completed" (1 renamed) in Jobs; the Copy with Replace · keep original stays "Completed with warnings", as designed. History: every same-volume Move ended "Completed with warnings" with "Source changes the system could not confirm as saved to disk: 1", because the Windows native Move reported `durability_confirmed: false` (`transfer_native_move.rs`) and source removals by delete disposition were reported before the handle that set the disposition closed. Fixed in source on 21 November: native renames flush their folders through new handles (`transfer_folder_flush.rs`), removals are confirmed once the handles are dropped by finding the name gone and flushing its folder (`TransferDurabilityConfirmation`), and a Move flushes the folders of its renames together ([Move durability on Windows](../TRANSFERS.md#move-durability-on-windows)). | `journeys_pointer_only` › Jobs lists the move as completed | `20261121-144422\shots\journeys_pointer_only\jobs.png`; before: `_build\acceptance-win\20261121-043911\shots\journeys_pointer_only\move-outcome.png` |
| WIN-7 | **Resolved as reported** (`20261121-170053`): after the search the compact Touch window (700×340, 24 px text) shows "Completed · 46 found · 46 examined" on the query area's last line, in view. The fix overlaps the form: WIN-10. History: FIND-COMPACT-STATUS-HIDDEN (the find record's F-L04): the idle status in the query area was the last child of the scrolled `query-form` (`find-window.slint`) and scrolled out of view. Fixed in source on 21 November: the status is pinned to the query area's last line (`find_compact_status`). | `find_layout` › Touch compact: a status line in view after the search | `20261121-170053\shots\find_layout\touch-700x340-idle.png` |
| WIN-8 | **Still failing**: F5 of the folder of 100,001 files out of `flat.tar` copied 69,896 files in its 600 s budget (116 files/s; `20261121-164408`, 30–50 % CPU from other builds) and 52,343 (87 files/s; `20261121-170053`, about 55 % CPU), where about 59,000 (about 100 files/s) arrived before the fix; cancelled at the budget, the job ended Cancelled within 1.3 s (`20261121-170053`). The pending copy of a 100,001-member selection: 37,905 files in 609 s (62 files/s), and its Cancel took 117 s to end it. At full scale 41,986 files (70 files/s; `20261121-175553`, other builds running) and the selection 33,482; the selection's Cancel again took 114 s, while the folder copy's ended within about a second each time. Debug test build. History: measured on 21 November without windows (the `EnqueueArchiveCopy` command, D:): release 97–132 files/s, debug 64–105; 82–85 % of the CPU was kernel time in the Windows namespace pins, which checked the destination folder's whole ancestry 11 times per copied file. Fixed in source on 21 November (cd2f821a: `transfer_namespace_parent.rs`, `transfer_namespace.rs`): a refresh batch checks each ancestor once, so a file takes 4 checks; measured then: release 133–183 files/s (median 562 s for 100,001 files), debug 142–186 on 20,001 files. The remaining per-file cost is mostly Defender and Defender for Endpoint (a create 0.33 ms, a rename 0.5 ms). In the suite the debug build stays far from 167 files/s on this machine: the budget, or the per-file cost under endpoint protection, needs a decision. | `viewers_archives` › F5 of the folder of 100,001 files out of flat.tar | the staged reports `20261121-164408\work\viewers_archives\staged\run\20261121-164409\index.html` and `20261121-170053\work\viewers_archives\staged\run\*\index.html` |
| WIN-9 | **Resolved** (`ui_polish_viewers` PASS, `20261121-144422`): the image viewer's zoom buttons have their tooltips. History: "Zoom image out" (−) and "Zoom image in" (+) had no tooltip (`midnight-viewer-ui/ui/viewer.slint`); fixed on 21 November (`ui_polish::zoom_buttons_name_their_action_and_key_in_a_tooltip`). | `ui_polish_viewers` › icon-only button without tooltip | `20261121-144422\shots\ui_polish_viewers\` |
| WIN-10 | New, from the WIN-7 fix: a Find window under 360 px high with text over 20 px (Touch with 24 px text at 700×340) keeps the idle status on the query area's last line, but the query form's last row overlaps it by 11 px: "Fewer options" and "Feed to panel" overlap the status line "Ready · 0 found · 0 examined" (198×11 and 193×11 px), the status text is drawn over the buttons' lower edge, and the form begins about 14 px below the area's top. Cause, in the source: `find-window.slint` gives the query area's `query-scroll` ScrollView `height: parent.height - root.query-status-height` but no `y`, and the Slint compiler centres an element with a fixed height and no `y` in a parent that is not a layout (`y: (parent.height - self.height) / 2`): the scroll view sits half the status band lower and its bottom reaches half the band less 3 px (11 px at the 28 px band of 21 px small text) into the status line at `y: parent.height - self.height`. `y: 0px` on `query-scroll` would end the form above the line. The WIN-7 test (`find_compact_status`) checks that the status line and the mask field are in view, not that they are clear of other controls. Pixel rows of the screenshot: the area's first text at y 19 (the area starts at y 4), Fewer options y 85–126, the status text y 121–141. | `find_layout` › Touch compact (the review's overlapping controls) | `20261121-144422\shots\find_layout\touch-700x340.png`; after the search `20261121-170053\shots\find_layout\touch-700x340-idle.png` |
| WIN-11 | **Fixed in source, rerun pending** (22 November; stays FAIL until an acceptance run passes it): (a) thumbnails capture under `RevisionPolicy::AcceptChangeTimeOnly` like viewers (the thumbnail host's `service::NATIVE_REVISIONS`, which `native_content::thumbnail_factory` and the harness's pulse in `viewers_support.rs` both use): a moved change time alone keeps the capture and its cached frame, while a new write time, length or identity still invalidates both; a source that fails its check in the cache is a `SourceFailure`, not "Could not store or read preview cache"; (b) the preview service's notices are shown in the status row of Thumbnails-view panes, never as a notice that takes the keyboard (as are the other background notices found: the regional-format and options requests at startup on the status line, a failed filename-color match in its pane's status row); (c) a tile whose work failed for a passing reason is retried after 2 s and 8 s, a changed file with a new capture, and one that cannot pass is not retried while in view and keeps its type icon. Tests: `cache_revision_tests` (a moved change time keeps the capture and its frame; an exact capture fails as its source; a content change invalidates), `a_source_failing_its_check_in_the_cache_is_not_a_cache_failure`, `a_file_changed_after_its_capture_is_captured_again_and_its_tile_retried`, `a_transient_failure_is_retried_after_a_short_delay`, `a_failure_that_cannot_pass_is_not_retried_while_its_tile_is_shown`, the scheduler's retry tests, the presenter's `a_preview_service_failure_is_a_status_row_never_a_notice` and the window test `storage_reasons_and_preview_failures_are_status_rows_that_leave_the_keyboard_alone`. Before: thumbnails of files that Microsoft Defender for Endpoint's data loss prevention has just tagged. In `viewers_thumbnails` a modal "Previews" notice, "Could not store or read preview cache: Captured native content changed.", came up while `thumbs/` was shown as thumbnails and took the keyboard, so Backspace did nothing for 60 s (run `20261121-144422`); in the rerun no notice came, but three of the nine visible tiles (`tile-03` to `tile-05`) had no thumbnail after 45 s, and after End and Home six of the nine were shown at once, all nine after 1.5 s (`20261121-170053`). The full-scale run passed every check (`20261121-175553`): it depends on when the tagging lands. Every fixture file of both runs carries a 302-byte alternate data stream `sec.endpointdlp`, which the DLP writes after a file is created while setting its write time back, so only the file's ChangeTime moves (as 1f8dcac5 found for archives). The application captures a thumbnail's source with `InstalledProvider::capture`, that is `RevisionPolicy::Exact` (`midnight-explorer/src/native_content.rs`, `factory`, used by `thumbnail_content.rs`), while viewers, Copy and Move sources and archives accept a ChangeTime-only move (VIEW-05: `factory_with_resources`, `ARCHIVE_REVISIONS`). The thumbnail cache revalidates the capture when it stores a frame and on every hit (`midnight-thumbnail-host/src/cache.rs`, `Key::validate`); the native plugin's `validate` (`midnight-filesystem-native-windows/src/content.rs`) then fails with "Captured native content changed.", the service reports the first such cache failure as that notice (`service.rs`), and a tile whose work failed stays empty until the view changes. The harness's thumbnail pulse mirrors the application's capture (`viewers_support.rs`) and must follow a change there. | `viewers_thumbnails` › every visible tile, cached thumbnails after Home (the notice: the step after it timed out) | the run summary of `20261121-144422`; `20261121-170053\work\viewers_thumbnails\staged\run\20261121-170531\index.html`; `Get-Item -Stream *` on the fixtures' `thumbs\*` |

Also seen before and resolved: a Jobs row counted in the plural whatever the number
("1 files · 0 skipped · 0 errors"). Fixed in source on 21 November; in `20261121-144422`
the rows read "1 file · 0 skipped · 0 errors"
(`20261121-144422\shots\ui_polish_dialogs\dialogs-jobs.png`).

## Harness defects found on Windows

Fixed in the acceptance crate or the Windows runner; those marked "all platforms" made
the scenario fail, or could have, on Linux too.

Found on 21 November afternoon (in the worktree, not yet committed):

- `scripts/acceptance.ps1` (Windows only): staged only the folders of each package
  source, but thumbnail providers are installed flat, as the thumbnail host discovers
  them (each provider's `<name>.json` and executable, the FFmpeg libraries they share and
  `notices\`), so `packages\thumbnailers` held only `notices\` and no provider. The
  thumbnailers folder is now copied whole and its providers are named by their manifests
  ("Plugin packages (thumbnailers): midnight-thumbnail-image, midnight-thumbnail-video").
  The copy also loses the explicit read-and-execute grant that `deploy-private.ps1` gives
  the restricted workers' AppContainer on the FFmpeg libraries and the playback helper
  (`avcodec-63.dll`, `avformat-63.dll`, `avutil-61.dll`, `swscale-10.dll`,
  `swresample-7.dll`, `midnight-video-playback.exe`), without which the video worker
  cannot load FFmpeg (found by the video role; the scenarios of 3f6fff8a open no video):
  the runner grants it again when the video thumbnailer is staged, with the SID computed
  as `deploy-private.ps1` computes it (never `DeriveAppContainerSidFromAppContainerName`,
  which a packaged PowerShell answers with a child AppContainer), and stages hard-link
  the granted files. `scripts/tests/test_acceptance_windows.py` models the flat layout
  and checks the grant on exactly those files (its 6 tests pass).
- `viewers_stage::installed_packages` (all platforms): counted folders as thumbnailer
  packages, so `notices/` alone made a stage without a provider count one; it counts
  the providers' manifests now.
- `find_layout` (all platforms): a layout finding in the compact window returned before
  the compact checks, so WIN-7's status check never ran; the finding is now reported
  with them.
- `viewers_archives` (all platforms): a copy over its budget was left running beside the
  next check, still writing into its folder; it is now cancelled from its progress window
  (laid out first, Cancel operation and the confirmation) and the failure says how many
  files had arrived and at what rate. F5 on a 100,001-member selection that is still
  resolving ("Selection still loading") is pressed again, as a user would.
- `viewers_thumbnails` (all platforms): a notice while thumbnails load took the keyboard,
  and the scenario waited 60 s for a panel that could not move; the notice now fails a
  check of its own and is closed, so the remaining checks run.
- `viewers_selection` (all platforms): the selected-text contrast probe took the
  gutter's line numbers beside the selection (their anti-aliased edges come near the
  selection's colour) and edge colours that the 1.5° direction rule split from their
  glyph colour for selected text: GruvboxDark #867f6b 2.21:1, CatppuccinLatte #a979f0
  1.72:1, RosePineDawn #bd9ead 2.35:1 and GraphiteContrast #cb834e 2.35:1, where the
  screenshots' cores reach #e7ebf0 4.52:1, #e1e5ec 4.51:1 and #a94700 4.51:1 (the
  viewers record's S4, S5 and X8, refuted in source). `viewers_probe::selected_glyph_cores`
  counts only runs between pixels the selection painted and lets an edge colour join any
  stronger colour of a group it blends towards.
- The UI gate (all platforms): Jobs' Clear all finished, clicked after Clear finished
  without errors, had nothing left to clear once every earlier job had ended without
  errors (WIN-6's fix removed the Move warning that used to stay); the Jobs surface now
  has a job that finished just before each opening. And the button-text rule (the gate's
  and the review's) flagged a caption whose own box holds it but which is cut at the edge
  of a table that scrolls sideways, or of a header kept in step with one (Find's "Path"
  heading at 320×200: 20 of 23 px); the elision rule's exemption for such text applies to
  it now.
- `fixtures::allow_read` (Windows only): a write-protected folder that the application
  moved (`files_transfer_errors`: a Move keeps its mode) kept entries that deny their own
  deletion under paths the process never recorded, so the kept run folder could not be
  deleted; the folder's entries on disk are cleared too now. Seen in both kept runs that
  ran the scenario (`20261121-144422`, `20261121-175553`: `move-dest\locked\inside.txt`
  kept a `(DENY)(DE)` entry of this account; cleared by hand); with the fix, the work
  folder of `20261121-192806` held no deny entry and was deleted whole.
- Pending marks whose behavior the Linux run of 21 November confirmed are strict checks
  now (all platforms): Wait off runs at once beside a paused job (T-J01,
  `files_transfer_jobs`, and the same step of `journeys_concurrent_jobs`), the framed
  decision window without a × of its own (T-J15), the rest of a tree copied after Skip
  of an unreadable folder (T-E05) and Skip of an unreadable selected folder while the
  transfer is prepared (T-E10) in `files_transfer_errors`, and T-R10 to T-R13 in
  `files_trash` (Linux only: Windows skips the scenario). A Move out of a
  write-protected folder (T-E06) stays pending. On Windows, with the Linux harness fixes
  of 21 November (`files_errors.rs` waits for the decision before reading its buttons),
  `files_transfer_jobs` and `journeys_concurrent_jobs` passed and `files_transfer_errors`
  was pending on T-E06 alone (`20261121-192806`).

Found in the earlier Windows runs and fixed since:

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

Linux ran the whole suite with the harness of 3f6fff8a on 21 November (WSL 2, Ubuntu
24.04 on ext4, smoke scale, debug test build, every plugin package built from that
commit: run `20261121-144246`, 107 scenarios in 6,778 s; reruns `20261121-164146` and
`20261121-170455` with the Linux harness fixes, now committed; reports in
`_build/acceptance-linux/<run>/`; results in the area records). Of the scenarios this
section listed:

- Passed on Linux: `journeys_concurrent_jobs`, `journeys_keyboard_only`,
  `journeys_filtered_conflicts`; `panels_busy_ops` (W-03), `panels_dialogs`,
  `panels_paths`; `find_feed_panel`, `find_query_syntax` and every other Find scenario
  but `find_layout`; `viewers_combinations`; `options_custom_commands`,
  `files_transfer_jobs`, `tools_notices_jobs`, `panels_notices`; `ui_themes`,
  `ui_polish_checks`, `ui_polish_viewers`, `options_visual`; and, with the Linux harness
  fixes, `ui_polish_dialogs`.
- `journeys_pointer_only`: PENDING with the fixes (every check held; the quick drop
  could not be observed, the synthetic drag took 482 and 603 ms). Without them it
  failed on its own Trash check, a harness defect.
- `find_layout`: FAIL, the product finding in the fix of WIN-7 that Windows reports as
  WIN-10 (FIND-COMPACT-STATUS-OVERLAP in the find record): at Touch 700×340 the pinned
  status line overlaps the scrolled query form's last row by 11 px; the scenario stopped
  there (the Windows harness now reports the finding with the compact checks).
- `ui_review`: FAIL, 3 flags with the fixes, all at 320×200: U-D22 in the UI polish
  record (Checksums' "Save…" and Find's View half outside the footer's scroller) and
  the Find results' "Path" heading cut where the table scrolls sideways (the gate defect
  that the Windows harness fix of 21 November exempts, see "Harness defects"). None of
  U-D01–U-D21, WIN-2, WIN-3 or WIN-9 recurred.
- `viewers_presentation`: FAIL on its HTML case only (the WSL distribution lacks the
  HTML viewer's NSS/NSPR and ALSA libraries); `viewers_selection`: FAIL on the probe's
  reading of antialiased pixels (RosePineDawn and GraphiteContrast, 2.35:1), as on
  Windows before the probe's fix.
- `ui_polish_jobs`: FAIL in the first run on a harness race (Slint panicked when the
  harness used the long copy's window after the short smoke-scale copy had ended and
  closed it); PASS on the rerun, which reached the cancel question in neither look.
- `ui_matrix_full`: SKIP (`--all-themes` only); not run on either platform.

Still needed on Linux: `ui_matrix_full`; the HTML cases on a machine with the HTML
viewer's libraries; and the scenarios whose harness the Windows run of 21 November
changed since 3f6fff8a, once those changes are committed: `find_layout`,
`viewers_archives`, `viewers_selection` (the probe in `viewers_probe.rs`),
`viewers_thumbnails` (and its SKIP decision in `viewers_stage.rs`), `ui_polish_dialogs`
(`ui_dialogs.rs`), and the button-text exemption of `ui_gate.rs` and `inspect.rs`, which
every UI gate and layout review applies (`ui_review` first); and the strict checks that
replaced pending marks Linux confirmed (`files_transfer_jobs`, `files_transfer_errors`,
`files_trash`, `journeys_concurrent_jobs`), whose `files_trash` part Windows cannot run.
Their Unix code was compiled only on Windows (CI lints the suite on both platforms); the
`fixtures.rs` change is Windows only. `scripts/acceptance.sh` and its Linux runner tests
are unchanged.

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
- Archive formats whose tools this machine lacks: 7-Zip (`7z`, `7za`, `7zr`), zstd and
  `zip`: `viewers_archives` skips `bundle.7z`, `outer.7z` (and the 7-Zip ⊃ tar.gz ⊃ ZIP
  nesting), `.tar.zst`, `.tzst`, `note.txt.zst` and the deflated ZIP.
- HTML wheel travel and kinetic (touchpad) coasting in `viewers_scroll`: the harness
  delivers no native window events.
- AVIF thumbnails (no encoder to make a fixture).
- `ui_matrix_full` (`--all-themes`); the full scale for every scenario but the transfer,
  archive and viewer ones listed above.
- A drop within 400 ms of the press (`journeys_pointer_only`, pending on Linux too): the
  synthetic drag took 612 ms on this loaded machine, so the check could not observe it.
