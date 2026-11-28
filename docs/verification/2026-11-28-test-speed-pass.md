# Test speed pass — 2026-11-28

## Scope

Every Windows test of the root workspace's library, binary and integration test
executables (70 executables) and every build-script test of `scripts/tests` (599 tests in
46 modules) was timed at `08ba0847`. Two changes were made: the build-script tests now
run a module per process, several at once, on Windows (CI and validation), and
`midnight-ui`'s `caption_glyphs` reads its sources on several threads. No test, assertion
or hang guard was removed or weakened. The plugins under `plugins/` are separate
workspaces and were not timed, nor were the PowerShell `scripts/tests/test-*.ps1` checks
or the image decoder's native helper tests. The [27 November pass](2026-11-27-test-speed-pass.md)
and the [25 November pass](2026-11-25-test-speed-pass.md) had looked at the Rust tests;
the build-script tests had not been timed before.

## Method

1. **Build.** `cargo test --workspace --no-run --message-format=json-render-diagnostics
   --locked --offline -j 6`, debug test profile, debug information off,
   `CARGO_INCREMENTAL=0`, in a fresh worktree with its own target directory (13 min 26 s,
   70 test executables). Each executable was copied aside under a new name in the same
   folder before any change; the changed build rebuilt only `midnight-ui`'s integration
   executable.
2. **Rust runs.** As on 27 November ([Method](2026-11-27-test-speed-pass.md#method)): each
   executable alone, one after another, from its package directory, libtest's default
   thread count (20), JSON output with `-Z unstable-options --report-time`
   (`RUSTC_BOOTSTRAP=1` for the test process only), a private temporary directory, a
   private `HOST_TEST_INSTALL_ROOT`, `MIDNIGHT_UI_RENDER_DIR` unset; each executable listed
   its tests twice and its ignored tests once before its timed run. A test's time is
   libtest's `exec_time` beside the other tests of its executable. *Before* is the median
   of three rounds of the copies (`survey`), *After* of three rounds of the final build
   (`after`).
3. **Script tests.** Per-test times from a `unittest` result class that stamps each test's
   start and stop, running the same discovery as CI (`discover -s scripts/tests` from the
   repository root), three runs; the A/B compares CI's former command
   (`python -B -m unittest discover -s scripts/tests -v`) with the new runner at 8 and at
   4 modules at a time (4: the CPUs of a hosted Windows runner), three alternating rounds.
4. **Cold source scans.** To time `caption_glyphs` in the state that made it slow, every
   file it reads (1,749 `.rs` and `.slint` files) was rewritten with its own bytes and its
   times restored before each run (Cargo and Git see no change; a real-time scanner sees a
   written file). Then the test ran once from that state and once again (warm), with the
   baseline copy and the changed build in alternating order.
5. **Load.** Before each round the machine's busy CPU share over three seconds was taken,
   and after each executable compiler and linker processes were counted.

The scripts are this pass's own, not repository tools.

## Environment

Windows 11 Enterprise 10.0.26200, i7-13800H (20 logical CPUs), 31.6 GB, NTFS on a data
volume, Microsoft Defender real-time protection on; Rust 1.99.1, Python 3.15.0,
PowerShell 7.7.0. A shared machine: other work took 23–36 % of the CPUs before survey
rounds 1 and 3 and kept compilers running beside 52 and 40 of their 70 executables; round
2 and the `after` rounds (5–20 % before them) saw no compiler. A warm `pwsh -NoProfile`
start took 0.26–0.33 s, `cmd /c` 0.015–0.07 s.

## Results

### Rust executables

Executables over 0.5 s; *rounds* are the three single values.

| Executable | Before s | Before rounds | After s | After rounds | Passed/failed/ignored |
| --- | ---: | --- | ---: | --- | --- |
| `midnight-ui::test:integration` | 25.12 | 25.12, 22.89, 50.20 | 24.23 | 17.91, 24.86, 24.23 | 1270/0/11 |
| `midnight-viewer-engine::lib:midnight_viewer_engine` | 15.74 | 15.74, 14.97, 16.58 | 14.88 | 14.53, 14.92, 14.88 | 673/0/20 |
| `midnight-engine::lib:midnight_engine` | 14.34 | 14.34, 14.08, 14.54 | 13.09 | 12.88, 13.09, 15.00 | 2070/0/34 |
| `midnight-engine::test:integration` | 5.58 | 5.30, 5.58, 5.77 | 5.84 | 5.26, 6.57, 5.84 | 438/0/17 |
| `midnight-compare-ui::lib:midnight_compare_ui` | 4.99 | 5.06, 4.88, 4.99 | 5.01 | 5.01, 4.91, 5.09 | 53/0/2 |
| `midnight-ui::lib:midnight_ui` | 4.32 | 4.44, 3.91, 4.32 | 4.97 | 4.06, 4.97, 6.27 | 1759/0/14 |
| `midnight-platform-windows::lib:midnight_platform_windows` | 3.56 | 3.56, 2.51, 11.48 | 3.04 | 3.46, 3.04, 2.64 | 470/0/30 |
| `midnight-explorer::bin:midnight-explorer` | 2.61 | 2.61, 2.74, 2.60 | 2.66 | 2.62, 2.76, 2.66 | 140/0/28 |
| `midnight-viewer-ui::lib:midnight_viewer_ui` | 2.37 | 2.37, 2.24, 2.45 | 2.40 | 2.21, 2.59, 2.40 | 262/0/0 |
| `midnight-worker-platform::lib:midnight_worker_platform` | 2.06 | 2.06, 2.07, 2.05 | 2.05 | 2.05, 2.06, 2.03 | 45/0/1 |
| `midnight-elevation::lib:midnight_elevation` | 2.04 | 2.03, 2.07, 2.04 | 2.07 | 2.07, 2.06, 2.07 | 41/0/0 |
| `midnight-compare-ui::test:integration` | 1.95 | 1.93, 1.95, 2.00 | 1.98 | 1.98, 2.05, 1.97 | 19/0/0 |
| `midnight-compare-engine::lib:midnight_compare_engine` | 1.76 | 1.78, 1.56, 1.76 | 1.72 | 1.72, 1.83, 1.67 | 191/0/4 |
| `midnight-worker-platform::test:containment` | 1.36 | 1.36, 0.16, 1.41 | 0.17 | 0.79, 0.17, 0.17 | 1/0/0 |
| `midnight-viewer-ui::test:integration` | 1.27 | 1.27, 1.19, 1.34 | 1.21 | 1.23, 1.20, 1.21 | 36/0/0 |
| `midnight-viewer-render::lib:midnight_viewer_render` | 1.10 | 1.10, 1.02, 4.68 | 1.04 | 1.06, 1.04, 1.03 | 46/0/0 |
| `midnight-filesystem-host::lib:midnight_filesystem_host` | 1.06 | 1.06, 1.06, 1.04 | 1.06 | 1.06, 1.06, 1.05 | 135/0/1 |
| `midnight-platform-common::lib:midnight_platform_common` | 1.05 | 1.03, 1.19, 1.05 | 1.28 | 0.97, 1.28, 1.30 | 208/0/0 |
| `midnight-platform-common::test:integration` | 0.94 | 0.94, 1.10, 0.91 | 1.14 | 0.79, 1.24, 1.14 | 186/0/0 |
| `midnight-update::test:integration` | 0.89 | 0.89, 0.84, 0.89 | 0.86 | 0.86, 0.85, 0.86 | 83/0/0 |
| `midnight-office-preview::test:performance` | 0.81 | 0.80, 0.95, 0.81 | 0.85 | 0.81, 0.85, 0.91 | 3/0/1 |
| `midnight-thumbnail-host::lib:midnight_thumbnail_host` | 0.73 | 0.73, 0.73, 0.74 | 0.73 | 0.74, 0.73, 0.73 | 51/0/5 |
| `midnight-office-preview::lib:midnight_office_preview` | 0.64 | 0.64, 0.75, 0.63 | 0.66 | 0.62, 0.66, 0.80 | 86/0/0 |
| `midnight-core::test:integration` | 0.55 | 0.55, 0.59, 0.53 | 0.54 | 0.54, 0.54, 0.56 | 170/0/4 |
| 46 other executables, together | 4.13 | | 4.33 | | |
| **All 70** | **101.0** | | **97.8** | | |

`midnight-explorer`'s `windows_batch` took 18.2 s in the first round (its first start of
the application executable after the build, as on 27 November) and 0.31–0.32 s later.
Every executable but one is byte-identical in both columns; their differences are the
machine's. `midnight-ui`'s integration executable ended with `caption_glyphs` in survey
rounds 1 and 3 (23.4 and 48.7 s of its 25.1 and 50.2 s); see
[Changes](#caption_glyphs-midnight-ui-integration).

### The 50 slowest Rust tests

Ranked by *Before* (median of the three survey rounds).

| # | Before s | Before rounds | After s | Executable | Test |
| ---: | ---: | --- | ---: | --- | --- |
| 1 | 23.36 | 23.36, 1.73, 48.66 | 0.67 | ui test integration | `caption_glyphs::every_shown_character_has_a_glyph_in_the_application_font` |
| 2 | 15.12 | 15.12, 14.37, 16.11 | 14.24 | viewer-engine lib | `worker::tests::long_rows::rows_of_8000_bytes_fill_the_window_at_rest::source` |
| 3 | 15.07 | 15.07, 14.32, 15.86 | 14.14 | viewer-engine lib | `worker::tests::long_rows::rows_of_8000_bytes_fill_the_window_at_rest::text` |
| 4 | 13.17 | 13.18, 12.39, 13.17 | 12.23 | viewer-engine lib | `worker::tests::long_rows::rows_of_8000_bytes_keep_every_gesture_s_travel::text` |
| 5 | 13.10 | 13.10, 12.56, 13.12 | 12.43 | viewer-engine lib | `worker::tests::long_rows::rows_of_8000_bytes_keep_every_gesture_s_travel::source` |
| 6 | 10.24 | 8.84, 10.24, 10.48 | 9.36 | engine lib | `copy_operations::input::root_bound_tests::a_panel_selection_of_more_native_roots_is_refused_before_any_root_is_prepared` |
| 7 | 7.83 | 7.83, 8.97, 6.55 | 8.93 | ui test integration | `native_operation_layout::native_operation_layout_preserves_long_unicode_content_and_action_bounds` |
| 8 | 7.56 | 7.56, 6.97, 7.89 | 6.22 | engine lib | `copy_operations::tests::streamed::a_streamed_move_across_volumes_removes_each_source_only_after_its_verified_copy` |
| 9 | 6.98 | 6.98, 7.43, 6.53 | 7.54 | viewer-engine lib | `worker::tests::long_rows::rows_of_2400_bytes_keep_every_gesture_s_travel::source` |
| 10 | 6.88 | 6.88, 7.51, 6.57 | 7.52 | viewer-engine lib | `worker::tests::long_rows::rows_of_2400_bytes_keep_every_gesture_s_travel::text` |
| 11 | 6.52 | 5.36, 6.89, 6.52 | 5.81 | engine lib | `checksum::manifest::tests::exports_of_any_size_are_encoded_and_read_back` |
| 12 | 6.13 | 6.13, 7.08, 5.70 | 7.24 | viewer-engine lib | `image_worker::document_band_tests::a_fast_drag_publishes_the_cameras_it_passes` |
| 13 | 6.04 | 6.04, 7.12, 5.11 | 7.01 | ui test integration | `mouse::actual_mouse_selection_and_activation_preserve_entry_identity` |
| 14 | 5.87 | 5.78, 6.00, 5.87 | 6.07 | ui test integration | `busy_folder_native::a_refresh_that_changes_nothing_visible_leaves_the_model_and_every_row_untouched` |
| 15 | 5.84 | 5.24, 6.10, 5.84 | 5.20 | engine lib | `checksum::format::tests::a_manifest_of_any_size_parses` |
| 16 | 5.75 | 5.75, 4.35, 6.28 | 4.77 | engine lib | `copy_operations::tests::archive::a_cancelled_selection_reaches_a_waiting_ui_after_a_few_events` |
| 17 | 5.59 | 6.02, 5.06, 5.59 | 4.69 | engine lib | `copy_operations::tests::streamed::a_vanished_item_fails_alone_and_retry_takes_it_again` |
| 18 | 5.43 | 5.43, 2.65, 5.83 | 3.47 | engine lib | `copy_operations::tests::streamed::a_streamed_selection_keeps_about_a_hundred_bytes_per_item_beside_its_paths` |
| 19 | 5.08 | 5.08, 5.65, 4.53 | 5.57 | viewer-engine lib | `worker::tests::long_rows::rows_of_8000_bytes_take_scrollbar_positions_after_page_down::text` |
| 20 | 5.07 | 5.07, 5.65, 4.57 | 5.58 | viewer-engine lib | `worker::tests::long_rows::rows_of_8000_bytes_take_scrollbar_positions_after_page_down::source` |
| 21 | 4.94 | 4.94, 6.08, 4.58 | 6.49 | ui test integration | `filename_colors::default_palette_renders_every_category_with_contrast_in_every_theme_and_selection_state` |
| 22 | 4.89 | 4.89, 5.41, 4.06 | 5.43 | ui test integration | `themes::all_themes_are_selectable_and_keep_text_readable` |
| 23 | 4.86 | 4.86, 5.26, 4.44 | 5.33 | viewer-engine lib | `worker::tests::long_rows::rows_of_1200_bytes_keep_every_gesture_s_travel::source` |
| 24 | 4.84 | 4.93, 4.74, 4.84 | 4.87 | compare-ui lib | `scroll_parity_tests::touchpad_motion_and_its_momentum_move_the_text_as_in_the_text_viewer` |
| 25 | 4.78 | 5.88, 4.62, 4.78 | 4.49 | engine lib | `clipboard::paste_capture_tests::paste_refuses_more_native_roots_than_one_copy_takes_as_it_reads_them` |
| 26 | 4.70 | 4.70, 5.26, 4.37 | 5.22 | viewer-engine lib | `worker::tests::long_rows::rows_of_1200_bytes_keep_every_gesture_s_travel::text` |
| 27 | 4.64 | 4.64, 4.65, 4.40 | 5.12 | ui test integration | `busy_folder_native::a_folder_that_keeps_changing_is_relisted_at_a_bounded_rate_and_converges` |
| 28 | 4.54 | 4.54, 5.07, 4.41 | 5.11 | viewer-engine lib | `worker::tests::long_rows::rows_of_2400_bytes_fill_the_window_at_rest::source` |
| 29 | 4.53 | 4.45, 5.01, 4.53 | 5.18 | viewer-engine lib | `worker::tests::long_rows::rows_of_2400_bytes_fill_the_window_at_rest::text` |
| 30 | 4.45 | 4.47, 4.45, 4.39 | 4.51 | compare-ui lib | `scroll_parity_tests::the_scrolling_glide_and_speed_settings_apply_as_in_the_text_viewer::glide_very_long` |
| 31 | 4.25 | 4.32, 3.84, 4.25 | 3.35 | engine lib | `copy_operations::tests::streamed::a_streamed_copy_takes_each_item_and_totals_them_item_by_item` |
| 32 | 4.24 | 4.69, 3.70, 4.24 | 3.09 | engine lib | `copy_operations::tests::streamed::an_item_replaced_after_its_identity_was_read_fails_alone_and_is_never_copied` |
| 33 | 4.18 | 3.76, 4.55, 4.18 | 4.39 | engine test integration | `checksums_backend::checksum::manifest::tests::exports_of_any_size_are_encoded_and_read_back` |
| 34 | 4.14 | 4.14, 4.23, 3.97 | 4.39 | ui test integration | `busy_folder_native::a_changed_file_updates_only_its_row_and_keeps_colors_and_folder_hints` |
| 35 | 4.12 | 4.24, 4.12, 3.91 | 4.02 | viewer-engine lib | `worker::tests::row_groups::a_refill_shapes_only_the_rows_it_adds` |
| 36 | 3.96 | 4.10, 3.89, 3.96 | 4.07 | compare-ui lib | `scroll_parity_tests::scroll_keys_and_horizontal_scrolling_move_the_text_as_in_the_text_viewer::horizontal_notch` |
| 37 | 3.96 | 3.38, 4.25, 3.96 | 4.12 | engine test integration | `checksums_backend::checksum::format::tests::a_manifest_of_any_size_parses` |
| 38 | 3.89 | 3.43, 3.98, 3.89 | 3.72 | engine test integration | `operations_real_windows::real_directory_corpus_matches_independent_sha256_for_both_policies` |
| 39 | 3.87 | 3.84, 3.87, 3.91 | 4.10 | engine lib | `copy_operations::tests::archive::a_copy_reading_the_folder_an_archive_copy_stages_in_waits_for_the_whole_copy` |
| 40 | 3.82 | 3.82, 3.21, 3.88 | 3.20 | engine lib | `copy_operations::tests::moves::directories::recursive_move_crosses_native_enumeration_pages_without_losing_children` |
| 41 | 3.64 | 3.64, 2.87, 3.89 | 3.67 | engine lib | `copy_operations::tests::archive::a_paused_archive_copy_keeps_its_staging_folder_and_resumes_in_it` |
| 42 | 3.61 | 3.35, 3.61, 4.04 | 3.29 | engine lib | `operations::metadata_tests::traversal::recursive_edit_changes_every_entry_of_a_folder_with_1000_entries` |
| 43 | 3.58 | 3.58, 4.32, 3.44 | 4.43 | viewer-engine lib | `worker::tests::frontier::a_notch_up_after_a_scrollbar_position_needs_no_corrected_surface` |
| 44 | 3.51 | 3.52, 3.51, 2.77 | 3.28 | ui test integration | `tree_clipboard::cut_nested_folders_from_left_tree_and_confirm_native_move_into_right_tree` |
| 45 | 3.40 | 3.40, 3.69, 2.90 | 3.64 | ui test integration | `panel_selection::selected_rows_keep_their_fill_when_focused_in_every_theme_and_view` |
| 46 | 3.40 | 3.50, 3.40, 3.33 | 3.46 | compare-ui lib | `scroll_parity_tests::scroll_keys_and_horizontal_scrolling_move_the_text_as_in_the_text_viewer::arrow_right_x2` |
| 47 | 3.35 | 3.35, 4.23, 3.25 | 4.49 | ui test integration | `dialog_layout::dialogs_keep_actions_visible_in_all_themes_and_densities` |
| 48 | 3.26 | 3.26, 2.50, 3.31 | 2.23 | engine lib | `copy_operations::tests::streamed::a_streamed_move_moves_each_item_and_keeps_no_item_prepared_ahead` |
| 49 | 3.23 | 3.23, 2.54, 3.54 | 2.24 | engine lib | `copy_operations::tests::streamed::a_streamed_move_takes_a_folder_selected_twice_once` |
| 50 | 3.18 | 3.18, 3.18, 3.17 | 3.19 | engine test integration | `watch_native::a_failed_registration_is_checked_often_and_retried_without_a_status` |

266 tests took over 1 s before (median), 269 after. Apart from `caption_glyphs`, every
test above is unchanged and byte-identical in both columns.

### Build-script tests

CI's former command, `python -B -m unittest discover -s scripts/tests -v`, took 211–240 s
for 605 tests (the 599 of the baseline and the runner's own six); 172 are skipped on
Windows (Linux-only tests). Its modules, summed per-test time (mean of two runs):
`test_build_hooks` 47.6 s, `test_validation_payload` 30.1 s, `test_html_viewer_build`
25.2 s, `test_acceptance_windows` 24.5 s, `test_validate` 22.7 s, `test_default_thumbnailers`
11.1 s, `test_targeted_test_runner` 6.3 s, every other module under 5 s. Nearly all of it
is waiting for child processes: `build.bat` starts PowerShell for about ten steps per run,
and most other tests start PowerShell once per case.

The 30 slowest tests (two runs, the third of the baseline lacked one module):

| # | s (two runs) | Test |
| ---: | --- | --- |
| 1 | 9.30, 9.04 | `test_build_hooks.BuildHookTests.test_validation_runs_after_every_build_step_and_before_deployment` |
| 2 | 7.04, 7.68 | `test_validate.WindowsValidationTests.test_invalid_arguments_stop_before_any_build_tool` |
| 3 | 6.37, 6.91 | `test_build_hooks.BuildHookTests.test_arguments_from_another_directory_still_build_the_repository` |
| 4 | 5.97, 6.92 | `test_validate.WindowsValidationTests.test_results_skips_strictness_and_the_private_installation` |
| 5 | 6.26, 6.06 | `test_validation_payload.ValidationPayloadTests.test_array_wrapped_objects_cannot_bypass_receipt_or_snapshot_structure` |
| 6 | 6.20, 5.14 | `test_build_hooks.BuildHookTests.test_validation_failure_prevents_deployment_and_is_returned` |
| 7 | 5.70, 5.37 | `test_build_hooks.BuildHookTests.test_explorer_build_names_the_helper_it_ships` |
| 8 | 5.07, 5.36 | `test_html_viewer_build.HtmlViewerBuildWindows.test_short_target_builds_from_the_verified_sdk_and_unchanged_inputs_keep_the_package` |
| 9 | 5.14, 4.83 | `test_acceptance_windows.AcceptanceRunnerTests.test_invalid_arguments_stop_with_usage` |
| 10 | 4.93, 4.90 | `test_acceptance_windows.AcceptanceRunnerTests.test_a_run_that_requires_a_lease_runs_only_under_it` |
| 11 | 3.22, 3.63 | `test_default_thumbnailers.WindowsThumbnailerSwapTests.test_hold_beyond_two_seconds_fails_and_restores_the_previous_package` |
| 12 | 3.17, 3.40 | `test_html_viewer_build.HtmlViewerBuildWindows.test_standalone_distribution_is_flattened_once_and_packaged_against_its_receipt` |
| 13 | 3.10, 3.18 | `test_acceptance_windows.AcceptanceRunnerTests.test_packages_newer_than_the_checkout_stop_the_run` |
| 14 | 2.91, 3.02 | `test_validation_payload.ValidationPayloadTests.test_stale_malformed_or_incomplete_live_snapshot_is_rejected` |
| 15 | 2.86, 2.80 | `test_html_viewer_build.HtmlViewerBuildWindows.test_packaged_cef_files_must_match_the_sdk_receipt` |
| 16 | 2.93, 2.64 | `test_build_hooks.BuildHookTests.test_success_builds_application_then_viewers_for_same_profile` |
| 17 | 2.66, 2.89 | `test_build_hooks.BuildHookTests.test_default_build_includes_debug_thumbnailers` |
| 18 | 2.70, 2.74 | `test_build_hooks.BuildHookTests.test_deployment_failure_is_returned_after_app_and_viewers` |
| 19 | 2.27, 2.87 | `test_acceptance_windows.AcceptanceRunnerTests.test_the_next_run_removes_installations_whose_run_has_ended` |
| 20 | 2.28, 2.75 | `test_validate.WindowsValidationTests.test_ci_steps_skipped_for_a_prerequisite_are_named_before_the_result` |
| 21 | 2.35, 2.54 | `test_build_hooks.BuildHookTests.test_build_failure_skips_validation` |
| 22 | 2.43, 2.41 | `test_build_hooks.BuildHookTests.test_archive_failure_prevents_deployment_and_is_returned` |
| 23 | 2.28, 2.10 | `test_html_viewer_build.HtmlViewerBuildWindows.test_brief_hold_of_the_new_package_is_waited_out` |
| 24 | 2.14, 2.07 | `test_viewer_build_profiles.ViewerBuildNoticesWindows.test_notices_are_replaced_only_when_generated_or_below_build` |
| 25 | 2.05, 2.10 | `test_prepare_html_cef.PrepareHtmlCef.test_hold_beyond_two_seconds_fails_with_the_rename_error_and_keeps_the_previous_sdk` |
| 26 | 1.99, 2.09 | `test_image_viewer_build.ImageViewerWorkerSdkWindows.test_a_fresh_checkout_packages_the_sdk_once_and_caches_it_beside_the_viewer_build` |
| 27 | 2.00, 2.06 | `test_default_thumbnailers.WindowsThumbnailerSwapTests.test_rebuild_swaps_in_a_fresh_package_and_keeps_a_mapped_library_intact` |
| 28 | 1.84, 2.17 | `test_validation_payload.ValidationPayloadTests.test_failed_incomplete_or_nonterminal_build_is_rejected` |
| 29 | 2.08, 1.92 | `test_build_hooks.BuildHookTests.test_thumbnail_failure_prevents_deployment_and_is_returned` |
| 30 | 1.78, 2.19 | `test_validate.WindowsValidationTests.test_a_powershell_session_gets_its_environment_back` |

## Changes

| Change | Before | After |
| --- | --- | --- |
| Build-script tests on Windows: a module per process, up to 8 at a time (`56338975`) | 240.4, 215.8, 211.2 s (`unittest discover`) | 57.1, 60.8, 58.3 s (8 at a time); 77.1, 78.4, 80.1 s (4 at a time) |
| `caption_glyphs`: sources read on 16 threads (`f56a2d8d`) | cold 37.1 s (32.5–47.0, six runs), warm 0.48 s | cold 7.3 s (5.7–8.4, six runs), warm 0.45 s |
| `midnight-ui` integration executable from a cold scan state | 45.8, 41.8, 41.3 s | 17.4, 19.0, 18.9 s |

### Build-script test runner (`scripts/run-script-tests.py`)

The new runner runs each `test*.py` module of `scripts/tests` as `python -B -m unittest
discover -s scripts/tests -p <module> -v`, from the repository root, in a process of its
own, at most `--jobs` modules at a time (default: the CPUs, at most 8), the largest files
first. The tests of one module still run one after another in one process, as they were
written; modules already ran in one shared process before, so no module gains an
isolation it relied on. Each module's output is printed whole when the module ends, then
a summary; the exit status is 1 when a module fails or runs no test. A test package below
the folder is refused rather than left out (discovery would enter it; there is none).
Each module's own `Ran N tests` and `OK`/`FAILED` lines stay together, so validation's
counter (`Get-TestCounts`) sums them as before.

Windows CI (`scripts-and-lint`) and `scripts/validate.ps1` use it; Linux CI and
`scripts/validate.sh` keep `unittest discover`, since this pass did not run Linux. Its six
tests (`test_run_script_tests.py`) check the counts, a failing and an empty module, a
refused package, `--jobs 0`, and that two modules run at once (each waits for the other's
marker). The runs took 57–61 s at 8 modules at a time, bounded by `test_build_hooks`
(50–57 s alone in a process); at 4, as on a hosted runner, 77–80 s.

### `caption_glyphs` (`midnight-ui` integration)

`every_shown_character_has_a_glyph_in_the_application_font` scans the string literals of
1,749 source files (141 `.slint`, 1,608 `.rs`, about 19.5 MB) and then software-renders
the 17 non-ASCII characters it finds. Warm, the whole test takes 0.5–1.7 s. In survey
rounds 1 and 3 it took 23.4 and 48.7 s, the tail of its executable; `common_edits`, the
only other test that reads many sources (141 `.slint` files), took 1.3–1.5 s instead of
0.15 s in the same rounds; no other test differed so between rounds. Reading fresh copies
of the 1,608 `.rs` files from Python took 35–44 s one after another (0.4–0.6 s when read
again), and 6.4–9.0 s (8 threads), 6.6 s (16) and 4.9 s (32) when read by several threads:
the first read of a written file waits for the real-time scan, about 25 ms a file, and
reads at once are scanned at once.

The test now lists all files first and reads each one once, by 16 threads that take files
from a shared index, then scans them in the former order, so every character keeps the
same first three places. A temporary check in the changed test built the map the former
way too and compared: equal (17 characters, 42 places). The test-only files' `#[path]`
includes are collected over all folders instead of per folder; an include path is joined
to its own file's folder either way, so the set of skipped files is the same. Cold runs
(the files rewritten before each, alternating builds, six rounds): baseline 47.0, 37.0,
33.8, 43.5, 37.3, 32.5 s; changed 7.7, 7.0, 8.4, 5.7, 7.7, 7.0 s. A first variant with 8
threads over contiguous chunks per folder took 9.2–11.3 s (five rounds; baseline 28.9–48.3 s).

## Measured, not changed

- **Viewer engine long rows** (`long_rows`, 12–15 s, the library's tail), **root-bound
  paths** (9–10 s), **checksum any-size manifests**, **streamed copies**,
  **`native_operation_layout`**, **every-theme render tests**: as on
  [27 November](2026-11-27-test-speed-pass.md#looked-at-not-changed); unchanged since.
- **Fixed waits of production timing**: `midnight-elevation`'s
  `a_cancelled_request_keeps_the_session_only_when_its_reply_arrives_soon` (2.0 s, the
  library's tail since the 27 November record: it asserts that a cancelled request gives
  up no earlier than `CANCEL_GRACE`), `midnight-worker-platform`'s
  `browser::native::tests::waits_after_a_termination_share_one_bound` (2.0 s, asserts the
  `TREE_EXIT` bound), the user-command stop grace, the comparator settings grace and the
  real-engine watch windows. A shorter wait needs a production seam and tests another
  number.
- **Cold first starts**: `windows_batch` 18.2 s once (first start of the application
  executable); `midnight-platform-windows`'s terminal tests 7.5–10.5 s each in survey
  round 3 (0.3–2.5 s otherwise; `cmd` and PowerShell children under others' builds);
  `midnight-worker-platform`'s `containment` 1.4 s in two rounds (0.16 s otherwise).
- **Build-script tests one by one**: each starts the scripts it tests through
  `cmd`/`build.bat` and PowerShell, which is what they test; a module's cases were not
  merged into fewer PowerShell sessions (`-File` and an in-session call differ in exit
  codes and error streams). Every hang guard (`HANG_GUARD = 180` s in `test_build_hooks`,
  for example) was left as it is. The critical path at 8 modules at a time is
  `test_build_hooks` alone; splitting a module across processes would run its cases side
  by side, which they were not written for.
- **The other executables of `midnight-ui`** and the large libraries: their walls are
  their summed work on 20 threads, as on 27 November.

## Validation

- Builds: the baseline and both changed builds (and a temporary build with the
  equivalence check) built every workspace test executable without an error.
- Rust: every executable passed in every survey and `after` round (9,537 tests passed in
  each), except one `after` round in which the unchanged
  `office_preview::tests::office_prepares_a_workbook_whose_change_time_alone_moved_after_capture`
  failed once ("missing ZIP end record"), as once on 27 November.
- `caption_glyphs`, changed: 30 runs alone, all passed (0.44–0.67 s); 12 runs in the cold
  A/B (six cold, six warm), all passed; three runs of the whole executable from a cold
  scan state, in which the changed test passed and two unchanged tests,
  `tree_user_command_process::tree_user_command_actual_native_*`, failed in both builds
  because that harness's environment had no `rustc` on `PATH` (their probe asks
  `rustc -vV`); in the survey environment they passed. Three `after` rounds of the whole
  executable: all passed.
- Build-script tests through the runner: 30 runs at 8 modules at a time while builds and
  other measurements of this pass loaded the machine, all passed (605 tests, 172 skipped,
  64.5–143.4 s); three more at 8 and three at 4, all passed; three runs of the former
  command, all passed (605 tests, 172 skipped). The runner's own tests, `test_validate`
  and `test_ci_workflow` passed after the change.
- `cargo fmt --all -- --check`: clean. `cargo clippy --workspace --all-targets --locked
  --features midnight-acceptance/acceptance -- -D warnings` (with `--offline -j 6`): no
  warning. `pwsh scripts/check-source-size.ps1`: passed.

## Limitations

- Windows only. Linux CI keeps `unittest discover` for the build-script tests; the
  runner and `caption_glyphs` were not run on Linux (the test change is platform-neutral).
- The CI step was not run on a hosted runner; the 4-at-a-time runs stand in for its four
  CPUs. Whether hosted Windows runners scan freshly checked-out files as this machine does
  was not measured.
- The machine was shared: survey rounds 1 and 3 met others' builds (see
  [Environment](#environment)); compare the A/B pairs, not the totals.
- The cold scan state was made by rewriting files; the slow survey rounds came from the
  fresh checkout (round 1) and from an unknown cause about ten minutes later (round 3).
