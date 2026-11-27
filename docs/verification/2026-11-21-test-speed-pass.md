# Test speed pass — 2026-11-21

## Scope

Every Windows test of the root workspace's library, binary and integration test
executables was timed on the review branch at `cd2f821a`; the tests added or slowed since
`3987d3c8` and others over 1 s were made faster where that loses nothing they prove, and
obsolete tests and code were looked for. The plugins under `plugins/` are separate
workspaces and were not timed. Follow-up to [test execution time](2026-11-18-test-execution-time.md)
and the [test speed pass](2026-11-18-test-speed-pass.md) of 18 November.

## Method

1. **Builds.** `cargo test --workspace --no-run --message-format=json-render-diagnostics
   --locked --offline -j 6` in a target directory of its own, debug test profile, debug
   information off (`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`) and
   `CARGO_INCREMENTAL=0`, as CI's `check` job builds. The baseline was built at 11:19 from
   the tree that became `cd2f821a` (its WIN-8 files were byte-identical in the working
   tree, and `a045453e`'s test change was already there), 67 test executables. The final
   build is a `git archive` export of `cd2f821a` plus this pass's ten changed files only,
   so that other agents' uncommitted work in the shared tree stayed out; the export
   carries no `.cargo/config.toml` of its own, so it sees the same Cargo configuration
   files as the checkout, and the user-command probe cache was copied along, as a warm
   checkout has it.
2. **Runs.** Each executable runs alone, one after another, from its package directory
   as `cargo test` runs it, with libtest's default thread count (20 here; CI does not pass
   `--test-threads` either, so its runners use their own CPU count), JSON output and
   `-Z unstable-options --report-time` (`RUSTC_BOOTSTRAP=1` for the test process only), a
   private temporary directory below the checkout's `_build`, and
   `MIDNIGHT_UI_RENDER_DIR` unset. Before its timed rounds each executable lists its
   tests twice and its ignored tests once, which pays a new file's first start (see
   [test execution](../developer/test-execution.md)) outside the timed runs; every wall
   time below is therefore of an executable that started moments before. A test's time
   is libtest's `exec_time`, measured while the other tests of its executable run beside
   it, so it grows with contention.
3. **Rounds.** Survey: two rounds of all 67 baseline executables. `ab1`: the changed
   executables of the first changed build in three rounds alternating with copies of the
   baseline executables. Final: three rounds of all 67 executables of the final build.
   `ab2`: all 67 executables of the baseline (copies) and of the final build in three
   alternating rounds, so that both meet the same load. Times are medians.
4. **Red runs.** A separate export with this pass's files and the fixes behind the changed
   tests undone behind run-time switches (one for the path bar's Slint fix, which is
   compile time), built in its own target directory; each changed test ran with and
   without each switch. The shared tree was never changed for them.
5. **Dead code.** The same export, with every `allow(dead_code)` of `midnight-ui`'s
   integration test files removed, reported its dead-code warnings; each reported item
   was then looked for in every test file that could include it.

The scripts are the orchestrator's (the 18 November pass's `ab.py`, `snap.py` and
`inplace.py`, with a sampler of compiler processes added, and new report scripts), not
repository tools. The results are in the checkout's `_build/qa/speed21/` (`base`, `ab1`,
`final`, `ab2`, `red`, `dead-probe.txt`).

No private plugin installation root (`HOST_TEST_INSTALL_ROOT`) could be made: drive C:
is full, and every folder on D: has ancestors that Authenticated Users may change, which
plugin discovery refuses. 52 `midnight-plugin-host` and 11 `midnight-image-viewer-host`
library tests therefore fail in every run, before and after, as in the 17 November
baseline; their times are not meaningful.

## Environment

Windows 11 Enterprise 10.0.26200, i7-13800H (20 logical CPUs), 31.6 GB, NTFS on D:,
Microsoft Defender real-time protection on (signatures 1.463.398.0); Rust 1.99.1. A shared
machine: other agents compiled, linted and ran tests during parts of the sessions (a
compiler ran in 16 of 57 five-second samples of the survey, 8 of 59 of `ab1`, 12 of 58 of
the final run and none of 82 of `ab2`; see [Limitations](#limitations)).

## Results

### Executables

*Survey* is the baseline's two rounds (11:34–11:41; a compiler ran in 16 of 57 five-second
samples). *Before* and *After* are `ab2`: all 67 executables of the baseline (copies) and of
the final build in three alternating rounds (12:41–12:56; no compiler ran in 82 samples).
The same baseline executables took 0.3 to 2.6 times their survey time in `ab2`, most of
them longer (the engine library 12.13 → 19.33 s, `midnight-office-preview`'s performance
test 0.96 → 2.41 s): the machine's state changed between the sessions, from other load
or its temperature, while each before/after pair of `ab2` met the same state. *18 Nov* is each
executable's first start right after its build on 18 November (`d0f44c55`), which
includes Defender's scan of the new file and is not comparable to the warm walls here.

| Executable | Survey s | Before s | After s | Passed/failed/ignored after | 18 Nov s |
| --- | ---: | ---: | ---: | --- | ---: |
| `midnight-ui::test:integration` | 14.83 | 21.19 | 21.18 | 972/0/2 | 24.15 |
| `midnight-engine::lib:midnight_engine` | 12.13 | 19.33 | 15.42 | 1851/0/19 | 14.11 |
| `midnight-viewer-engine::lib:midnight_viewer_engine` | 8.14 | 7.48 | 4.63 | 575/0/14 | 4.32 |
| `midnight-ui::lib:midnight_ui` | 4.62 | 5.08 | 2.80 | 1450/0/5 | 12.50 |
| `midnight-engine::test:integration` | 4.21 | 5.27 | 5.16 | 475/0/10 | 13.89 |
| `midnight-compare-ui::test:integration` | 2.68 | 2.18 | 2.09 | 19/0/0 | 12.55 |
| `midnight-platform-windows::lib:midnight_platform_windows` | 2.30 | 3.08 | 3.01 | 347/0/18 | 6.02 |
| `midnight-viewer-ui::lib:midnight_viewer_ui` | 2.28 | 1.86 | 1.80 | 209/0/0 | 9.51 |
| `midnight-compare-engine::lib:midnight_compare_engine` | 1.66 | 2.59 | 2.51 | 177/0/4 | 3.58 |
| `midnight-explorer::bin:me` | 1.22 | 1.47 | 1.22 | 130/0/27 | 14.63 |
| `midnight-office-preview::test:performance` | 0.96 | 2.41 | 2.30 | 3/0/1 | — |
| `midnight-office-preview::lib:midnight_office_preview` | 0.79 | 1.96 | 1.93 | 68/0/0 | — |
| `midnight-platform-common::test:integration` | 0.64 | 1.53 | 1.54 | 175/0/0 | 5.48 |
| `midnight-elevation::lib:midnight_elevation` | 0.63 | 0.65 | 0.65 | 25/0/0 | 2.00 |
| `midnight-worker-platform::test:containment` | 0.61 | 0.18 | 0.15 | 1/0/0 | 1.41 |
| `midnight-platform-common::lib:midnight_platform_common` | 0.50 | 1.08 | 1.07 | 158/0/0 | 5.08 |
| `midnight-viewer-ui::test:integration` | 0.45 | 0.46 | 0.45 | 34/0/0 | 7.65 |
| `midnight-compare-ui::lib:midnight_compare_ui` | 0.44 | 1.09 | 1.23 | 33/0/0 | 8.02 |
| `midnight-viewer-render::test:integration` | 0.43 | 0.26 | 0.28 | 58/0/3 | 3.49 |
| `midnight-viewer-render::lib:midnight_viewer_render` | 0.36 | 0.26 | 0.18 | 38/0/0 | 3.30 |
| `midnight-web-viewer-host::lib:midnight_web_viewer_host` | 0.33 | 0.33 | 0.33 | 81/0/0 | 0.36 |
| `midnight-filesystem-host::lib:midnight_filesystem_host` | 0.30 | 0.78 | 0.74 | 105/0/1 | 2.68 |
| `midnight-thumbnail-host::test:integration` | 0.25 | 0.37 | 0.41 | 24/0/8 | 1.88 |
| 44 other executables, together | 2.37 | 4.02 | 4.04 | | 67.60 |
| **All 67** (66 on 18 Nov) | **63.1** | **84.9** | **75.1** | | **238.5** |

The changed executables, before → after in `ab2`: the engine library 19.33 → 15.42 s, the
viewer engine's library 7.48 → 4.63 s, `midnight-ui`'s library 5.08 → 2.80 s;
`midnight-ui`'s integration executable 21.19 → 21.18 s, whose 1.7 s less of the path bar
test is within its range between rounds. In `ab1` (11:58, three alternating rounds of
the changed executables): 11.09 → 9.46 s, 7.57 → 4.56 s and 4.92 → 2.19 s. All other
executables run the same tests; their differences are noise.

### Slowest tests

The survey's 40 slowest of its 7,973 timed tests, with `ab2`'s before and after (for a
test split into several, its slowest part) and libtest's time on 18 November where the
test existed. The survey found 254 tests over 0.5 s and 130 over 1 s; the
[appendix](#appendix-tests-over-05-s) lists all of them.

| Survey s | Before s | After s | 18 Nov s | Executable | Test |
| ---: | ---: | ---: | ---: | --- | --- |
| 7.96 | 7.35 | 4.48 (4 tests) | — | viewer-engine lib | `worker::tests::long_rows::rows_of_2400_bytes_keep_every_gesture_s_travel` |
| 6.85 | 6.39 | 3.77 (4 tests) | — | viewer-engine lib | `worker::tests::long_rows::rows_of_8000_bytes_keep_every_gesture_s_travel` |
| 6.78 | 6.42 | 4.49 (2 tests) | — | viewer-engine lib | `worker::tests::long_rows::rows_of_2400_bytes_fill_the_window_at_rest` |
| 6.48 | 8.82 | 3.65 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::followed_source_folder_still_holds_back_a_write_into_it` |
| 6.28 | 9.38 | 2.73 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::reading_the_busy_copys_output_waits_for_it_with_its_reason` |
| 6.21 | 9.03 | 2.67 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::wait_for_earlier_copy_still_waits_for_the_busy_copy` |
| 6.06 | 8.65 | 2.33 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::paused_followed_copy_does_not_hold_back_a_sibling_folder_copy` |
| 5.83 | 6.22 | 6.36 | — | ui test integration | `busy_folder_native::a_refresh_that_changes_nothing_visible_leaves_the_model_and_every_row_untouched` |
| 5.72 | 9.11 | 3.13 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::same_named_item_waits_alone_while_the_rest_of_the_copy_proceeds` |
| 5.69 | 8.90 | 1.94 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::immediate_copy_into_another_folder_progresses_beside_a_busy_copy` |
| 5.66 | 5.24 | 3.21 (4 tests) | — | viewer-engine lib | `worker::tests::long_rows::rows_of_1200_bytes_keep_every_gesture_s_travel` |
| 5.58 | 7.08 | 5.33 | 2.86 | engine lib | `copy_operations::tests::moves::directories::recursive_move_crosses_native_enumeration_pages_without_losing_children` |
| 5.56 | 8.98 | 2.41 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::immediate_copy_into_the_same_folder_progresses_beside_a_busy_copy` |
| 5.40 | 9.03 | 2.71 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::the_busy_copy_publishes_into_output_that_a_waiting_copy_prepared` |
| 5.15 | 8.59 | 1.99 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::one_disk_slot_interleaves_a_second_copy_with_a_busy_copy` |
| 4.93 | 6.85 | 5.36 | 2.25 | engine lib | `copy_namespace::tests::one_hundred_thousand_sibling_metadata_roots_use_indexes_and_shared_parent` |
| 4.83 | 5.68 | 5.71 | — | ui test integration | `busy_folder_native::a_folder_that_keeps_changing_is_relisted_at_a_bounded_rate_and_converges` |
| 4.66 | 4.23 | 2.51 (4 tests) | — | viewer-engine lib | `worker::tests::long_rows::rows_mixing_short_and_very_long_ones_keep_every_gesture_s_travel` |
| 4.38 | 6.88 | 6.43 | 1.54 | ui test integration | `mouse::actual_mouse_selection_and_activation_preserve_entry_identity` |
| 4.10 | 4.47 | 4.63 | — | ui test integration | `busy_folder_native::a_changed_file_updates_only_its_row_and_keeps_colors_and_folder_hints` |
| 3.85 | 3.84 | 3.84 | — | engine lib | `watch::tests::a_folder_without_notifications_is_checked_often_and_reported_only_when_it_lasts` |
| 3.80 | 4.16 | 1.68 | — | ui lib | `scroll_input::pane_tests::kinetic_tests::x11_fractional_line_stream_follows_the_finger_then_coasts` |
| 3.74 | 4.10 | 1.69 | — | ui lib | `scroll_input::pane_tests::kinetic_tests::phased_touchpad_flick_coasts_once_with_decaying_steps_then_settles` |
| 3.54 | 6.27 | 6.61 | 2.91 | engine lib | `operations::metadata_tests::traversal::recursive_edit_changes_every_entry_of_a_folder_with_1000_entries` |
| 3.44 | 5.51 | 4.95 | 1.89 | engine lib | `copy_operations::tests::moves::directories::recursive_move_depth_budget_retains_unfinished_tree` |
| 3.38 | 5.00 | 3.29 | — | ui test integration | `path_bar_fit::path_components_and_the_edit_area_stay_inside_the_bar_at_every_width` |
| 3.30 | 4.68 | 4.46 | 5.64 | engine test integration | `operations_real_windows::real_directory_corpus_matches_independent_sha256_for_both_policies` |
| 3.19 | 3.20 | 3.20 | — | engine test integration | `watch_native::a_failed_registration_is_checked_often_and_retried_without_a_status` |
| 3.15 | 6.12 | 5.70 | 2.59 | engine lib | `operations::metadata_tests::traversal::queued_metadata_jobs_holding_every_shared_target_cannot_limit_running_recursive_edits` |
| 3.05 | 5.15 | 4.73 | 4.08 | ui test integration | `native_operation_layout::native_operation_layout_preserves_long_unicode_content_and_action_bounds` |
| 2.66 | 4.06 | 3.89 | 2.40 | ui test integration | `filename_colors::default_palette_renders_every_category_with_contrast_in_every_theme_and_selection_state` |
| 2.54 | 3.73 | 3.66 | 2.08 | ui test integration | `drag_integration::folder_targets_follow_actual_scrolled_cells_in_details_brief_and_thumbnail_modes` |
| 2.50 | 3.04 | 3.08 | 1.08 | ui test integration | `operations_native_ui::real_folder_drag_uses_same_volume_move_ctrl_copy_and_escape_cancellation` |
| 2.49 | 3.66 | 3.61 | 2.87 | ui test integration | `themes::all_themes_are_selectable_and_keep_text_readable` |
| 2.42 | 3.47 | 3.45 | 2.67 | ui test integration | `tree_clipboard::copy_nested_folders_from_left_tree_and_paste_into_right_tree_root` |
| 2.31 | 3.34 | 3.23 | 2.95 | ui test integration | `tree_clipboard::copy_nested_folders_from_right_tree_and_paste_into_left_tree_root` |
| 2.21 | 3.35 | 3.24 | 2.47 | ui test integration | `tree_clipboard::cut_nested_folders_from_left_tree_and_confirm_native_move_into_right_tree` |
| 2.20 | 3.16 | 3.17 | 2.96 | ui test integration | `tree_clipboard::cut_nested_folders_from_right_tree_and_confirm_native_move_into_left_tree` |
| 2.16 | 3.30 | 3.12 | 1.23 | ui test integration | `native_operation_windows::embedded_copy_draft_shows_every_wrapped_source_completely` |
| 2.16 | 3.07 | 3.17 | 2.38 | ui test integration | `tooltips::shared_tooltips_keep_cursor_clear_in_panels_dropdowns_and_options` |

## Changes

Times are medians of `ab1`'s three alternating rounds of whole executables; a test's time
includes the contention of the tests beside it.

| Change | Before | After |
| --- | --- | --- |
| `midnight-viewer-engine` long-row travel: each of the four travel checks runs its four views (Text and Source, wrapped or not) as four tests, the fill check its two modes as two; the overlap page-key check its two modes as two | 6 tests, 1.92–7.45 s each, 32.1 s together; executable 7.57 s | 20 tests, 1.05–4.44 s each; executable 4.56 s |
| `midnight-ui` touchpad momentum and scrollbar tests: frames lay the window out instead of software-rendering 1340×820 pixels | kinetic tests 4.08 and 4.13 s, scrollbar test 0.81 s; executable 4.92 s | 1.39, 1.43 and 0.35 s; executable 2.19 s |
| `midnight-engine` immediate-start scheduler tests: the busy copy holds 600 small files at 10 ms each instead of 3,000 at 2 ms | 9 tests, 4.50–5.86 s each, 46.2 s together; executable 11.09 s (suite 11.07 s) | 1.02–2.08 s each, 13.5 s together; executable 9.46 s (suite 8.44 s) |
| `midnight-ui` path bar fit: Light Desktop and Dark Touch, without Blue Desktop | 4.13 s (1.19 s alone) | 2.93 s (0.94 s alone) |

### Long-row travel (`worker_long_row_tests.rs`, `worker_overlap_tests.rs`)

The five long-row tests each ran their views one after another and bounded the viewer
engine's library executable (the slowest, `rows_of_2400_bytes_keep_every_gesture_s_travel`,
7.45 s of a 7.57 s executable). A local macro now generates the four views of each travel
check (`::text`, `::text_wrapped`, `::source`, `::source_wrapped`), and the fill and
overlap checks are a test per mode (`::text`, `::source`); libtest runs them in parallel.
Each view keeps its source, window, seeks, gestures, assertions and tolerances; a failure
now names its view, and a filter by a former test name, as in
[viewer presentation](../developer/viewer-presentation.md), still selects all of its
views. The per-test sum grows (32.1 → 47.7 s) because more of them compete for the CPUs at
once. Run alone, the 2400-byte travel views took 0.45 s wrapped and 2.1 to 2.2 s
unwrapped, where 54 layouts of 120 KiB pages and 97 painted frames took 1.1 s and 0.7 s
(`MIDNIGHT_VIEWER_SCROLL_TRACE`); that work is what the views check and stays.

Red runs: without the renderer's cut of layouts into shaping runs of at most 64 KiB
(`ffa6f3fe`), 9 of the 16 travel tests fail (the eight unwrapped views and 8000-byte rows
in Source wrapped), so each former travel test still fails; with pages of 64 KiB again
(`ffa6f3fe`'s `PAGE_BYTES`), both fill tests fail; with a refill reading its whole request
again instead of the rows past the shown page (`6cd20942`), both overlap tests fail.
With the fixes, all 20 pass.

### Touchpad momentum (`scroll_input_pane_tests.rs`, `scroll_input_kinetic_tests.rs`)

The kinetic tests advance a virtual clock frame by frame for up to 400 frames, and every
frame software-rendered the 1340×820 window, although they check scroll offsets only.
`layout_harness()` runs the same frame (timers and animations, the presenter's refresh of
both 1000-row panels, the `RedrawRequested` routing of the native hook) and lays the
window out (`WindowInner::ensure_tree_instantiated`, as the integration tests'
`settle_layout`) without rendering it. The six kinetic tests and the scrollbar test use
it; `real_file_panes_keep_fractional_wheel_positions_during_model_refresh`, which reads
pixels, keeps rendering. Red runs: with the momentum's time constant doubled, the two
travel tests fail; with synthesized momentum off, four kinetic tests fail; with neither,
all pass.

### Immediate start (`copy_immediate_start_tests.rs`)

Each of the nine tests created a folder of 3,000 files of 64 bytes for its busy copy and
removed it afterwards (0.7 s and 0.2 s single-threaded in a probe on D:, much more with
the nine tests and the rest of the library's 1,870 beside them), and the busy copy
published one file per 2 ms. It now holds 600 files and stages one per 10 ms: the same
six-second floor that keeps the busy copy running through every observation, a fifth of
the files, and a fifth of the disk work while the tests run. The library's summed test
time fell from 163 s to 109 s in `ab1` (medians), because its other tests compete with
less disk and CPU work too. Every observation, deadline and assertion is unchanged; the
one fixed wait, 800 ms that a waiting item must keep waiting, stays, as it tests that
nothing admits the item meanwhile.

Red runs: when every Copy waits for earlier transfers (no immediate start), eight of the
nine tests fail (seven at their 10 s start bound, the paused sibling copy at its 30 s
completion bound); when "Wait for other operations" never waits, the ninth,
`wait_for_earlier_copy_still_waits_for_the_busy_copy`, fails. Not red-run: `accfcc53`'s
sharing fix, which `the_busy_copy_publishes_into_output_that_a_waiting_copy_prepared`
checks by holding the prepared source until the busy copy has published two more files
(at most 5 s); the slower pace makes that take about 20 instead of 4 ms and does not
change the check. `reading_the_busy_copys_output_waits_for_it_with_its_reason`, which
caught that regression by chance in CI, now overlaps a publication less often; the
deterministic test covers it.

### Path bar fit (`path_bar_fit.rs`)

The test laid the main window out at 501 widths in three looks: Light and Blue at the
desktop density with 12 px text, and Dark Touch. Themes choose colours and the light or
dark icon variant only (`ui/theme.slint`; no geometry depends on `Theme.scheme` or
`Theme.light`), so Blue repeated Light's layouts exactly. Red run: without WIN-3's pixel
per drawn component (`5fc9d45e`), the test fails at 22 widths in Light Desktop and 19 in
Dark Touch.

## Obsolete code and tests

Removed or changed:

- `native_information_windows.rs`: `UrlRecorder` and `Harness::recording_urls`, unused
  since `4f072e00` removed the About link test that used them, kept compiling by an
  `#[allow(dead_code)]`.
- `midnight-ui` `ChecksumSession::directory`: its `#[allow(dead_code)]` and comment ("only
  tests read it") were stale; Copy as a checksum file reads it since `57680e2d`. The
  comment now says what it is for.
- `midnight-engine` `Operations::confirm` and `OperationWorker::confirm`: wrappers that
  only tests call were compiled into the application under
  `cfg_attr(not(test), allow(dead_code))`; they are `#[cfg(test)]` now.

Looked for, nothing removed:

- Ignored tests: all 204 `#[ignore]` attributes give a reason (opt-in installations,
  desktops, Xvfb displays, volumes, timing harnesses, children of re-run tests). Three
  opt-in groups document their variable only in that reason and run in neither CI nor
  validation: the Linux modeless-filesystem test (`MIDNIGHT_MODELESS_TEST_DIR`), the
  thumbnailer catalog tests (`PLUGIN_CATALOG_TEST_EXECUTABLE`) and the native image
  provider tests (`MIDNIGHT_IMAGE_TEST_HELPER`). They run when given their input, so they
  are listed here, not removed.
- Duplicates: no two test functions have the same body except the
  `every_test_file_is_a_module` guards (each reads its own crate's `tests/`) and two
  Windows/Linux pairs that test each platform's own implementation. The 162 library unit
  tests that `midnight-engine`'s integration executable runs again through `#[path]`
  includes are documented in [test execution](../developer/test-execution.md) and stay.
- Dead test helpers: with every `allow(dead_code)` of `midnight-ui`'s integration test
  files removed, the compiler reported 133 dead-code findings; apart from the two above,
  each item is used by another module that includes its support file, or is a field kept
  for ownership (windows, senders) or for its `Debug` output in a failure message. Rustc
  reports an item once even when a file is compiled into several modules, so these were
  checked by name against every test file. Other `allow(dead_code)` in test code covers sources
  included by path (`midnight-engine`'s checksum, retirement and Find fixtures), files
  shared by several test executables (`midnight-platform-windows`' `test_fixtures.rs`)
  or test-only modules (`#[cfg(test)]`), as their comments say.
- Production code reachable only from tests: beyond the two `confirm` wrappers, every
  `allow(dead_code)` in library sources is on `#[cfg(test)]` items, generated code or a
  field production reads.

## Not changed

- **Real engines and watchers on production timing.** `busy_folder_native` (UI, six tests
  of 1.3–5.8 s) observes a folder that a thread keeps changing through the production
  engine and native watcher, with 1.5 s and 3 s observation windows; the engine
  integration's `watch_native` tests (1.2–3.2 s; three of them took 0.33 s on 18
  November) run a real engine on production timing, whose hints have a one-second
  cooldown since `4edfba2a` (the likely reason for their growth; not attributed per
  test). Shorter windows need the engine's timing injected into these harnesses, a
  production change beyond a test seam.
- **Watch tests with injected timing.** `watch::tests::a_folder_without_notifications_is_checked_often_and_reported_only_when_it_lasts`
  (3.9 s) asserts that its status comes no earlier than its two-second report time,
  `a_folder_that_keeps_changing_is_hinted_at_most_once_per_cooldown` (1.9 s) its cooldown.
  A shorter report time narrows the margin of its "found before the report" and "retried
  at least four times" assertions; that needs a margin study on loaded runners, as the
  18 November pass made for the rejected-work test. These tests sleep rather than
  compute, but alphabetically they start last and may form the library's tail.
- **Fixed engine timing**: `engine_comparator_policy_tests::settings_return_after_shutdown_grace_and_runtime_exit_retires_on_cleanup_worker`
  (2.1 s, the two-second settings grace), as on 18 November.
- **Fixtures whose size is what they test**: the native enumeration-page Move (5.6 s), the
  100,000 sibling metadata roots (4.9 s), the 1,000-entry metadata traversals (2.0–3.5 s),
  the depth-budget Move (3.4 s), the Windows corpus (3.3 s), 100,000 archive subfolders
  (1.4 s) and the 4,096-member archive selection (1.3 s).
- **UI sweeps that lay every theme out** (`native_operation_layout`, `dialog_layout`,
  `command_line_layout`, `user_command_settings_ui`, 1.1–3.1 s): as for the path bar,
  their other themes repeat the first one's geometry, but they also check colours or
  were left to the UI owner by the 18 November pass; candidates for that owner.
- **`midnight-viewer-ui` `raster_frontier_tests`** (five tests of 1.5–1.9 s): each round of
  gestures meets the page frontier at another offset; fewer rounds would test fewer
  alignments.
- **`midnight-compare-ui`** fonts tests: they wait for the product's scan of installed
  fixed-pitch fonts.

## Linux

Not run here: WSL lives on the full drive C:. The changed tests are platform-neutral and
run on Linux too; Linux-only tests (the `midnight-platform-linux` crate, the `cfg(unix)`
files of `midnight-core`, `midnight-engine`, `midnight-platform-common` and
`midnight-platform`, the Linux modules of `midnight-worker-platform`, and the
`target_os = "linux"` tests inside other files) were neither timed nor changed. Clippy for
`x86_64-unknown-linux-gnu` passed for `midnight-engine` (`--all-targets -D warnings`); the
viewer engine and `midnight-ui` need fontique's fontconfig headers for Linux, which this
machine lacks (also with `RUST_FONTCONFIG_DLOPEN=1`), so their Linux type-check is left to
CI. The path bar test keeps one desktop and one touch look on Linux as on Windows.

## Validation

- Builds: the baseline, the first changed build and the final export each built every
  workspace test executable without an error.
- Tests: every round of the final build passed 7,924 tests and ignored 160 (the baseline:
  7,910 and 160); the same 63 tests failed in every round of both builds, the 52
  `midnight-plugin-host` and 11 `midnight-image-viewer-host` library tests that need a
  private installation root ([Method](#method)).
- Test lists (`--list` and `--list --ignored` of every executable, baseline against
  final): 66 of the 67 executables list exactly the same tests; the viewer engine's
  library lists the six former long-row and overlap tests as their 20 parts
  (8,133 → 8,147 tests, 160 ignored in both).
- `cargo clippy -p midnight-viewer-engine -p midnight-engine -p midnight-ui --all-targets
  --locked --offline -j 6 -- -D warnings`: no warning, after the last edit.
- `cargo clippy -p midnight-engine --target x86_64-unknown-linux-gnu --all-targets
  --locked --offline -j 6 -- -D warnings`: no warning.
- `rustfmt --edition 2024 --check` of the ten changed files: clean.
- Red runs as described under [Changes](#changes): each changed test that guards a fix
  fails with that fix undone and passes with it.

[Test execution](../developer/test-execution.md) needs no change: it states no test's
duration, and its first-start figures still hold (the warm-up's first `--list` of the
newly linked 114 MB `me` test executable took 16.7 s, of the 105 MB `midnight-ui`
integration executable 17.7 and 18.0 s, of executables below 5 MB 0.9 to 2.0 s).

## Limitations

- One machine shared with other agents' builds and tests, which ran during parts of the
  timed sessions, and whose speed changed during the day: in `ab2` the baseline
  executables took 0.3 to 2.6 times their survey time. Single executables varied by up to
  6.4 s between rounds of the same build (a round with several seconds more outside the
  test harness, as a first start costs). Only `ab1` and `ab2` alternate the builds;
  compare their pairs, not the survey with the final run.
- Walls of 18 November are of each executable's first start after its build (the method
  of that record), so they include Defender's scan of the new file; this record's walls
  are of executables that started moments before. The per-test times of both are
  libtest's and comparable, with the contention of the day.
- No private installation root: 63 tests of the plugin and image viewer hosts fail in
  every run, and the opt-in tests that need one were not run.
- Windows debug builds only. The Linux tests were neither timed nor run, and the plugin
  workspaces were not timed.
- CI's hosted runners have fewer CPUs than this machine; splitting the long-row tests
  shortens an executable's wall time only where CPUs are free to run them in parallel,
  while the smaller immediate-start fixture saves work everywhere.

## Appendix: tests over 0.5 s

Every test of the survey over 0.5 s, with `ab2`'s before and after and 18 November, as
above.

| Survey s | Before s | After s | 18 Nov s | Executable | Test |
| ---: | ---: | ---: | ---: | --- | --- |
| 7.96 | 7.35 | 4.48 (4 tests) | — | viewer-engine lib | `worker::tests::long_rows::rows_of_2400_bytes_keep_every_gesture_s_travel` |
| 6.85 | 6.39 | 3.77 (4 tests) | — | viewer-engine lib | `worker::tests::long_rows::rows_of_8000_bytes_keep_every_gesture_s_travel` |
| 6.78 | 6.42 | 4.49 (2 tests) | — | viewer-engine lib | `worker::tests::long_rows::rows_of_2400_bytes_fill_the_window_at_rest` |
| 6.48 | 8.82 | 3.65 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::followed_source_folder_still_holds_back_a_write_into_it` |
| 6.28 | 9.38 | 2.73 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::reading_the_busy_copys_output_waits_for_it_with_its_reason` |
| 6.21 | 9.03 | 2.67 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::wait_for_earlier_copy_still_waits_for_the_busy_copy` |
| 6.06 | 8.65 | 2.33 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::paused_followed_copy_does_not_hold_back_a_sibling_folder_copy` |
| 5.83 | 6.22 | 6.36 | — | ui test integration | `busy_folder_native::a_refresh_that_changes_nothing_visible_leaves_the_model_and_every_row_untouched` |
| 5.72 | 9.11 | 3.13 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::same_named_item_waits_alone_while_the_rest_of_the_copy_proceeds` |
| 5.69 | 8.90 | 1.94 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::immediate_copy_into_another_folder_progresses_beside_a_busy_copy` |
| 5.66 | 5.24 | 3.21 (4 tests) | — | viewer-engine lib | `worker::tests::long_rows::rows_of_1200_bytes_keep_every_gesture_s_travel` |
| 5.58 | 7.08 | 5.33 | 2.86 | engine lib | `copy_operations::tests::moves::directories::recursive_move_crosses_native_enumeration_pages_without_losing_children` |
| 5.56 | 8.98 | 2.41 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::immediate_copy_into_the_same_folder_progresses_beside_a_busy_copy` |
| 5.40 | 9.03 | 2.71 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::the_busy_copy_publishes_into_output_that_a_waiting_copy_prepared` |
| 5.15 | 8.59 | 1.99 | — | engine lib | `copy_operations::tests::scheduler::immediate_start::one_disk_slot_interleaves_a_second_copy_with_a_busy_copy` |
| 4.93 | 6.85 | 5.36 | 2.25 | engine lib | `copy_namespace::tests::one_hundred_thousand_sibling_metadata_roots_use_indexes_and_shared_parent` |
| 4.83 | 5.68 | 5.71 | — | ui test integration | `busy_folder_native::a_folder_that_keeps_changing_is_relisted_at_a_bounded_rate_and_converges` |
| 4.66 | 4.23 | 2.51 (4 tests) | — | viewer-engine lib | `worker::tests::long_rows::rows_mixing_short_and_very_long_ones_keep_every_gesture_s_travel` |
| 4.38 | 6.88 | 6.43 | 1.54 | ui test integration | `mouse::actual_mouse_selection_and_activation_preserve_entry_identity` |
| 4.10 | 4.47 | 4.63 | — | ui test integration | `busy_folder_native::a_changed_file_updates_only_its_row_and_keeps_colors_and_folder_hints` |
| 3.85 | 3.84 | 3.84 | — | engine lib | `watch::tests::a_folder_without_notifications_is_checked_often_and_reported_only_when_it_lasts` |
| 3.80 | 4.16 | 1.68 | — | ui lib | `scroll_input::pane_tests::kinetic_tests::x11_fractional_line_stream_follows_the_finger_then_coasts` |
| 3.74 | 4.10 | 1.69 | — | ui lib | `scroll_input::pane_tests::kinetic_tests::phased_touchpad_flick_coasts_once_with_decaying_steps_then_settles` |
| 3.54 | 6.27 | 6.61 | 2.91 | engine lib | `operations::metadata_tests::traversal::recursive_edit_changes_every_entry_of_a_folder_with_1000_entries` |
| 3.44 | 5.51 | 4.95 | 1.89 | engine lib | `copy_operations::tests::moves::directories::recursive_move_depth_budget_retains_unfinished_tree` |
| 3.38 | 5.00 | 3.29 | — | ui test integration | `path_bar_fit::path_components_and_the_edit_area_stay_inside_the_bar_at_every_width` |
| 3.30 | 4.68 | 4.46 | 5.64 | engine test integration | `operations_real_windows::real_directory_corpus_matches_independent_sha256_for_both_policies` |
| 3.19 | 3.20 | 3.20 | — | engine test integration | `watch_native::a_failed_registration_is_checked_often_and_retried_without_a_status` |
| 3.15 | 6.12 | 5.70 | 2.59 | engine lib | `operations::metadata_tests::traversal::queued_metadata_jobs_holding_every_shared_target_cannot_limit_running_recursive_edits` |
| 3.05 | 5.15 | 4.73 | 4.08 | ui test integration | `native_operation_layout::native_operation_layout_preserves_long_unicode_content_and_action_bounds` |
| 2.66 | 4.06 | 3.89 | 2.40 | ui test integration | `filename_colors::default_palette_renders_every_category_with_contrast_in_every_theme_and_selection_state` |
| 2.54 | 3.73 | 3.66 | 2.08 | ui test integration | `drag_integration::folder_targets_follow_actual_scrolled_cells_in_details_brief_and_thumbnail_modes` |
| 2.50 | 3.04 | 3.08 | 1.08 | ui test integration | `operations_native_ui::real_folder_drag_uses_same_volume_move_ctrl_copy_and_escape_cancellation` |
| 2.49 | 3.66 | 3.61 | 2.87 | ui test integration | `themes::all_themes_are_selectable_and_keep_text_readable` |
| 2.42 | 3.47 | 3.45 | 2.67 | ui test integration | `tree_clipboard::copy_nested_folders_from_left_tree_and_paste_into_right_tree_root` |
| 2.31 | 3.34 | 3.23 | 2.95 | ui test integration | `tree_clipboard::copy_nested_folders_from_right_tree_and_paste_into_left_tree_root` |
| 2.21 | 3.35 | 3.24 | 2.47 | ui test integration | `tree_clipboard::cut_nested_folders_from_left_tree_and_confirm_native_move_into_right_tree` |
| 2.20 | 3.16 | 3.17 | 2.96 | ui test integration | `tree_clipboard::cut_nested_folders_from_right_tree_and_confirm_native_move_into_left_tree` |
| 2.16 | 3.30 | 3.12 | 1.23 | ui test integration | `native_operation_windows::embedded_copy_draft_shows_every_wrapped_source_completely` |
| 2.16 | 3.07 | 3.17 | 2.38 | ui test integration | `tooltips::shared_tooltips_keep_cursor_clear_in_panels_dropdowns_and_options` |
| 2.15 | 1.75 | 1.34 (2 tests) | — | viewer-engine lib | `worker::tests::overlap::page_keys_read_only_the_rows_past_the_shown_page` |
| 2.14 | 3.18 | 3.30 | 2.35 | ui test integration | `tree_filters::native_tree_controls_apply_clear_sort_and_cancel_across_layouts` |
| 2.09 | 3.13 | 3.21 | 2.01 | ui test integration | `native_controls::native_choices_keep_gestures_tooltips_and_full_unicode_working_paths` |
| 2.06 | 2.09 | 2.08 | — | engine lib | `engine_comparator_policy_tests::settings_return_after_shutdown_grace_and_runtime_exit_retires_on_cleanup_worker` |
| 2.03 | 2.29 | 2.26 | 2.34 | ui test integration | `tree_user_command_process::tree_user_command_actual_native_once_preserves_nested_unicode_path_and_retires_child` |
| 2.03 | 2.42 | 2.34 | 2.36 | ui test integration | `tree_user_command_process::tree_user_command_actual_native_per_item_preserves_unicode_selection_and_retires_children` |
| 2.00 | 3.00 | 3.01 | 2.97 | ui test integration | `tree_move::native_move_updates_both_trees_and_new_file_viewer_without_losing_marks` |
| 2.00 | 4.28 | 1.89 | 1.87 | engine lib | `operations::metadata_tests::traversal::recursive_edit_ignores_shared_targets_held_by_other_jobs` |
| 2.00 | 3.98 | 2.33 | 1.70 | engine lib | `operations::metadata_tests::traversal::windows::recursive_hidden_attribute_reaches_1000_entries` |
| 1.98 | 2.61 | 4.54 | 2.02 | engine lib | `copy_operations::preparation::planning_tests::planned_native_roots_and_discovery_release_each_real_source_before_the_next` |
| 1.93 | 1.40 | 1.47 | — | compare-ui test integration | `fonts::application_fonts_and_palette_updates_preserve_document_font_and_prime_later_opens` |
| 1.92 | 2.04 | 1.97 | — | compare-ui test integration | `find::find_keys_search_both_files_center_hits_and_clear_stale_results_without_a_close_button` |
| 1.91 | 1.53 | 1.53 | — | viewer-ui lib | `raster_frontier_tests::keys_and_wheel_keep_their_travel_across_corrected_surfaces_at_slow_reads` |
| 1.90 | 1.89 | 1.90 | — | engine lib | `watch::tests::a_folder_that_keeps_changing_is_hinted_at_most_once_per_cooldown` |
| 1.88 | 1.27 | 1.31 | 3.05 | compare-ui test integration | `fonts::comparator_offers_fixed_pitch_fonts_persists_explicit_choice_and_fits_small_window` |
| 1.87 | 1.50 | 1.51 | — | viewer-ui lib | `raster_frontier_tests::keys_and_wheel_keep_their_travel_at_slow_page_reads` |
| 1.85 | 1.92 | 2.18 | 1.04 | engine lib | `copy_operations::tests::moves::windows::move_of_the_64_root_limit_verifies_and_removes_every_pinned_file` |
| 1.85 | 2.57 | 2.34 | 2.43 | ui test integration | `tree_watch::external_root_and_expanded_branch_changes_preserve_left_tree` |
| 1.84 | 1.77 | 1.86 | 0.99 | platform-windows lib | `user_command_terminal_tests::custom_powershell_receives_native_environment_without_evaluating_filename_source` |
| 1.80 | 2.91 | 2.99 | 2.32 | ui test integration | `tree_move::native_copy_updates_destination_tree_and_new_file_viewer_without_losing_marks` |
| 1.79 | 1.42 | 1.42 | — | viewer-ui lib | `raster_frontier_tests::keys_and_wheel_keep_their_travel_across_a_corrected_surface` |
| 1.78 | 2.43 | 2.40 | 2.35 | ui test integration | `tree_watch::external_root_and_expanded_branch_changes_preserve_right_tree` |
| 1.77 | 2.79 | 2.69 | 1.62 | ui test integration | `dialog_layout::dialogs_keep_actions_visible_in_all_themes_and_densities` |
| 1.76 | 1.38 | 1.39 | — | viewer-ui lib | `raster_frontier_tests::keys_and_wheel_keep_their_travel_across_raster_and_page_frontiers` |
| 1.75 | 2.57 | 2.55 | 2.41 | ui test integration | `tree_move::f6_moves_nested_native_directories_from_right_panel` |
| 1.71 | 2.25 | 2.39 | 1.67 | ui test integration | `tree_checksums::tree_checksums_ctrl_f8_right_selected_nested_folder_and_root_file_recalculate` |
| 1.71 | 2.41 | 2.48 | 1.57 | engine lib | `checksum::worker::tests::completed_and_budget_limited_calculations_always_fit_the_manifest_verifier` |
| 1.70 | 2.72 | 2.57 | 2.60 | ui test integration | `tree_move::f6_moves_nested_native_directories_from_left_panel` |
| 1.67 | 2.41 | 2.34 | 1.51 | ui test integration | `panel_selection::selected_rows_keep_their_fill_when_focused_in_every_theme_and_view` |
| 1.60 | 2.39 | 2.20 | 2.06 | ui test integration | `tree_folder_sizes::explicit_size_sort_and_filter_preserve_calculated_descendant_values` |
| 1.57 | 2.53 | 2.38 | 1.56 | ui test integration | `command_line_layout::command_editor_and_controls_match_the_footer_across_themes_and_font_sizes` |
| 1.51 | 1.24 | 1.24 | — | viewer-ui lib | `raster_frontier_tests::a_page_restyled_around_the_lookahead_does_not_stop_a_burst` |
| 1.49 | 2.36 | 2.28 | — | ui test integration | `hover_feedback::hover_looks_keep_each_themes_contrast` |
| 1.44 | 1.97 | 1.94 | — | ui test integration | `comparator_transaction::touch_external_comparator_workflow_saves_raw_paths_and_cancel_rolls_back` |
| 1.44 | 1.90 | 1.93 | 1.66 | engine test integration | `checksum_manifest_verification::checksum::worker::tests::completed_and_budget_limited_calculations_always_fit_the_manifest_verifier` |
| 1.43 | 2.10 | 2.14 | 2.02 | ui test integration | `tree_move::top_level_move_removes_the_row_and_preserves_the_expanded_tree_session` |
| 1.41 | 1.72 | 1.59 | 1.01 | engine lib | `folder_sizes::native_tests::native_size_scans_skip_links_inside_the_tree_and_follow_only_a_selected_root_link` |
| 1.40 | 1.91 | 1.95 | 1.65 | engine test integration | `checksums_backend::checksum::worker::tests::completed_and_budget_limited_calculations_always_fit_the_manifest_verifier` |
| 1.39 | 1.89 | 2.02 | 1.81 | ui test integration | `tree_checksums::tree_checksums_ctrl_f8_left_native_file_fallback` |
| 1.39 | 1.07 | 0.78 | 0.55 | engine lib | `operations::metadata_tests::pins::a_subfolder_after_its_finished_siblings_sees_their_names_returned` |
| 1.38 | 2.06 | 1.95 | — | ui test integration | `busy_folder_native::a_large_burst_replaces_the_rows_and_still_converges_without_the_loading_state` |
| 1.37 | 1.73 | 1.92 | — | engine lib | `archive_sizes::tests::a_folder_of_more_than_100000_subfolders_is_sized_completely` |
| 1.37 | 1.97 | 1.86 | — | ui test integration | `busy_folder_native::a_file_that_jumps_to_the_top_moves_without_refilling_the_rows_it_passed` |
| 1.36 | 2.02 | 1.95 | — | ui test integration | `panel_fit_and_empty::an_empty_folder_says_so_below_parent_which_still_goes_up` |
| 1.34 | 2.14 | 2.06 | — | ui test integration | `busy_folder_native::small_change_bursts_update_rows_in_place_without_the_loading_state` |
| 1.34 | 1.72 | 1.78 | 1.43 | ui test integration | `tree_copy::archive_nested_folders_copy_and_deselect_through_f5` |
| 1.34 | 1.84 | 2.04 | 1.65 | ui test integration | `tree_copy::native_nested_folders_copy_and_deselect_through_f5` |
| 1.31 | 1.33 | 1.33 | 1.30 | engine test integration | `selected_folder_sizes::selected_status_reads_cached_totals_and_tracks_global_and_individual_scans` |
| 1.29 | 1.84 | 1.98 | 1.94 | ui test integration | `tree_user_commands::tree_user_command_right_selected_nested_folder_and_root_file_launch_in_display_order` |
| 1.29 | 1.41 | 1.33 | 0.81 | engine lib | `copy_operations::tests::throughput::throughput_timer_cannot_publish_terminal_before_native_resource_cleanup` |
| 1.28 | 2.03 | 1.73 | — | engine lib | `copy_operations::archive_source::size_tests::a_selection_of_more_than_4096_members_is_taken_whole` |
| 1.26 | 1.89 | 1.90 | 1.28 | ui test integration | `operations_native_ui::real_f5_multiselection_toolbar_copy_and_f6_move_preserve_bytes_and_clear_selection` |
| 1.26 | 1.68 | 1.79 | 1.50 | ui test integration | `tree_folder_sizes::selected_descendants_report_exact_bytes_without_measuring_unselected_parent` |
| 1.25 | 1.87 | 1.83 | — | ui test integration | `tree_copy::captured_native_copy_passes_unchecked_directory_links_to_preparation` |
| 1.24 | 1.80 | 2.06 | 1.86 | ui test integration | `tree_user_commands::tree_user_command_left_unmarked_nested_folder_once_uses_native_focus` |
| 1.24 | 1.67 | 1.92 | 1.57 | ui test integration | `tree_folder_sizes::native_bulk_measures_visible_folders_without_reordering_the_tree` |
| 1.23 | 1.78 | 1.85 | 1.50 | ui test integration | `tree_folder_sizes::archive_overlapping_selection_has_independent_ancestor_and_descendant_totals` |
| 1.22 | 1.53 | 1.60 | 1.33 | ui test integration | `tree_clipboard::unselected_parent_is_excluded_and_focused_nested_folder_is_the_fallback` |
| 1.22 | 1.65 | 1.85 | 1.48 | ui test integration | `tree_folder_sizes::space_measures_its_original_descendant_while_other_folders_are_marked` |
| 1.21 | 1.95 | 1.77 | — | ui test integration | `panel_fit_and_empty::a_narrow_panel_shows_short_names_whole_and_drops_time_and_date_first` |
| 1.17 | 1.80 | 1.79 | — | compare-engine lib | `text::align::tests::many_changes_between_anchors_align_in_bounded_time` |
| 1.17 | 1.17 | 1.17 | 0.32 | engine test integration | `watch_native::an_event_driven_refresh_lists_its_directory_once` |
| 1.16 | 1.17 | 1.17 | 0.33 | engine test integration | `watch_native::a_resort_of_rows_a_check_found_current_reads_nothing` |
| 1.16 | 1.17 | 1.17 | 0.33 | engine test integration | `watch_native::a_resort_without_the_ui_rows_lists_its_directory_once` |
| 1.15 | 1.53 | 1.63 | — | ui test integration | `operations_native_ui::windows_delete::locked_directory_delete_reports_an_error_and_succeeds_after_release` |
| 1.14 | 1.28 | 1.06 | 1.02 | engine lib | `copy_operations::tests::progress_discovery::full_outbox_retries_latest_discovery_without_another_callback` |
| 1.14 | 0.70 | 0.65 | 0.57 | engine lib | `operations::metadata_tests::pins::name_budget_spans_the_current_path_and_returns_each_name_when_it_is_taken` |
| 1.13 | 1.05 | 1.06 | — | engine lib | `watch::tests::hidden_tests::rows_are_checked_with_the_choice_they_were_listed_with` |
| 1.13 | 1.42 | 1.71 | 1.42 | ui test integration | `tree_prefix_search::rapid_typing_escape_and_full_queue_converge_without_reopening_folders` |
| 1.12 | 1.64 | 1.56 | 1.10 | ui test integration | `tree_command_insertion::native_tree_command_insertions_honor_persisted_remaps_without_default_fallbacks` |
| 1.11 | 1.67 | 1.53 | 0.95 | ui test integration | `tree_bulk_selection::native_left_clear_removes_nested_folder_marks` |
| 1.10 | 1.78 | 1.87 | 1.43 | ui test integration | `tree_filters::archive_tree_inactive_panel_filter_preserves_authority_and_last_valid_result` |
| 1.10 | 1.48 | 1.49 | 0.99 | ui test integration | `tree_bulk_selection::native_right_tree_shortcuts_distinguish_files_and_all_entries` |
| 1.09 | 1.65 | 1.55 | 1.38 | ui test integration | `tree_folder_sizes::archive_bulk_reads_metadata_and_keeps_all_three_panel_layouts` |
| 1.08 | 1.46 | 1.53 | 0.89 | ui test integration | `tree_bulk_selection::native_left_tree_shortcuts_distinguish_files_and_all_entries` |
| 1.08 | 1.71 | 1.51 | 0.86 | ui test integration | `tree_bulk_selection::native_right_clear_removes_nested_folder_marks` |
| 1.08 | 1.57 | 1.39 | 1.26 | ui test integration | `tree_folder_sizes::files_only_selection_never_falls_back_to_measuring_all_folders` |
| 1.08 | 1.60 | 1.73 | 1.57 | ui test integration | `tree_prefix_search::archive_quick_filter_preview_cancel_and_clear_keep_ancestor_authority` |
| 1.07 | 1.09 | 1.10 | 1.03 | engine lib | `copy_operations::tests::progress_verification::full_sha256_events_report_partial_readback_and_reset_for_the_next_file` |
| 1.07 | 1.44 | 1.48 | 1.17 | ui test integration | `user_command_settings_ui::editor_page_and_done_remain_accessible_in_narrow_layout_and_every_theme_density` |
| 1.06 | 1.79 | 1.21 | 1.03 | engine lib | `operation_throughput::tests::three_hours_keep_the_first_activity_and_each_series_peak_in_bounded_uniform_buckets` |
| 1.06 | 1.16 | 1.09 | 1.04 | engine lib | `copy_operations::tests::progress_verification::sampled_events_report_sample_bytes_and_reset_for_the_next_file` |
| 1.05 | 1.59 | 1.62 | 0.79 | ui test integration | `archive_badges::archive_vector_fallback_is_visible_and_associated_artwork_wins_in_every_view` |
| 1.05 | 1.57 | 1.55 | 1.01 | ui test integration | `copy_dialog::compact_move_controls_remain_reachable_with_large_touch_font` |
| 1.05 | 1.55 | 1.68 | 0.92 | ui test integration | `copy_dialog::compact_copy_controls_remain_reachable_with_large_touch_font` |
| 1.04 | 1.53 | 1.46 | 1.34 | engine lib | `copy_operations::tests::conflict_display::copy_conflict_prompt_formats_only_its_two_displayed_entries` |
| 1.02 | 1.13 | 1.03 | 0.17 | explorer bin me | `window_backend::native::tests::native_debug_layers_are_explicit_and_indirect_validation_is_retained` |
| 1.02 | 1.47 | 1.44 | 1.45 | ui test integration | `tree_prefix_search::insert_clears_search_then_marks_the_captured_match_and_advances` |
| 1.01 | 1.40 | 1.39 | — | engine test integration | `find_archives::every_archive_and_member_is_searched_however_many_there_are` |
| 1.00 | 1.35 | 1.32 | 1.24 | ui test integration | `tree_filters::real_branch_overtaking_filter_reports_failure_and_keeps_draft_retryable` |
| 1.00 | 0.35 | 0.19 | 0.81 | engine lib | `folder_sizes::batched::tests::fixtures::a_network_scan_yields_its_open_iterator_so_a_small_waiting_root_finishes_io_first` |
| 1.00 | 1.03 | 1.15 | — | compare-engine lib | `capture_tests::bytes::the_hex_view_of_a_text_pair_finishes_its_scan_after_progress_frames` |
| 0.99 | 1.62 | 1.46 | 1.11 | ui test integration | `panel_search_render::product_prefix_color_and_compression_badge_survive_themes_density_fonts_and_row_input` |
| 0.99 | 1.57 | 1.63 | 1.28 | ui test integration | `tree_prefix_search::unicode_typing_and_match_navigation_use_real_native_tree_publications` |
| 0.99 | 1.31 | 1.43 | 1.15 | ui test integration | `tree_filters::actual_engine_receipt_after_navigation_cannot_apply_to_replacement_pane` |
| 0.98 | 1.11 | 1.13 | 0.89 | engine lib | `copy_operations::tests::scheduler::real_target_conflict_keeps_opt_in_waiter_behind_decision_and_cancel_cleanup` |
| 0.97 | 1.51 | 1.41 | 1.16 | ui test integration | `tree_metadata::native_left_tree_case_uses_captured_descendant` |
| 0.96 | 1.40 | 1.40 | 0.75 | ui test integration | `native_operation_windows::native_operation_controls_remain_reachable_at_320_by_200_across_appearance_preferences` |
| 0.96 | 1.17 | 1.07 | — | ui lib | `scroll_input::pane_tests::real_file_panes_keep_fractional_wheel_positions_during_model_refresh` |
| 0.96 | 1.34 | 1.34 | 1.17 | ui test integration | `tree_viewers::native_tree_f3_and_basic_viewer_keep_exact_root_identity_in_all_layouts` |
| 0.96 | 1.40 | 1.52 | — | ui test integration | `action_icon_colors::action_glyphs_share_the_dimmed_theme_tint_and_ignore_native_artwork` |
| 0.96 | 1.37 | 1.37 | 1.11 | ui test integration | `tree_command_insertion::virtual_and_control_names_do_not_insert_or_execute_and_parent_keeps_name_only` |
| 0.95 | 1.43 | 1.54 | 1.13 | ui test integration | `tree_prefix_search::branch_collapse_rebuilds_matches_and_escape_preserves_collapsed_state` |
| 0.95 | 1.38 | 1.34 | — | ui test integration | `archive_path_bar::a_shortened_archive_path_keeps_the_archive_and_the_current_folder_whole` |
| 0.94 | 1.32 | 1.39 | 1.09 | ui test integration | `tree_editor::tree_editor_f4_routes_native_right_root_file_through_real_capture` |
| 0.94 | 1.42 | 1.02 | 0.82 | engine lib | `copy_operations::preparation::planning_tests::captured_native_roots_keep_deferred_owners_and_original_plan_on_retry` |
| 0.94 | 1.33 | 1.38 | 1.27 | ui test integration | `tree_rename::f7_native_expanded_tree_keeps_existing_create_directory_route` |
| 0.94 | 2.38 | 2.27 | — | office-preview test performance | `ten_thousand_entries_with_near_one_mib_names_open_and_render_real_cells` |
| 0.93 | 1.41 | 1.39 | 1.10 | ui test integration | `tree_editor::tree_editor_f4_routes_native_left_root_file_through_real_capture` |
| 0.93 | 0.89 | 0.88 | 0.67 | viewer-ui lib | `raster_render_tests::native_rasters_draw_one_pixel_checkerboards_exactly_at_fractional_canvas_origins` |
| 0.93 | 1.08 | 1.07 | 0.87 | engine lib | `copy_operations::tests::scheduler::acknowledged_pause_keeps_earlier_dependency_unfinished_until_real_cleanup` |
| 0.93 | 1.45 | 1.55 | 1.27 | ui test integration | `tree_rename::f2_native_right_expanded_descendant_routes_typed_capture` |
| 0.93 | 1.08 | 1.07 | 0.86 | engine lib | `copy_operations::tests::scheduler::failed_write_decision_and_checked_cleanup_both_precede_finished_dependency_release` |
| 0.92 | 1.48 | 1.32 | 0.81 | ui test integration | `tree_bulk_selection::archive_right_tree_shortcuts_distinguish_files_and_all_entries` |
| 0.92 | 1.36 | 1.29 | 0.88 | ui test integration | `tree_bulk_selection::archive_left_clear_removes_nested_folder_marks` |
| 0.92 | 1.50 | 1.45 | 1.13 | ui test integration | `tree_metadata::native_right_tree_case_uses_captured_descendant` |
| 0.92 | 1.25 | 1.34 | 0.83 | ui test integration | `tree_bulk_selection::archive_right_clear_removes_nested_folder_marks` |
| 0.91 | 1.47 | 1.34 | 1.26 | ui test integration | `tree_metadata::native_right_tree_attributes_use_captured_descendant` |
| 0.91 | 1.30 | 1.27 | 0.83 | ui test integration | `tree_bulk_selection::archive_left_tree_shortcuts_distinguish_files_and_all_entries` |
| 0.91 | 0.97 | 0.87 | 0.86 | engine lib | `drag_drop::internal::retirement_tests::accepted_drop_fill_then_cancel_keeps_waiting_successor_out_of_native_io` |
| 0.91 | 1.25 | 1.03 | 0.65 | engine lib | `operations::metadata_tests::metadata_preview_lists_every_bounded_root_and_refreshes_native_times` |
| 0.91 | 0.99 | 0.84 | 0.90 | engine lib | `drag_drop::internal::retirement_tests::accepted_drop_rejection_keeps_capacity_and_finished_until_actual_file_close` |
| 0.90 | 1.74 | 1.23 | 0.87 | platform-windows lib | `watch::batch_tests::batch_overflow_signals_every_target_and_keeps_watching` |
| 0.89 | 1.39 | 1.35 | 0.79 | ui test integration | `filename_colors::all_themes_states_and_views_render_readable_filename_colors_with_preserved_cues` |
| 0.89 | 0.82 | 1.10 | — | viewer-engine lib | `worker::tests::frontier::a_burst_over_a_page_restyled_for_its_lookahead_keeps_its_whole_travel` |
| 0.88 | 1.60 | 1.74 | 1.05 | platform-windows lib | `transfer::namespace::parent::tests::namespace_tests_pass_in_the_other_lookup_mode_too` |
| 0.88 | 1.40 | 1.64 | 1.27 | ui test integration | `tree_rename::f2_native_left_expanded_descendant_routes_typed_capture` |
| 0.88 | 1.51 | 1.34 | 1.16 | ui test integration | `tree_metadata::native_left_tree_attributes_use_captured_descendant` |
| 0.88 | 1.94 | 1.92 | 0.99 | platform-windows lib | `user_command_terminal_tests::cmd::custom_cmd_companions_round_trip_native_argv_and_raw_environment` |
| 0.88 | 1.34 | 1.33 | — | ui test integration | `comparator_transaction::compact_external_comparator_workflow_saves_raw_paths_and_cancel_rolls_back` |
| 0.88 | 1.20 | 1.26 | 0.87 | ui test integration | `tree_command_insertion::native_tree_name_and_full_path_append_focused_raw_text_from_files_and_command` |
| 0.87 | 1.30 | 1.29 | — | ui test integration | `options_caption_fit::options_captions_stay_whole_at_the_minimum_size_with_large_fonts` |
| 0.86 | 0.94 | 0.92 | — | compare-engine lib | `text::tests::large_text_reports_reading_progress_and_is_compared` |
| 0.86 | 1.88 | 0.82 | 0.79 | engine lib | `operations::metadata_tests::traversal::a_running_recursive_edit_leaves_the_shared_budget_to_previews` |
| 0.86 | 1.11 | 1.24 | 1.02 | ui test integration | `tree_viewers::archive_tree_f3_and_basic_viewer_keep_mount_identity_in_all_layouts` |
| 0.85 | 1.19 | 1.19 | 0.64 | ui test integration | `archive_options::archives_topic_has_individual_archive_and_document_checkboxes` |
| 0.85 | 0.97 | 0.91 | 0.57 | engine lib | `copy_operations::tests::scheduler::exact_two_slots_copy_same_folders_holds_two_native_reads_and_excludes_third` |
| 0.84 | 1.08 | 0.52 | 0.91 | ui lib | `scroll_input::pane_tests::file_panel_scrollbar_wheel_releases_explicit_cursor_reveal_and_keeps_panel_policy` |
| 0.83 | 1.05 | 1.01 | — | engine lib | `copy_operations::archive_source::size_tests::a_folder_beyond_the_former_copy_budgets_is_copied_completely` |
| 0.83 | 0.99 | 1.11 | 1.40 | engine test integration | `user_command_tree_preparation::tree_user_command_per_item_revalidates_only_its_immutable_native_scope` |
| 0.82 | 1.31 | 1.01 | 0.78 | explorer bin me | `thumbnails::pulse_tests::a_static_view_stops_polling_and_a_presentation_change_runs_one_pass` |
| 0.81 | 0.84 | 0.83 | 0.78 | engine lib | `copy_operations::tests::progress_verification::readback_retry_starts_a_fresh_pass_without_replaying_the_verified_prefix` |
| 0.81 | 0.85 | 0.81 | — | engine lib | `providers::archive::tests::a_folder_of_more_than_100000_members_lists_completely` |
| 0.81 | 1.19 | 1.22 | 0.80 | ui test integration | `tree_command_insertion::native_tree_ctrl_p_uses_the_pane_root_at_the_caret_or_selected_text` |
| 0.81 | 1.21 | 1.23 | 1.43 | platform-windows lib | `shell_menu::tests::shell_menu_real_shell_resolves_unicode_multiselection_without_invoking` |
| 0.80 | 1.09 | 1.03 | 0.63 | ui test integration | `operations_native_ui::real_inline_delete_uses_confirmation_and_reconciles_only_the_selected_branch` |
| 0.78 | 0.44 | 0.61 | — | compare-engine lib | `capture_tests::identity_lifecycle::retry_increments_comparison_generation_but_geometry_and_text_navigation_do_not` |
| 0.78 | 0.06 | 0.30 | — | compare-engine lib | `capture_tests::bytes::a_text_pair_shows_its_bytes_on_request_navigates_them_and_returns_to_text` |
| 0.78 | 0.85 | 0.95 | 0.59 | engine lib | `providers::folder_listing_tests::presence_probe_charges_inspected_tokens_to_the_token_ledger` |
| 0.78 | 1.21 | 1.20 | — | ui test integration | `filename_colors::inline_tree_pdf_results_repaint_visible_descendants_without_rebuilding_rows` |
| 0.77 | 1.92 | 1.89 | — | office-preview lib | `format_tests::early_text_and_escaped_html_windows_resume_at_first_omitted_block` |
| 0.77 | 1.39 | 0.62 | 0.37 | engine lib | `copy_operations::tests::verification::queued_path_selected_and_captured_copy_preserve_verification_choice` |
| 0.77 | 1.10 | 1.12 | — | ui test integration | `themes::marked_rows_keep_each_theme_hue_and_distinct_cursor_outline` |
| 0.76 | 0.84 | 0.88 | 0.59 | engine lib | `copy_operations::tests::scheduler::unfilled_earlier_reservation_blocks_opt_in_but_later_immediate_bypasses_it` |
| 0.76 | 1.16 | 0.99 | — | ui test integration | `operations_native_ui::windows_delete::skip_keeps_only_the_open_file_with_its_folders_and_deletes_the_rest` |
| 0.75 | 0.95 | 0.98 | — | ui test integration | `about_dialog::end_and_the_other_keys_keep_the_selected_library_whole_in_view` |
| 0.74 | 1.62 | 1.63 | — | office-preview test performance | `first_far_and_revisited_deflated_windows_replay_captured_bytes` |
| 0.73 | 1.25 | 1.31 | 0.87 | platform-windows lib | `transfer::namespace::tests::native_file_stage_lifecycles_announce_exact_original_arcs_with_linear_root_refreshes` |
| 0.72 | 0.45 | 0.40 | — | compare-engine lib | `capture_tests::identity_lifecycle::change_time_drift_during_the_initial_text_read_reads_the_fresh_capture` |
| 0.71 | 1.07 | 1.08 | 0.59 | ui test integration | `keyboard_options::typing_shortcut_search_survives_scrolled_selection_and_empty_results` |
| 0.71 | 1.14 | 1.21 | 0.95 | ui test integration | `tree_move::f6_refuses_an_archive_tree_without_starting_a_transfer` |
| 0.71 | 1.19 | 1.22 | 0.72 | platform-windows lib | `transfer::namespace::tests::one_live_job_retires_each_native_stage_alias_and_its_index_before_the_next_attempt` |
| 0.69 | 0.90 | 0.90 | 0.63 | ui test integration | `small_window::exact_320_by_200_minimum_survives_density_fonts_and_recovers_both_panels` |
| 0.69 | 0.99 | 1.01 | 0.63 | ui test integration | `operations_native_ui::real_delete_confirmation_cancel_and_shift_f8_permanently_delete_only_the_selected_tree` |
| 0.69 | 0.77 | 0.90 | — | engine lib | `copy_operations::archive_source::size_tests::stop_ends_resolving_a_large_selection` |
| 0.69 | 0.98 | 0.99 | — | ui test integration | `operations_native_ui::real_copy_without_following_creates_an_empty_directory` |
| 0.68 | 0.77 | 0.82 | 0.56 | engine lib | `copy_operations::tests::scheduler::exact_two_slots_move_same_folders_holds_two_committed_moves_and_excludes_third` |
| 0.67 | 1.03 | 0.94 | 0.66 | ui test integration | `comparison_toolbar::comparison_buttons_follow_the_active_panel_even_when_panels_are_swapped` |
| 0.67 | 0.96 | 1.08 | 0.86 | platform-windows lib | `transfer::namespace::tests::published_stages_do_not_accumulate_private_claims_after_actual_owner_close` |
| 0.66 | 0.94 | 0.97 | 0.59 | engine lib | `copy_operations::tests::scheduler::exact_two_slots_mixed_same_folders_preserves_proof_and_move_removal` |
| 0.66 | 0.95 | 0.96 | 0.54 | ui test integration | `options_reflow::options_navigation_grows_with_its_labels_and_keeps_selected_captions_unclipped` |
| 0.66 | 1.06 | 1.08 | — | compare-ui test integration | `ui_polish::image_mode_buttons_select_and_preserve_acknowledged_composition_until_ready` |
| 0.66 | 1.01 | 0.99 | — | ui test integration | `operations_native_ui::real_copy_follows_a_junction_by_default` |
| 0.65 | 0.85 | 0.92 | — | engine lib | `copy_operations::tests::scheduler::one_slot_reports_disk_capacity_until_cancelled_predecessor_actual_stage_cleanup_returns` |
| 0.65 | 1.00 | 0.41 | — | compare-engine lib | `capture_tests::identity_lifecycle::initial_frame_precedes_whole_input_work_and_late_proof_cannot_revive_a_replaced_pair` |
| 0.65 | 0.44 | 0.41 | — | compare-engine lib | `capture_tests::identity_lifecycle::change_time_drift_during_whole_input_io_with_unchanged_text_restarts_verification` |
| 0.65 | 0.59 | 0.63 | 0.51 | viewer-ui lib | `text_presentation_tests::source_opening_keeps_first_glyph_geometry_when_its_toolbar_appears` |
| 0.65 | 0.57 | 0.79 | 0.79 | engine lib | `operations::tests::throughput::delete_speed_settles_to_zero_after_reported_work_then_a_native_stall` |
| 0.64 | 0.96 | 0.94 | — | compare-ui test integration | `ui_polish::identity_strip_colors_only_positive_verdicts_and_keeps_byte_scope_accessible` |
| 0.63 | 0.30 | 0.31 | — | compare-engine lib | `capture_tests::identity_lifecycle::source_change_during_whole_input_io_replaces_pending_frame_with_an_error` |
| 0.62 | 0.76 | 0.79 | 0.40 | ui test integration | `archive_options::wheel_reveals_last_archive_checkbox_after_resizing_options` |
| 0.62 | 0.61 | 0.61 | 0.62 | elevation lib | `native::tests::pending_reads_and_writes_honor_cancellation_and_the_deadline` |
| 0.61 | 0.90 | 0.78 | 0.60 | explorer bin me | `thumbnails::delivery_tests::zoom_releases_hidden_pane_pressure_and_delivers_rejected_visible_frame_without_input` |
| 0.61 | 0.72 | 0.64 | 0.62 | platform-windows lib | `watch::tests::native_unicode_directory_reports_mutations_and_releases_callback_on_drop` |
| 0.60 | 0.89 | 0.94 | 0.59 | ui test integration | `commander_edges::footer_mouse_release_keeps_the_pressed_action_and_logical_pane` |
| 0.60 | 0.17 | 0.14 | 1.37 | worker-platform test containment | `native_worker_denies_private_file_and_network_access_but_keeps_broker_pipes` |
| 0.60 | 1.03 | 0.99 | 0.78 | platform-windows lib | `transfer::namespace::tests::published_private_aliases_reannounce_exact_arcs_without_probing_unrelated_history` |
| 0.59 | 0.84 | 0.80 | 0.92 | engine test integration | `operations_real_windows::native::real_permanent_delete_cancels_at_first_reported_node_then_resumes_owned_tree` |
| 0.59 | 0.83 | 0.80 | — | ui test integration | `tool_window_polish::the_compare_tolerance_field_selects_with_theme_colours_in_every_theme` |
| 0.59 | 1.03 | 1.12 | 0.64 | engine lib | `tree_actor::tests::reclaim_tests::watched_temporary_churn_keeps_reference_tables_bounded_and_live_rows_intact` |
| 0.59 | 1.23 | 1.19 | 0.98 | platform-common test integration | `archive_association_preferences::every_known_choice_is_bounded_validated_and_independent` |
| 0.58 | 0.87 | 0.81 | 0.52 | ui test integration | `parent_focus::parent_routes_focus_child_and_scroll_it_into_view_without_checking_it` |
| 0.57 | 0.82 | 0.81 | — | ui test integration | `operations_native_ui::real_failed_f2_rename_reports_the_error_without_holding_input` |
| 0.57 | 0.84 | 0.88 | 0.55 | ui test integration | `operations_native_ui::real_copy_conflict_shows_both_entries_in_the_native_locale` |
| 0.55 | 0.07 | 0.54 | 0.05 | compare-engine lib | `capture_tests::a_window_with_a_full_queue_neither_delays_nor_loses_a_catalog_reply` |
| 0.55 | 0.82 | 0.65 | — | compare-ui test integration | `ui_polish::identical_text_result_is_green_and_cannot_survive_changed_inputs_or_retry` |
| 0.55 | 0.49 | 0.45 | — | compare-engine lib | `capture_tests::identity_lifecycle::a_repaint_after_change_time_drift_keeps_unchanged_text_and_reports_changed_text` |
| 0.55 | 0.83 | 0.83 | 0.54 | ui test integration | `mutation_controls::rename_delete_create_dialogs_preserve_confirmation_and_editor_focus` |
| 0.55 | 0.56 | 0.57 | 0.55 | engine test integration | `find_actor::sorted_views_keep_discovery_validation_authority_and_latest_revision` |
| 0.55 | 0.91 | 0.84 | 0.78 | engine lib | `copy_operations::tests::throughput::blocked_copy_emits_zero_activity_without_releasing_native_capacity` |
| 0.55 | 0.54 | 0.55 | 0.54 | engine lib | `watch::tests::fence_tests::a_refresh_that_interrupts_a_check_is_not_listed_again` |
| 0.55 | 0.49 | 0.49 | — | viewer-ui lib | `web_wheel_tests::html_wheel_notches_match_the_text_viewer_at_twice_the_scale` |
| 0.54 | 0.61 | 0.56 | — | engine lib | `operations::tests::cancelled_delete_preserves_partial_progress_and_only_genuine_errors` |
| 0.54 | 0.90 | 0.85 | 0.48 | ui test integration | `file_actions_ui::copy_conflict_native_choices_cancel_and_later_keep_exact_job_identity` |
| 0.54 | 0.81 | 0.77 | 0.82 | engine test integration | `operations_real_windows::real_same_volume_directory_move_preserves_identity_and_reads_zero_content` |
| 0.54 | 0.55 | 0.58 | — | engine test integration | `find_archives::folders_waiting_in_an_archive_count_against_retained_memory_until_listed` |
| 0.53 | 0.78 | 0.77 | 0.77 | engine lib | `copy_operations::tests::progress_discovery::preparing_discovery_publishes_successive_counts_and_paths_without_throughput` |
| 0.53 | 0.90 | 0.48 | 0.34 | engine lib | `drag_drop::tests::captured_nested_drop_reports_exact_tree_root_completion_for_copy_and_move` |
| 0.53 | 0.70 | 0.70 | 0.59 | engine test integration | `find_duplicate_failures::simulated_short_native_reads_do_not_truncate_hash_or_byte_comparison` |
| 0.51 | 0.73 | 0.68 | 0.47 | ui test integration | `panel_view_modes::independent_modes_preserve_file_identity_and_virtualize_column_and_icon_layouts` |
| 0.51 | 0.79 | 0.70 | 0.45 | ui test integration | `selection_rectangle_ui::logical_gutter_and_grid_math_survive_density_font_and_window_changes` |
| 0.51 | 0.61 | 0.64 | 0.47 | compare-ui test integration | `difference_navigation::real_keys_jump_both_sides_without_wrapping_losing_repeats_or_bypassing_overrides` |
| 0.51 | 0.07 | 0.27 | — | compare-engine lib | `capture_tests::bytes::a_binary_pair_is_compared_as_bytes_with_a_summary_and_a_byte_verdict` |
| 0.50 | 0.06 | 0.27 | — | compare-engine lib | `capture_tests::bytes::identical_binary_files_get_their_verdict_from_one_byte_scan` |
