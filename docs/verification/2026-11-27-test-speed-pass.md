# Test speed pass — 2026-11-27

## Scope

Every Windows test of the root workspace's library, binary and integration test
executables was timed at `dbec70aa` (70 test executables); the slowest tests and every
executable's tail were looked at, and three tests were changed where that loses nothing
they prove. The plugins under `plugins/` are separate workspaces and were not timed. 152
commits lie between the [25 November pass](2026-11-25-test-speed-pass.md)'s final build
`580ca5ab` and `dbec70aa`; 73 of them touch test files.

## Method

As on 25 November ([Method](2026-11-25-test-speed-pass.md#method)), with these differences:

1. **Builds.** `cargo test --workspace --no-run --message-format=json-render-diagnostics
   --locked --offline -j 6`, debug test profile, debug information off, `CARGO_INCREMENTAL=0`,
   in a worktree of its own whose target directory was warm from an earlier commit: the
   baseline at `dbec70aa` (6 min 8 s, 70 test executables, copied aside before any change),
   and the changed builds by `cargo test --workspace --no-run` in the same target
   directory, so that each test executable keeps its file name.
2. **Runs.** Each executable alone, one after another, from its package directory, with
   libtest's default thread count (20 here), JSON output, `-Z unstable-options
   --report-time` (`RUSTC_BOOTSTRAP=1` for the test process only), a private temporary
   directory below the worktree's `_build`, `MIDNIGHT_UI_RENDER_DIR` unset, a private
   `HOST_TEST_INSTALL_ROOT`, and the build's `deps` folder and the toolchain's runtime
   folder on `PATH`. Before each timed run the executable listed its tests twice and its
   ignored tests once. Whole-set rounds held the desktop lease. A wall time ends when the
   executable's output closes.
3. **Rounds.** *Survey*: three rounds of all 70 baseline executables. *A/B*: the baseline
   copies and the final build, alternating executable sets, three rounds of all 70
   (`ab-final`), and five alternating rounds of the two changed executables
   (`ab-focused`). Times are medians.
4. **Load.** Before each executable set of a round, the CPU time of every other process
   over three seconds was sampled; above 10 % of the machine the round waited (up to about
   ten minutes) and sampled again. After each executable, compiler and linker processes
   were counted.
5. **Tails.** For the largest executables, each test's end was stamped as its result
   arrived, and its start taken as its end less its `exec_time`, to see which tests end
   last.

The scripts are this pass's own, not repository tools.

## Environment

Windows 11 Enterprise 10.0.26200, i7-13800H (20 logical CPUs), 31.6 GB, NTFS on a data
volume; Rust 1.99.1. The survey ran on a quiet machine (other processes 0.7–1.8 % of the
CPUs before each round, no compiler process in any of its 210 samples). From the second
hour, other work on the machine (builds, Clippy and acceptance runs of other checkouts)
took 5–40 % of the CPUs for minutes at a time; see [Executables](#executables) for how
the A/B rounds were affected.

## Results

### First runs

The survey's first round ran each executable for the first time after the build. Three
were then much slower than later, from cold system state rather than test code:

- `midnight-explorer` `windows_batch` 20.3 s (0.37 and 0.38 s later): its one test runs a
  batch file twice through the application executable as console launcher, whose first
  start after the build took the time; listing the test executable beforehand does not
  start the application executable.
- `midnight-platform-windows` library 4.1 s (2.1 and 2.0 s later): the terminal tests'
  `cmd` and PowerShell children, as on 25 November.
- `midnight-worker-platform` `containment` 1.5 s (0.16 and 0.17 s later).

The medians below are of warm runs. CI runs every executable once, cold.

### Executables

*Survey* is the median of the three survey rounds (08:30–08:43, quiet). *Before* and
*After* are `ab-final`: the baseline copies and the final build in three alternating
rounds (10:13–10:35), on a machine that others' compilers kept busy (one compiler process
or more in 372 of 420 samples, 5–20 % of the CPUs before each set): the large
executables took 20–50 % longer in both halves than in the survey (`midnight-ui`
integration 18.9 → 25.1 s, the viewer engine's library 13.5 → 18.5 s, the engine library
13.5 → 17.2 s, `midnight-ui`'s library 4.3 → 6.4 s). *25 Nov after* is that record's
*After* column.

| Executable | Survey s | Before s | After s | Passed/failed/ignored after | 25 Nov after s |
| --- | ---: | ---: | ---: | --- | ---: |
| `midnight-ui::test:integration` | 18.93 | 25.10 | 25.74 | 1260/0/11 | 19.29 |
| `midnight-viewer-engine::lib:midnight_viewer_engine` | 13.47 | 18.51 | 18.51 | 671/0/20 | 10.16 |
| `midnight-engine::lib:midnight_engine` | 13.45 | 17.20 | 16.76 | 2056/0/34 | 13.79 |
| `midnight-compare-ui::lib:midnight_compare_ui` | 4.96 | 5.37 | 5.15 | 53/0/2 | 4.93 |
| `midnight-engine::test:integration` | 4.57 | 5.14 | 5.04 | 422/0/14 | 4.50 |
| `midnight-ui::lib:midnight_ui` | 4.31 | 6.37 | 6.81 | 1746/0/14 | 4.20 |
| `midnight-explorer::bin:midnight-explorer` | 2.36 | 2.77 | 2.77 | 139/0/28 | 2.32 |
| `midnight-viewer-ui::lib:midnight_viewer_ui` | 2.30 | 2.73 | 2.83 | 262/0/0 | 2.12 |
| `midnight-update::test:integration` | 2.27 | 2.31 | 0.87 | 66/0/0 | 0.86 |
| `midnight-platform-windows::lib:midnight_platform_windows` | 2.10 | 3.68 | 2.86 | 447/0/28 | 2.03 |
| `midnight-compare-ui::test:integration` | 2.03 | 1.99 | 1.99 | 19/0/0 | 1.92 |
| `midnight-compare-engine::lib:midnight_compare_engine` | 1.66 | 1.68 | 1.74 | 191/0/4 | 1.54 |
| `midnight-viewer-ui::test:integration` | 1.28 | 1.39 | 1.38 | 36/0/0 | 0.75 |
| `midnight-worker-platform::lib:midnight_worker_platform` | 1.27 | 1.28 | 1.27 | 43/0/1 | — |
| `midnight-viewer-render::lib:midnight_viewer_render` | 1.06 | 1.36 | 1.26 | 46/0/0 | 1.02 |
| `midnight-filesystem-host::lib:midnight_filesystem_host` | 1.06 | 1.07 | 1.07 | 135/0/1 | 1.05 |
| `midnight-platform-common::lib:midnight_platform_common` | 1.00 | 1.29 | 1.32 | 208/0/0 | 0.50 |
| `midnight-office-preview::test:performance` | 0.84 | 0.95 | 0.97 | 3/0/1 | 0.80 |
| `midnight-platform-common::test:integration` | 0.83 | 1.21 | 1.21 | 186/0/0 | 0.69 |
| `midnight-thumbnail-host::lib:midnight_thumbnail_host` | 0.74 | 0.74 | 0.74 | 51/0/5 | 0.73 |
| `midnight-office-preview::lib:midnight_office_preview` | 0.66 | 0.77 | 0.75 | 86/0/0 | 0.63 |
| `midnight-elevation::lib:midnight_elevation` | 0.64 | 0.65 | 0.65 | 31/0/0 | 0.64 |
| 48 other executables, together | 5.12 | 5.96 | 5.69 | | |
| **All 70** | **86.9** | **109.5** | **107.4** | | |

Since 25 November, `midnight-viewer-engine`'s library grew from 10.2 to 13.5 s (the new
8000-byte row tests of `dfd80cb7`, below), `midnight-update`'s integration executable
from 0.86 to 2.27 s (a stray child process, changed here), and the workspace from 69 to
70 executables (`midnight-explorer`'s `windows_batch`). The others are within the
machine's range of 25 November.

### What ends each large executable

| Executable | Wall s | Its last tests |
| --- | ---: | --- |
| `midnight-ui` integration | 16.5 | no single test: 1,260 tests of 346 s summed `exec_time` on 20 threads; the last are `tree_watch` (2.0 s each, starting at 14.2 s) |
| `midnight-viewer-engine` library | 13.5 | `long_rows::rows_of_8000_bytes_fill_the_window_at_rest::{text,source}` (12.8–13.1 s, from the start) and `rows_of_8000_bytes_keep_every_gesture_s_travel::{text,source}` (11.0–11.6 s) |
| `midnight-engine` library | 12.9 | the `watch::tests` and `user_command_output` tests, which sleep rather than compute and start last (alphabetical): `a_folder_without_notifications_is_checked_often_and_reported_only_when_it_lasts` (3.85 s from 9.05 s; changed here), `stop_ends_the_tree_and_kills_it_when_asked_politely_in_vain` (3.17 s, the three-second stop grace), `the_last_changes_of_a_folder_that_kept_changing_are_hinted_once_they_settle` (2.61 s) |
| `midnight-compare-ui` library | 5.0 | `scroll_parity_tests::touchpad_motion_and_its_momentum_move_the_text_as_in_the_text_viewer` (4.85 s, one schedule, on record) |
| `midnight-engine` integration | 4.7 | `watch_native::a_failed_registration_is_checked_often_and_retried_without_a_status` (3.19 s from 1.49 s, production timing) |
| `midnight-ui` library | 3.7 | the `scroll_input::pane_tests` layout tests (1.7–2.4 s) |
| `midnight-update` integration | 2.2 | no test: every test ended by 0.86 s, the process exited at 0.87–0.96 s, and its output closed at 2.25–2.29 s (changed here) |

### Tests over 1 s

The survey found 239 tests over 1 s (25 November: 201). The 40 slowest, with the
25 November record's *After* where its appendix has the test (*—*: under 1 s then, or
new):

| Survey s | Before s | After s | 25 Nov s | Executable | Test |
| ---: | ---: | ---: | ---: | --- | --- |
| 13.01 | 17.93 | 17.80 | — | viewer-engine lib | `worker::tests::long_rows::rows_of_8000_bytes_fill_the_window_at_rest::source` |
| 12.80 | 17.64 | 17.57 | — | viewer-engine lib | `worker::tests::long_rows::rows_of_8000_bytes_fill_the_window_at_rest::text` |
| 11.41 | 15.58 | 15.77 | 9.33 | viewer-engine lib | `worker::tests::long_rows::rows_of_8000_bytes_keep_every_gesture_s_travel::source` |
| 11.20 | 14.87 | 14.84 | 9.39 | viewer-engine lib | `worker::tests::long_rows::rows_of_8000_bytes_keep_every_gesture_s_travel::text` |
| 9.70 | 13.86 | 12.53 | 10.63 | engine lib | `copy_operations::input::root_bound_tests::a_panel_selection_of_more_native_roots_is_refused_before_any_root_is_prepared` |
| 7.91 | 10.23 | 9.31 | 4.73 | ui test | `native_operation_layout::native_operation_layout_preserves_long_unicode_content_and_action_bounds` |
| 6.87 | 8.25 | 8.70 | 5.49 | viewer-engine lib | `worker::tests::long_rows::rows_of_2400_bytes_keep_every_gesture_s_travel::source` |
| 6.85 | 8.33 | 8.62 | 5.47 | viewer-engine lib | `worker::tests::long_rows::rows_of_2400_bytes_keep_every_gesture_s_travel::text` |
| 6.31 | 7.96 | 8.25 | 5.58 | viewer-engine lib | `image_worker::document_band_tests::a_fast_drag_publishes_the_cameras_it_passes` |
| 5.88 | 8.11 | 8.16 | 6.87 | engine lib | `copy_operations::tests::streamed::a_streamed_move_across_volumes_removes_each_source_only_after_its_verified_copy` |
| 5.81 | 6.20 | 6.08 | 5.96 | ui test | `busy_folder_native::a_refresh_that_changes_nothing_visible_leaves_the_model_and_every_row_untouched` |
| 5.77 | 7.50 | 7.76 | 6.62 | ui test | `mouse::actual_mouse_selection_and_activation_preserve_entry_identity` |
| 5.55 | 4.41 | 4.59 | 2.98 | engine lib | `copy_operations::tests::moves::directories::recursive_move_crosses_native_enumeration_pages_without_losing_children` |
| 5.28 | 9.25 | 9.39 | 5.68 | engine lib | `checksum::manifest::tests::exports_of_any_size_are_encoded_and_read_back` |
| 5.27 | 5.68 | 5.81 | — | viewer-engine lib | `worker::tests::long_rows::rows_of_8000_bytes_take_scrollbar_positions_after_page_down::source` |
| 5.24 | 5.68 | 5.87 | — | viewer-engine lib | `worker::tests::long_rows::rows_of_8000_bytes_take_scrollbar_positions_after_page_down::text` |
| 5.15 | 6.95 | 8.03 | 5.65 | engine lib | `copy_operations::tests::archive::a_cancelled_selection_reaches_a_waiting_ui_after_a_few_events` |
| 5.11 | 7.86 | 7.39 | 4.74 | engine lib | `checksum::format::tests::a_manifest_of_any_size_parses` |
| 5.00 | 6.60 | 6.39 | 3.25 | ui test | `filename_colors::default_palette_renders_every_category_with_contrast_in_every_theme_and_selection_state` |
| 4.90 | 6.03 | 5.91 | 2.85 | ui test | `themes::all_themes_are_selectable_and_keep_text_readable` |
| 4.81 | 5.25 | 5.46 | 4.34 | viewer-engine lib | `worker::tests::long_rows::rows_of_1200_bytes_keep_every_gesture_s_travel::text` |
| 4.78 | 5.16 | 4.95 | 4.78 | compare-ui lib | `scroll_parity_tests::touchpad_motion_and_its_momentum_move_the_text_as_in_the_text_viewer` |
| 4.77 | 5.45 | 5.29 | 4.53 | viewer-engine lib | `worker::tests::long_rows::rows_of_1200_bytes_keep_every_gesture_s_travel::source` |
| 4.47 | 4.79 | 4.99 | 4.93 | ui test | `busy_folder_native::a_folder_that_keeps_changing_is_relisted_at_a_bounded_rate_and_converges` |
| 4.44 | 4.64 | 4.64 | 4.35 | compare-ui lib | `scroll_parity_tests::the_scrolling_glide_and_speed_settings_apply_as_in_the_text_viewer::glide_very_long` |
| 4.41 | 5.68 | 5.57 | 4.78 | engine lib | `copy_operations::tests::streamed::a_vanished_item_fails_alone_and_retry_takes_it_again` |
| 4.41 | 5.51 | 5.78 | 4.00 | viewer-engine lib | `worker::tests::long_rows::rows_of_2400_bytes_fill_the_window_at_rest::text` |
| 4.21 | 4.39 | 4.36 | 4.22 | ui test | `busy_folder_native::a_changed_file_updates_only_its_row_and_keeps_colors_and_folder_hints` |
| 4.19 | 4.77 | 4.92 | 3.88 | viewer-engine lib | `worker::tests::long_rows::rows_of_2400_bytes_fill_the_window_at_rest::source` |
| 4.06 | 7.56 | 5.85 | 4.81 | engine lib | `clipboard::paste_capture_tests::paste_refuses_more_native_roots_than_one_copy_takes_as_it_reads_them` |
| 3.98 | 4.14 | 4.17 | 3.81 | compare-ui lib | `scroll_parity_tests::scroll_keys_and_horizontal_scrolling_move_the_text_as_in_the_text_viewer::horizontal_notch` |
| 3.97 | 5.02 | 5.26 | 3.64 | viewer-engine lib | `worker::tests::frontier::a_notch_up_after_a_scrollbar_position_needs_no_corrected_surface` |
| 3.95 | 5.42 | 5.35 | 4.05 | engine lib | `copy_operations::tests::archive::a_copy_reading_the_folder_an_archive_copy_stages_in_waits_for_the_whole_copy` |
| 3.86 | 3.85 | 1.93 | 3.85 | engine lib | `watch::tests::a_folder_without_notifications_is_checked_often_and_reported_only_when_it_lasts` |
| 3.75 | 4.10 | 4.40 | 4.38 | engine lib | `copy_operations::tests::streamed::a_streamed_copy_takes_each_item_and_totals_them_item_by_item` |
| 3.64 | 4.63 | 4.82 | 3.57 | viewer-engine lib | `worker::tests::row_groups::a_refill_shapes_only_the_rows_it_adds` |
| 3.62 | 5.18 | 5.00 | 3.69 | engine lib | `copy_operations::tests::archive::a_paused_archive_copy_keeps_its_staging_folder_and_resumes_in_it` |
| 3.47 | 3.86 | 4.44 | 3.51 | engine lib | `copy_operations::tests::streamed::an_item_replaced_after_its_identity_was_read_fails_alone_and_is_never_copied` |
| 3.35 | 3.58 | 3.47 | 3.28 | compare-ui lib | `scroll_parity_tests::scroll_keys_and_horizontal_scrolling_move_the_text_as_in_the_text_viewer::arrow_right_x2` |
| 3.33 | 4.82 | 4.73 | 3.18 | engine test | `checksums_backend::checksum::manifest::tests::exports_of_any_size_are_encoded_and_read_back` |

## Changes

| Change | Before | After |
| --- | --- | --- |
| `midnight-engine` watch: the unwatched-folder test's report time 2 s → 1.8 s (`b2263a61`) | 3.85 s (five rounds, 3.85 each) | 1.92 s (1.91–1.93) |
| `midnight-engine` root-bound tests: a million paths below the volume root (`f07e884f`) | panel selection 3.49–3.63 s alone, 14.06 s in the library under load | 2.96–2.97 s alone, 12.97 s in the library under load |
| `midnight-update` running-executable test: no grandchild process (`c0a12a8b`) | executable 2.24 s (output closed 1.3 s after exit) | 0.88 s |

*Before* and *After* are `ab-focused` medians unless the cell says *alone* (the test alone
in its executable, two runs).

### Watch report time (`watch/tests.rs`)

`a_folder_without_notifications_is_checked_often_and_reported_only_when_it_lasts` injects
a 40 ms check, a 60 ms first retry and a report time, and asserts that a change is found
before the report time, that `PollingOnly` comes no earlier than it and once, and that the
registration was retried at least four times and the folder checked at least five times
meanwhile. Temporary timers showed the change found after 88 ms and the status after
3.84 s, not 2 s: a failed registration is retried after twice each previous wait
(`Retry::delay`), at 0.06, 0.18, 0.42, 0.90, 1.86 and 3.78 s, and the status comes with the
first failed retry at or after the report time. With 2 s that was the sixth retry. A
report time of 1.8 s is met by the fifth (1.86 s): the retry and check counts keep wide
margins (five retries for at least four, about 45 checks for at least five), the "found
before the report" margin is 1.7 s instead of 1.9 s, and a retry delayed by load comes
later, which still meets the report time. The 21 November pass asked for a margin study
before a shorter report time: it passed in 17 runs of the whole engine library, most
of them on a machine 5–20 % busy with others' builds (one of the 17 runs failed on a
test its runner did not record), and in six runs of three concurrent copies of the library (60 test threads on 20 CPUs), in
which one copy failed on the unchanged
`copy_operations::tests::archive::a_cancelled_selection_reaches_a_waiting_ui_after_a_few_events`
("46 events (for 600 copied roots) came before the job's end; at most 42"). The test was
among the library's last to start.

### Root-bound paths (`copy_root_bound_tests.rs`)

Both tests build 1,000,001 paths (`MAX_NATIVE_TRANSFER_ROOTS` + 1), and the panel
selection test an index and selection of them, below the current directory. Nothing is
created or read there; every request is refused, or accepted, by its count. The paths now
start at the current directory's volume root, the shortest absolute folder. Temporary
timers after the change: building the entries 0.49 s, the index 0.54 s, the selection
0.23 s, the refused `resolve` 1.38 s. Both tests keep the count, the bound's message and
the real index, selection and `CopyInput::resolve` path.

### Running-executable rename (`apply_windows.rs`)

`a_running_executable_can_be_renamed` started a copy of the command interpreter as
`/c ping -n 3 127.0.0.1` and killed it after renaming its file. The kill ended the
interpreter but not its `ping`, which kept the test process's inherited output open for
about two more seconds: the executable exited after 0.87–0.96 s, its output closed after
2.25–2.29 s, and a stray process outlived each run. A scan of all 70 executables
(process exit against the close of its output) found no other. The copy now reads
commands from a pipe the test keeps open, without output handles; it runs until killed,
which a new assertion checks before the rename. It still proves that a running process's
executable file can be renamed.

## Looked at, not changed

- **8000-byte rows in the viewer engine** (`long_rows`, the library's tail). Temporary
  timers in `fills_the_window_at_rest`: 93 % of its 7.1 s alone is the worker's own work,
  of which 63 % the 626 plugin replies (one row of 8000 bytes each, 6.6 ms to take and
  lay out), 13 % painting and 21 % strip preparation; the simulated window and plugin
  take the rest. Beside the library's other tests it takes 13 s. A 22-row window (as the
  travel tests use, `ROWS_22`) was tried: alone it took longer (7.9–8.1 s), and with the
  fix of `dfd80cb7` switched off it caught the blank window only after the opening, not
  after the scrollbar position; not adopted. Fewer gestures would check fewer states.
- **`native_operation_layout`** (7.9 s, 4.7 s on 25 November): laid out for every theme
  (46 now), four densities and fonts and five kinds. Temporary timers: 2.16 s of its
  3.09 s alone is `begin`, which shows the form's popup anew, as the adapter does for each
  operation; 0.18 s the 20 renders of the first theme, 0.05 s the other layouts.
  Several other "every theme" tests grew the same way (`filename_colors` 3.3 → 5.0 s,
  368 renders of 4.6 ms; `themes` 2.9 → 4.9 s).
- **Fixed waits of production timing**: the user-command stop test's three-second
  `STOP_GRACE` (which it asserts), the comparator settings grace, the real-engine
  `watch_native` and `busy_folder_native` windows, as on 21 and 25 November.
- **Fixtures whose size is what they test**: the root-bound tests' 1,000,001 items, the
  streamed copies' 193 files, the any-size checksum manifests, the native enumeration
  pages, as on 25 November.
- **`midnight-ui` integration**: its wall is its summed work (346 s on 20 threads); the
  largest modules are real-engine and native tests (`busy_folder_native` 18.6 s summed,
  `tree_clipboard` 13.5 s, `tree_move` 11.8 s, `tree_folder_sizes` 10.3 s).

## Obsolete code and tests

None removed. No new duplicate was found among the tests over 1 s; the checksum library
tests that the engine integration executable runs again are the documented `#[path]`
includes, as on 25 November. No dead-code probe was made in this pass.

## Validation

- Builds: the baseline and the final build built every workspace test executable
  without an error.
- Survey: every executable passed in all three rounds (9,416 tests passed and 225 ignored
  in each round). `ab-final`: every executable of both builds passed in all three rounds
  (9,416 passed and 225 ignored in each round of each build; no test was added or
  removed).
- An earlier attempt at `ab-final` (09:28–09:57) met 2–41 % of other load before its
  sets and is not used for timing; four unchanged tests failed in it, each once:
  `busy_folder_native::a_changed_file_updates_only_its_row_and_keeps_colors_and_folder_hints`
  and `a_refresh_that_changes_nothing_visible_leaves_the_model_and_every_row_untouched`
  ("decorations never settled", after 20.9 s, in a `midnight-ui` integration run of
  68.9 s, baseline build), `office_preview::tests::office_prepares_a_workbook_whose_change_time_alone_moved_after_capture`
  ("missing ZIP end record", baseline build) and
  `volumes::unchanged_hints_publish_nothing_and_a_burst_coalesces` ("4 discoveries for
  one burst", final build). They passed in every other run; not investigated further.
- `apply_windows::a_running_executable_can_be_renamed` fails now and then, before and
  after the change: alone, the unchanged test failed 2 of 15 times with a sharing
  violation on the rename of its freshly copied executable (most likely a scanner
  reading it), and the changed test 4 of 90 times, all 4 within the first ten runs after
  its build (their message was not kept). It passed in every executable run of the A/B.
  Not changed further: a retry would test the updater's retry, not the rename.
- The changed watch test: see [Watch report time](#watch-report-time-watchtestsrs).
- `cargo clippy --workspace --all-targets --locked --features midnight-acceptance/acceptance
  -- -D warnings` (with `--offline -j 6`), after the last change: no warning.
- `cargo fmt --all --check`: clean.
- No red run of a production fault was made for the three changes: none changes an
  assertion's condition (one assertion was added), and the watch test's timing is
  derived above from the production backoff.

## Limitations

- After the survey, the machine was shared with other checkouts' builds and acceptance
  runs: the A/B rounds met 3–34 % of other load before their sets, so their totals and
  the large executables' walls are not comparable with the survey's. Compare the changed
  tests and executables' alternating pairs.
- Windows debug builds only. Linux was neither timed nor built: two changes are
  platform-neutral test code, the third a Windows-only test.
- The survey's first round is of executables started for the first time after the build;
  the medians are of warm runs, CI's runs are cold.
- Eight of the final build's 70 executables differ from the baseline copies: the two
  changed ones and six rebuilt from their unchanged sources after temporary timers were
  removed (`midnight-ui` and `midnight-viewer-ui` integration, the viewer engine's and
  viewer UI's libraries, `midnight-explorer`'s binary and `windows_batch`); the builds
  are not byte-reproducible here, so those six are compared by timing only.
