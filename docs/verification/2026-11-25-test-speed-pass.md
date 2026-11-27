# Test speed pass — 2026-11-25

## Scope

Every Windows test of the root workspace's library, binary and integration test
executables was timed at `e21ff7a7` (69 test executables); the tests added or slowed since
the [21 November pass](2026-11-21-test-speed-pass.md) (`cd2f821a`, recorded in `ddc95448`)
were looked at, and two were made faster where that loses nothing they prove. The plugins
under `plugins/` are separate workspaces and were not timed. 282 commits lie between
`cd2f821a` and `e21ff7a7`; 233 of them touch test files.

## Method

As on 21 November ([Method](2026-11-21-test-speed-pass.md#method)), with these differences:

1. **Builds.** `cargo test --workspace --no-run --message-format=json-render-diagnostics
   --locked --offline -j 6`, debug test profile, debug information off
   (`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`), `CARGO_INCREMENTAL=0`, in a
   fresh worktree of its own with its own target directory: the baseline at `e21ff7a7`
   (9 min 40 s, 69 test executables, copied aside before any change), and the final build
   at `580ca5ab` (the same command; it rebuilt the three changed test executables).
2. **Runs.** Each executable alone, one after another, from its package directory, with
   libtest's default thread count (20 here), JSON output, `-Z unstable-options
   --report-time` (`RUSTC_BOOTSTRAP=1` for the test process only), a private temporary
   directory below the worktree's `_build`, `MIDNIGHT_UI_RENDER_DIR` unset, and the build's
   `deps` folder and the toolchain's runtime folder on `PATH`. Before each timed run the
   executable listed its tests twice and its ignored tests once. A test's time is libtest's
   `exec_time`, measured beside the other tests of its executable.
3. **Private installation root.** Unlike 21 November, `HOST_TEST_INSTALL_ROOT` pointed at
   a private folder in the user profile, made as [test execution](../developer/test-execution.md#private-plugin-installation-root)
   describes. The `midnight-plugin-host` and `midnight-image-viewer-host` library tests that
   failed in every run on 21 November therefore pass and are timed here (134 and 38 tests,
   0.37 s and 0.14 s).
4. **Rounds.** *Survey*: three rounds of all 69 baseline executables (each round all
   executables, then the next round). *A/B*: the baseline copies and the final build,
   alternating executable sets, three rounds of all 69 (`ab-final`), plus focused A/B
   runs of the two changed executables (three rounds each, and five more of
   `midnight-ui`'s integration executable). Times are medians.
5. **Load.** A sampler counted compiler and linker processes (`rustc`, `cargo`, `link`,
   `clippy-driver`) every five seconds: 2 of 117 samples of the survey and none of 145 of
   `ab-final` saw one; the other samples with compilers were this pass's own builds,
   between the timed runs.

The scripts are this pass's own, not repository tools.

## Environment

Windows 11 Enterprise 10.0.26200, i7-13800H (20 logical CPUs), 31.6 GB, NTFS on a data
volume, Microsoft Defender real-time protection on (signatures 1.463.457.0); Rust 1.99.1.
A shared machine, quiet during the timed runs (see Load), whose speed drifted within the
session: the same `midnight-ui` integration executable took 15.3 to 21.3 s in consecutive
rounds of the same hour.

## Results

### First runs

The survey's first round ran each executable for the first time after the build. Three
were then much slower than in the later rounds, all from cold system state rather than
test code:

- `midnight-ui` integration 57.8 s (16.2 and 19.3 s later): `caption_glyphs::every_shown_character_has_a_glyph_in_the_application_font`
  took 56.7 s (1.05 and 1.41 s later). It renders each non-ASCII character of the UI
  sources through the system's font fallback; the first run loads the fallback fonts.
- `midnight-platform-windows` library 11.0 s (2.5 s later): the six
  `user_command_terminal_tests` (`cmd` and PowerShell children) took 7.3 to 10.0 s each,
  0.07 to 1.1 s later.
- `midnight-viewer-render` library 4.6 s (1.0 s later): `fonts::cache_tests::the_preferred_family_check_loads_one_face_per_installed_candidate`
  4.62 s, 0.25 s later.

The medians below are therefore of warm runs. CI runs every executable once, cold.

### Executables

*Survey* is the median of the three survey rounds. *Before* and *After* are `ab-final`:
the baseline copies and the final build in three alternating rounds (04:44–04:59). *21 Nov
after* is that record's *After* column.

| Executable | Survey s | Before s | After s | Passed/failed/ignored after | 21 Nov after s |
| --- | ---: | ---: | ---: | --- | ---: |
| `midnight-ui::test:integration` | 19.32 | 16.20 | 19.29 | 1241/0/10 | 21.18 |
| `midnight-engine::lib:midnight_engine` | 13.33 | 13.34 | 13.79 | 2038/0/31 | 15.42 |
| `midnight-viewer-engine::lib:midnight_viewer_engine` | 9.48 | 9.68 | 10.16 | 658/0/20 | 4.63 |
| `midnight-compare-ui::lib:midnight_compare_ui` | 8.21 | 8.24 | 4.93 | 51/0/2 | 1.23 |
| `midnight-engine::test:integration` | 4.47 | 4.48 | 4.50 | 417/0/12 | 5.16 |
| `midnight-ui::lib:midnight_ui` | 4.08 | 3.96 | 4.20 | 1719/0/14 | 2.80 |
| `midnight-platform-windows::lib:midnight_platform_windows` | 2.45 | 2.52 | 2.03 | 439/0/26 | 3.01 |
| `midnight-explorer::bin:midnight-explorer` | 2.37 | 2.34 | 2.32 | 138/0/27 | 1.22 |
| `midnight-viewer-ui::lib:midnight_viewer_ui` | 2.12 | 2.14 | 2.12 | 245/0/0 | 1.80 |
| `midnight-compare-ui::test:integration` | 1.95 | 1.94 | 1.92 | 19/0/0 | 2.09 |
| `midnight-compare-engine::lib:midnight_compare_engine` | 1.54 | 1.57 | 1.54 | 179/0/4 | 2.51 |
| `midnight-filesystem-host::lib:midnight_filesystem_host` | 1.06 | 1.06 | 1.05 | 135/0/1 | 0.74 |
| `midnight-viewer-render::lib:midnight_viewer_render` | 1.03 | 1.01 | 1.02 | 46/0/0 | 0.18 |
| `midnight-update::test:integration` | 0.86 | 0.85 | 0.86 | 66/0/0 | — |
| `midnight-office-preview::test:performance` | 0.81 | 0.80 | 0.80 | 3/0/1 | 2.30 |
| `midnight-viewer-ui::test:integration` | 0.77 | 0.75 | 0.75 | 36/0/0 | 0.45 |
| `midnight-thumbnail-host::lib:midnight_thumbnail_host` | 0.73 | 0.73 | 0.73 | 51/0/5 | — |
| `midnight-platform-common::test:integration` | 0.69 | 0.72 | 0.69 | 185/0/0 | 1.54 |
| `midnight-elevation::lib:midnight_elevation` | 0.64 | 0.63 | 0.64 | 25/0/0 | 0.65 |
| `midnight-office-preview::lib:midnight_office_preview` | 0.63 | 0.63 | 0.63 | 86/0/0 | 1.93 |
| `midnight-platform-common::lib:midnight_platform_common` | 0.53 | 0.50 | 0.50 | 193/0/0 | 1.07 |
| `midnight-core::test:integration` | 0.53 | 0.53 | 0.56 | 166/0/4 | — |
| 47 other executables, together | 4.11 | 4.11 | 4.14 | | |
| **All 69** | **81.7** | **78.7** | **79.2** | | **75.1** (67) |

`ab-final`'s totals do not show the changes: the unchanged executables, byte-identical
builds of the same sources, ran 2–5 % slower in their *After* half than in their *Before*
half (the engine library 13.34 → 13.79 s, the viewer engine's library 9.68 → 10.16 s,
`midnight-ui`'s library 3.96 → 4.20 s), and `midnight-ui`'s integration executable varied
by 4.5 s between rounds of either build. The changed executables:

- `midnight-compare-ui` library: 8.24 → 4.93 s (`ab-final`); 8.31 → 4.98 s in the focused
  A/B. Since 21 November it grew from 1.23 s by the scroll parity tests (`59b841e6`).
- `midnight-ui` integration: 19.44 → 18.92 s (three alternating rounds) and 20.47 → 20.19 s
  (five more); the changed test's 8 s of one thread's work is about 2 % of the
  executable's summed test time (350–370 s on 20 threads), below its round-to-round range.

Since 21 November, `midnight-viewer-engine`'s library grew from 4.63 to 9.7 s and
`midnight-compare-ui`'s from 1.23 to 8.2 s; the engine library (15.42 → 13.3 s) and
`midnight-ui`'s integration executable (21.18 → 16–20 s) did not grow. 69 test executables are built now, 67 on 21 November.

### Tests over 1 s

The survey found 201 tests over 1 s (median of three rounds; 21 November: 130). By their
state on 21 November: 120 are in that record's appendix (over 0.5 s then), 8 existed but
were under 0.5 s then, and 73 are new. The [appendix](#appendix-tests-over-1-s) lists every
test over 1 s in the survey, `ab-final`'s before or after. The largest new or slower ones,
with the commit that added or slowed them:

| Survey s | 21 Nov s | Test | Since 21 Nov |
| ---: | ---: | --- | --- |
| 10.12 | < 0.5 | engine lib `copy_operations::input::root_bound_tests::a_panel_selection_of_more_native_roots_is_refused_before_any_root_is_prepared` | bound raised to 1,000,000 items (`3e960b59`) |
| 8.70, 8.72 | 3.77 (slowest of 4) | viewer-engine lib `worker::tests::long_rows::rows_of_8000_bytes_keep_every_gesture_s_travel::{text,source}` | slower worker paging since `cd2f821a`; window lowered in `ea81b5d2` |
| 8.07, 7.82, 4.71, 3.83 | — | compare-ui lib `scroll_parity_tests::*` (four checks) | new (`59b841e6`); changed here |
| 7.68 | — | ui test integration `panel_toolbar_fit::pickers_turn_compact_only_when_the_drive_buttons_need_their_room` | new (`24f4c481`); changed here |
| 6.08, 5.15 | — | engine lib `checksum::manifest::tests::exports_of_any_size_are_encoded_and_read_back`, `checksum::format::tests::a_manifest_of_any_size_parses` (and 3.2, 3.0 s again in the engine integration executable) | new (`d0f65c49`) |
| 5.72, 4.07, 3.24, 2.99 | — | engine lib `copy_operations::tests::streamed::*` | new (`3e960b59`) |
| 5.57, 4.11, 3.73 | — | engine lib `copy_operations::tests::archive::*` | new (`7b77d714` and later) |
| 4.88 | — | viewer-engine lib `image_worker::document_band_tests::a_fast_drag_publishes_the_cameras_it_passes` | new (`aac18026`); shortened in `ea81b5d2` |
| 4.86 | — | engine lib `clipboard::paste_capture_tests::paste_refuses_more_native_roots_than_one_copy_takes_as_it_reads_them` | new (`3e960b59`) |
| 3.29, 2.06 | — | viewer-engine lib `worker::tests::row_groups::*` | new (`8a2b87c6`) |
| 3.17 | — | engine lib `user_command_output::tests::stop_ends_the_tree_and_kills_it_when_asked_politely_in_vain` | new (`b8042591`) |
| 3.15 | — | viewer-engine lib `worker::tests::frontier::a_notch_up_after_a_scrollbar_position_needs_no_corrected_surface` | new (`bcd61454`) |
| 2.83, 2.66 | 1.68, 1.69 | ui lib `scroll_input::pane_tests::kinetic_tests::*` (two) | slower; beside eight new `reveal_tests` and `rows_tests` (1.7–2.7 s, `49a8e9c9` and later) |
| 2.33–2.08 | < 0.5 | platform-windows lib `icons::tests::*`, `icons::sized_tests::*` (eight) | each waits for the others' native calls (`native_qualification`); the library's wall 3.01 → 2.45 s |

## Changes

| Change | Before | After |
| --- | --- | --- |
| `midnight-compare-ui` scroll parity: each schedule of the three multi-schedule checks is a test of its own on a harness of its own (`1a9be580`) | four checks 3.83–8.19 s; executable 8.31 s | 11 schedule tests 0.43–4.38 s, the touchpad check 4.81 s (unchanged); executable 4.98 s |
| `midnight-ui` panel toolbar fit: the sweep lays the window out instead of software-rendering it (`580ca5ab`) | 8.90 s (4.39 s alone) | 1.13 s (0.63 s alone) |

Times are medians of the focused A/B runs (three alternating rounds of the executable).

### Scroll parity (`scroll_parity_tests.rs`)

Each moving frame of a schedule waits in real time until the comparison window has been
quiet for 30 ms (`Harness::settle`: a worker that keeps up with every frame), so the
schedules are sleep-bound, and the checks ran three or four of them one after another on
one harness. `check` now runs one schedule on a harness of its own, and a local macro makes
each schedule of `wheel_notches_move_the_text_as_far_and_as_smoothly_as_in_the_text_viewer`,
`the_scrolling_glide_and_speed_settings_apply_as_in_the_text_viewer` and
`scroll_keys_and_horizontal_scrolling_move_the_text_as_in_the_text_viewer` a test
(`check::schedule`); the touchpad check keeps its one schedule. Every schedule keeps its
steps, settings, reset and `assert_parity`; `run` resets to the top left before each
schedule, so a fresh harness starts where the shared one did, and the glide and wheel speed
settings are thread-local. A filter by a former test name still selects all of its
schedules. Ten further runs of `scroll_parity`: all 14 tests passed each time, 4.85–4.95 s.
With `--test-threads=4`, as on 4-CPU hosted runners: 7.9–8.7 s in three runs, all passed
(before: 8.2 s on 20 threads with four tests). The 30 ms quiet window stays: the window has
no signal for "the worker has answered every input", and a shorter window would end a
frame before a slow worker's answer on a loaded runner.

### Panel toolbar fit (`panel_toolbar_fit.rs`)

`pickers_turn_compact_only_when_the_drive_buttons_need_their_room` measured 530 toolbar
rows (six densities and fonts, five widths, five drive counts, each also at 4,000 px)
and software-rendered the main window, up to 4,000 × 700 pixels, for each. Every check
reads item geometry. The sweep now resizes with `TestUi::set_size` and lays the window out
with `TestUi::layout` (timers, change handlers and lazily instantiated items, as a frame
does), as `path_bar_fit` and the dialog sweeps do. Evidence of equivalence: with a
temporary switch that rendered as before and a dump of every measured row, both modes
measured byte-identical geometry for all 530 rows (the same SHA-256 of both dumps).
`compact_pickers_keep_their_names_menus_and_keys` still renders a frame after its first
row, before its clicks. `panel_toolbar_fit` five times: 12 tests passed each time.

## Obsolete code and tests

Looked for, nothing removed:

- New `allow(dead_code)` since `cd2f821a` outside the opt-in acceptance crate: on test
  support files shared by several test modules (`midnight-ui` `alignment.rs`,
  `metadata_form.rs`, `minimum_layout.rs`, `modeless.rs`, `progress_form.rs`;
  `midnight-engine` `qa_fixture.rs`; `midnight-update` `support.rs`) and on a memory
  mirror in `midnight-core`'s checksum verification test, as their comments say; and in
  production code on items only tests call: `midnight-ui` `Presenter::request_archive_password`
  (its comment: no menu entry or shortcut yet, by design) and `midnight-plugin-host`
  `trust_linux::check_private` and `shared_error` (production calls them on Windows only).
  None is dead test code.
- A test of a removed feature: `job_progress::the_progress_window_has_no_details_button_and_always_shows_the_current_file`
  checks that the removed Details button stays gone (`621d9468`); it is current.
- Duplicates: the checksum library tests that the engine integration executable runs again
  (`checksums_backend::checksum::*`, including the two new any-size tests at 3.2 and 3.0 s)
  are the documented `#[path]` includes of [test execution](../developer/test-execution.md).
- No full dead-code probe (removing every `allow(dead_code)` and building) was made in
  this pass.

## Not changed

- **Bounds whose size is what the test proves.** The two root-bound tests refuse
  1,000,001 items through the real selection and clipboard paths (`MAX_NATIVE_TRANSFER_ROOTS`,
  raised from a small bound to 1,000,000 by `3e960b59`); the panel-selection test takes
  3.2 s alone (building 1,000,001 entries 0.54 s, their index 0.78 s, the selection
  0.21 s, the refused resolve 1.60 s, measured with temporary timers) and 10 s beside the
  library's 2,038 other tests, where it is the engine library's tail. A smaller bound needs
  a production seam and would test another number. The any-size checksum tests need more
  than the former 16 MiB of manifest text. The streamed copies need more than
  `MAX_PREPARED_NATIVE_ROOTS` (64) items (193 real files); the archive notice test needs
  more than `MAX_ROOTS_PER_NOTICE` (512) copied roots (600).
- **Viewer engine travel and paging** (`long_rows`, `row_groups`, `frontier`, 2–9 s):
  co-simulations on a virtual clock whose time is the worker's own page layout and paint
  in a debug build; a gesture sequence carries its state, so it cannot be split further
  without testing other states. `ea81b5d2` measured the same and lowered the windows.
- **Fixed production timing**: the user-command stop test waits for the three-second
  stop grace (`STOP_GRACE`); the watch and comparator-policy tests as on 21 November.
- **`midnight-ui` library scroll tests** (`reveal_tests`, `rows_tests`, `kinetic_tests`,
  1.5–2.8 s): already layout-only (`harness_entries(false, …)`); their time is frames of
  the presenter's refresh of two 1,000-row panels on a virtual clock.
- **Windows icon tests** (eight, 2.1–2.3 s each): they hold `native_qualification` in turn
  because cold parallel stock-icon calls can fail; the time is their serialized native
  work, and the executable takes 2.5 s.
- **`caption_glyphs`**: 1.0–1.4 s warm; its 57 s first run is the system's font
  fallback loading, not the test's own work.

## Validation

- Builds: the baseline and the final build each built every workspace test executable
  without an error.
- Tests: in all three survey rounds and every `ab-final` round, every executable passed
  (9,227 tests passed and 216 ignored in each round of the final build; 9,219 and 216 in
  the baseline, the difference being 11 schedule tests replacing three checks), except one
  survey round:
  `midnight-viewer-engine`'s `office_preview::tests::links::office_real_generated_links_resolve_from_bootstrap_catalog_and_nested_sheets`
  failed once in the second survey round ("Source changed since capture": the fixture's
  change time moved between its capture and its open, with its write time and length
  unchanged) and passed in the other eight runs of that executable. Not investigated
  further; it is unrelated to this pass's changes.
- `cargo clippy -p midnight-compare-ui --all-targets --locked --offline -j 6 -- -D warnings`
  and `cargo clippy -p midnight-ui --all-targets --locked --offline -j 6 -- -D warnings`:
  no warning, after the last edit of each.
- `rustfmt --edition 2024 --check` of both changed files: clean.
- No red run of a production fault was made: both changes keep every assertion, and the
  geometry comparison above shows the toolbar sweep measures exactly what it measured.

## Limitations

- One machine whose speed drifted during the session: the unchanged executables ran 2–5 %
  slower in `ab-final`'s *After* half, which hides the changed executables' gains in the
  workspace total. Compare the changed executables' alternating pairs, not the totals.
- Windows debug builds only. Linux was neither timed nor built; both changes are
  platform-neutral test code.
- Tests timed with a private installation root, unlike 21 November: the plugin and image
  viewer host library tests pass here and failed there.
- The survey's first round is of executables started for the first time after the build;
  the medians are of warm runs, CI's runs are cold.
- CI's hosted runners have fewer CPUs: splitting the parity schedules shortens the
  executable only where CPUs are free; with four test threads it took as long as before.

## Appendix: tests over 1 s

Every test over 1 s in the survey (median of three rounds), `ab-final`'s before or after
(medians of three rounds). *21 Nov s* is that record's *After* where the test is in its
appendix (for a test split into views there, its slowest view); *Since 21 Nov*: *on record*
(in that appendix), *existed* (its function existed at `cd2f821a`, under 0.5 s then) or
*new*. Split tests appear under their new names with no *Before*, the former tests with no
*After*.

| Survey s | Before s | After s | 21 Nov s | Since 21 Nov | Executable | Test |
| ---: | ---: | ---: | ---: | --- | --- | --- |
| 10.12 | 10.26 | 10.63 | — | existed | engine lib | `copy_operations::input::root_bound_tests::a_panel_selection_of_more_native_roots_is_refused_before_any_root_is_prepared` |
| 8.70 | 8.89 | 9.39 | 3.77 | on record | viewer-engine lib | `worker::tests::long_rows::rows_of_8000_bytes_keep_every_gesture_s_travel::text` |
| 8.72 | 9.09 | 9.33 | 3.77 | on record | viewer-engine lib | `worker::tests::long_rows::rows_of_8000_bytes_keep_every_gesture_s_travel::source` |
| 8.07 | 8.12 | — | — | new | compare-ui lib | `scroll_parity_tests::scroll_keys_and_horizontal_scrolling_move_the_text_as_in_the_text_viewer` |
| 7.82 | 7.78 | — | — | new | compare-ui lib | `scroll_parity_tests::the_scrolling_glide_and_speed_settings_apply_as_in_the_text_viewer` |
| 7.68 | 7.47 | 1.17 | — | new | ui test integration | `panel_toolbar_fit::pickers_turn_compact_only_when_the_drive_buttons_need_their_room` |
| 5.72 | 5.81 | 6.87 | — | new | engine lib | `copy_operations::tests::streamed::a_streamed_move_across_volumes_removes_each_source_only_after_its_verified_copy` |
| 5.36 | 5.20 | 6.62 | 6.43 | on record | ui test integration | `mouse::actual_mouse_selection_and_activation_preserve_entry_identity` |
| 6.08 | 5.60 | 5.68 | — | new | engine lib | `checksum::manifest::tests::exports_of_any_size_are_encoded_and_read_back` |
| 5.83 | 5.84 | 5.96 | 6.36 | on record | ui test integration | `busy_folder_native::a_refresh_that_changes_nothing_visible_leaves_the_model_and_every_row_untouched` |
| 5.57 | 5.45 | 5.65 | — | new | engine lib | `copy_operations::tests::archive::a_cancelled_selection_reaches_a_waiting_ui_after_a_few_events` |
| 4.88 | 4.90 | 5.58 | — | new | viewer-engine lib | `image_worker::document_band_tests::a_fast_drag_publishes_the_cameras_it_passes` |
| 5.16 | 5.22 | 5.49 | 4.48 | on record | viewer-engine lib | `worker::tests::long_rows::rows_of_2400_bytes_keep_every_gesture_s_travel::source` |
| 5.06 | 5.21 | 5.47 | 4.48 | on record | viewer-engine lib | `worker::tests::long_rows::rows_of_2400_bytes_keep_every_gesture_s_travel::text` |
| 5.15 | 5.08 | 4.74 | — | new | engine lib | `checksum::format::tests::a_manifest_of_any_size_parses` |
| 4.76 | 4.67 | 4.93 | 5.71 | on record | ui test integration | `busy_folder_native::a_folder_that_keeps_changing_is_relisted_at_a_bounded_rate_and_converges` |
| 4.86 | 4.45 | 4.81 | — | new | engine lib | `clipboard::paste_capture_tests::paste_refuses_more_native_roots_than_one_copy_takes_as_it_reads_them` |
| 4.71 | 4.74 | 4.78 | — | new | compare-ui lib | `scroll_parity_tests::touchpad_motion_and_its_momentum_move_the_text_as_in_the_text_viewer` |
| 4.07 | 4.03 | 4.78 | — | new | engine lib | `copy_operations::tests::streamed::a_vanished_item_fails_alone_and_retry_takes_it_again` |
| 3.68 | 3.75 | 4.73 | 4.73 | on record | ui test integration | `native_operation_layout::native_operation_layout_preserves_long_unicode_content_and_action_bounds` |
| 4.04 | 4.20 | 4.53 | 3.21 | on record | viewer-engine lib | `worker::tests::long_rows::rows_of_1200_bytes_keep_every_gesture_s_travel::source` |
| 3.24 | 3.55 | 4.38 | — | new | engine lib | `copy_operations::tests::streamed::a_streamed_copy_takes_each_item_and_totals_them_item_by_item` |
| — | — | 4.35 | — | new | compare-ui lib | `scroll_parity_tests::the_scrolling_glide_and_speed_settings_apply_as_in_the_text_viewer::glide_very_long` |
| 3.92 | 4.06 | 4.34 | 3.21 | on record | viewer-engine lib | `worker::tests::long_rows::rows_of_1200_bytes_keep_every_gesture_s_travel::text` |
| 4.09 | 4.10 | 4.22 | 4.63 | on record | ui test integration | `busy_folder_native::a_changed_file_updates_only_its_row_and_keeps_colors_and_folder_hints` |
| 4.11 | 3.99 | 4.05 | — | new | engine lib | `copy_operations::tests::archive::a_copy_reading_the_folder_an_archive_copy_stages_in_waits_for_the_whole_copy` |
| 3.77 | 3.85 | 4.00 | 4.49 | on record | viewer-engine lib | `worker::tests::long_rows::rows_of_2400_bytes_fill_the_window_at_rest::text` |
| 3.74 | 3.78 | 3.88 | 4.49 | on record | viewer-engine lib | `worker::tests::long_rows::rows_of_2400_bytes_fill_the_window_at_rest::source` |
| 3.85 | 3.86 | 3.85 | 3.84 | on record | engine lib | `watch::tests::a_folder_without_notifications_is_checked_often_and_reported_only_when_it_lasts` |
| 3.83 | 3.83 | — | — | new | compare-ui lib | `scroll_parity_tests::wheel_notches_move_the_text_as_far_and_as_smoothly_as_in_the_text_viewer` |
| — | — | 3.81 | — | new | compare-ui lib | `scroll_parity_tests::scroll_keys_and_horizontal_scrolling_move_the_text_as_in_the_text_viewer::horizontal_notch` |
| 3.73 | 3.57 | 3.69 | — | new | engine lib | `copy_operations::tests::archive::a_paused_archive_copy_keeps_its_staging_folder_and_resumes_in_it` |
| 3.15 | 3.27 | 3.64 | — | new | viewer-engine lib | `worker::tests::frontier::a_notch_up_after_a_scrollbar_position_needs_no_corrected_surface` |
| 3.29 | 3.33 | 3.57 | — | new | viewer-engine lib | `worker::tests::row_groups::a_refill_shapes_only_the_rows_it_adds` |
| 2.99 | 3.07 | 3.51 | — | new | engine lib | `copy_operations::tests::streamed::an_item_replaced_after_its_identity_was_read_fails_alone_and_is_never_copied` |
| 2.63 | 3.16 | 3.51 | — | new | engine lib | `copy_operations::tests::streamed::a_streamed_selection_keeps_about_a_hundred_bytes_per_item_beside_its_paths` |
| 2.99 | 3.40 | 2.98 | 5.33 | on record | engine lib | `copy_operations::tests::moves::directories::recursive_move_crosses_native_enumeration_pages_without_losing_children` |
| — | — | 3.28 | — | new | compare-ui lib | `scroll_parity_tests::scroll_keys_and_horizontal_scrolling_move_the_text_as_in_the_text_viewer::arrow_right_x2` |
| 2.60 | 2.60 | 3.25 | 3.89 | on record | ui test integration | `filename_colors::default_palette_renders_every_category_with_contrast_in_every_theme_and_selection_state` |
| 3.21 | 3.16 | 3.18 | — | new | engine test integration | `checksums_backend::checksum::manifest::tests::exports_of_any_size_are_encoded_and_read_back` |
| 3.19 | 3.18 | 3.19 | 3.20 | on record | engine test integration | `watch_native::a_failed_registration_is_checked_often_and_retried_without_a_status` |
| 3.17 | 3.18 | 3.17 | — | new | engine lib | `user_command_output::tests::stop_ends_the_tree_and_kills_it_when_asked_politely_in_vain` |
| 2.56 | 2.66 | 3.13 | 6.61 | on record | engine lib | `operations::metadata_tests::traversal::recursive_edit_changes_every_entry_of_a_folder_with_1000_entries` |
| 2.46 | 2.42 | 3.12 | 3.29 | on record | ui test integration | `path_bar_fit::path_components_and_the_edit_area_stay_inside_the_bar_at_every_width` |
| 2.36 | 2.41 | 3.08 | — | new | ui test integration | `copy_dialog_polish::a_long_first_line_and_a_note_wrap_whole_at_every_width` |
| 2.31 | 2.31 | 3.08 | — | new | ui test integration | `options::tab_walk::every_options_page_tab_cycle_keeps_to_shown_enabled_controls` |
| 2.97 | 2.94 | 2.93 | — | new | engine test integration | `checksums_backend::checksum::format::tests::a_manifest_of_any_size_parses` |
| 2.83 | 2.60 | 2.96 | 1.68 | on record | ui lib | `scroll_input::pane_tests::kinetic_tests::x11_fractional_line_stream_follows_the_finger_then_coasts` |
| 2.67 | 2.68 | 2.92 | — | new | ui lib | `scroll_input::pane_tests::reveal_tests::home_and_end_show_the_cursor_row_where_the_cursor_stays` |
| 2.52 | 2.61 | 2.92 | 3.66 | on record | ui test integration | `drag_integration::folder_targets_follow_actual_scrolled_cells_in_details_brief_and_thumbnail_modes` |
| 2.49 | 2.87 | 2.55 | 3.17 | on record | ui test integration | `tree_clipboard::cut_nested_folders_from_right_tree_and_confirm_native_move_into_left_tree` |
| 2.33 | 2.42 | 2.86 | 5.70 | on record | engine lib | `operations::metadata_tests::traversal::queued_metadata_jobs_holding_every_shared_target_cannot_limit_running_recursive_edits` |
| 2.18 | 2.27 | 2.85 | 3.61 | on record | ui test integration | `themes::all_themes_are_selectable_and_keep_text_readable` |
| 2.76 | 2.84 | 2.76 | 5.36 | on record | engine lib | `copy_namespace::tests::one_hundred_thousand_sibling_metadata_roots_use_indexes_and_shared_parent` |
| 2.65 | 2.58 | 2.83 | — | new | ui lib | `scroll_input::pane_tests::reveal_tests::up_and_down_show_the_cursor_row_at_the_first_and_last_rows` |
| 2.68 | 2.78 | 2.80 | — | new | ui lib | `scroll_input::pane_tests::reveal_tests::page_up_and_page_down_show_the_cursor_row_at_the_first_and_last_rows` |
| 2.28 | 2.33 | 2.79 | — | new | engine lib | `copy_operations::tests::streamed::a_streamed_move_moves_each_item_and_keeps_no_item_prepared_ahead` |
| 2.22 | 2.71 | 2.76 | — | new | engine lib | `copy_operations::tests::moves::metadata::required_modified_time_takes_at_most_64_files_for_copy_and_move` |
| 2.26 | 2.31 | 2.67 | — | new | engine lib | `copy_operations::tests::streamed::a_streamed_move_takes_a_folder_selected_twice_once` |
| 2.66 | 2.63 | 2.63 | 1.69 | on record | ui lib | `scroll_input::pane_tests::kinetic_tests::phased_touchpad_flick_coasts_once_with_decaying_steps_then_settles` |
| 2.05 | 2.08 | 2.65 | 3.23 | on record | ui test integration | `tree_clipboard::copy_nested_folders_from_right_tree_and_paste_into_left_tree_root` |
| 2.05 | 2.09 | 2.64 | 3.45 | on record | ui test integration | `tree_clipboard::copy_nested_folders_from_left_tree_and_paste_into_right_tree_root` |
| 2.60 | 2.41 | 2.62 | — | new | ui lib | `scroll_input::pane_tests::rows_tests::rows_paint_their_model_state_after_flings` |
| 2.39 | 2.43 | 2.61 | 2.51 | on record | viewer-engine lib | `worker::tests::long_rows::rows_mixing_short_and_very_long_ones_keep_every_gesture_s_travel::source` |
| 2.05 | 2.06 | 2.60 | 3.17 | on record | ui test integration | `tooltips::shared_tooltips_keep_cursor_clear_in_panels_dropdowns_and_options` |
| 2.32 | 2.43 | 2.60 | 2.51 | on record | viewer-engine lib | `worker::tests::long_rows::rows_mixing_short_and_very_long_ones_keep_every_gesture_s_travel::text` |
| 2.44 | 2.30 | 2.59 | 3.08 | on record | ui test integration | `operations_native_ui::real_folder_drag_uses_same_volume_move_ctrl_copy_and_escape_cancellation` |
| 2.40 | 2.31 | 2.57 | — | new | ui lib | `scroll_input::pane_tests::rows_tests::reveals_refreshes_and_sorting_keep_the_rows_covering_the_list` |
| 2.07 | 2.18 | 2.57 | 3.24 | on record | ui test integration | `tree_clipboard::cut_nested_folders_from_left_tree_and_confirm_native_move_into_right_tree` |
| 1.87 | 1.92 | 2.50 | 3.30 | on record | ui test integration | `tree_filters::native_tree_controls_apply_clear_sort_and_cancel_across_layouts` |
| 2.43 | 2.05 | 2.39 | — | new | engine lib | `copy_operations::tests::scheduler::immediate_start::reading_a_paused_copys_output_says_it_waits_for_the_same_folders` |
| 1.93 | 1.90 | 2.42 | 3.01 | on record | ui test integration | `tree_move::native_move_updates_both_trees_and_new_file_viewer_without_losing_marks` |
| 1.99 | 1.91 | 2.41 | 2.69 | on record | ui test integration | `dialog_layout::dialogs_keep_actions_visible_in_all_themes_and_densities` |
| 1.87 | 1.86 | 2.40 | 3.21 | on record | ui test integration | `native_controls::native_choices_keep_gestures_tooltips_and_full_unicode_working_paths` |
| 1.87 | 1.79 | 2.33 | — | new | viewer-engine lib | `worker::tests::prepared::a_fast_wheel_spin_never_waits_for_the_next_page` |
| 1.83 | 1.85 | 2.33 | — | new | viewer-engine lib | `worker::tests::prepared::page_down_never_waits_for_the_next_page` |
| 2.33 | 1.89 | 1.91 | — | existed | platform-windows lib | `icons::tests::native_link_composite_ignores_arbitrary_root_and_extension_data` |
| 2.33 | 1.89 | 1.90 | — | existed | platform-windows lib | `icons::tests::unassociated_document_art_is_identified_for_semantic_fallbacks` |
| 2.29 | 2.24 | 2.21 | 4.46 | on record | engine test integration | `operations_real_windows::real_directory_corpus_matches_independent_sha256_for_both_policies` |
| 2.06 | 2.04 | 2.26 | — | new | viewer-engine lib | `worker::tests::row_groups::a_refilled_page_answers_copies_and_finds_as_a_fresh_layout` |
| 1.83 | 1.86 | 2.25 | — | new | viewer-engine lib | `worker::tests::prepared::scrolling_inside_the_prepared_window_prepares_nothing_again` |
| 2.23 | 2.19 | 2.17 | — | new | explorer bin explorer | `thumbnails::pulse_tests::a_selected_video_steps_through_its_frames_in_order_at_a_steady_pace` |
| 2.17 | 0.65 | 1.76 | — | new | platform-windows lib | `icons::sized_tests::unassociated_types_are_generic_at_every_edge_and_associated_ones_are_not` |
| 2.17 | 1.72 | 1.75 | — | existed | platform-windows lib | `icons::tests::concurrent_native_icon_requests_preserve_optional_availability_and_pixel_bounds` |
| 2.15 | 2.16 | 2.08 | — | new | engine lib | `copy_operations::tests::moves::directories::recursive_move_of_a_tree_deeper_than_128_folders_moves_it_whole` |
| 2.16 | 1.72 | 1.75 | — | existed | platform-windows lib | `icons::tests::current_native_volume_renders_bounded_nonempty_system_icon_pixels` |
| 2.16 | 1.72 | 1.74 | — | existed | platform-windows lib | `icons::tests::folder_and_file_links_request_system_composites_without_any_path_lookup` |
| 2.16 | 1.72 | 1.74 | — | existed | platform-windows lib | `icons::tests::native_action_and_known_folder_render_without_reading_a_supplied_path` |
| 1.73 | 1.67 | 2.15 | 2.99 | on record | ui test integration | `tree_move::native_copy_updates_destination_tree_and_new_file_viewer_without_losing_marks` |
| 2.14 | 2.07 | 2.10 | — | new | engine lib | `copy_operations::tests::archive::a_completed_selection_closes_with_every_member` |
| 2.14 | 2.05 | 2.05 | 3.65 | on record | engine lib | `copy_operations::tests::scheduler::immediate_start::followed_source_folder_still_holds_back_a_write_into_it` |
| 1.99 | 2.12 | 2.11 | — | new | engine lib | `copy_operations::tests::scheduler::immediate_start::a_paused_copy_latched_unconfirmed_holds_back_a_sibling_and_says_why` |
| 1.77 | 1.55 | 2.11 | 2.55 | on record | ui test integration | `tree_move::f6_moves_nested_native_directories_from_right_panel` |
| 2.06 | 2.00 | 2.08 | — | new | ui lib | `presenter::tests::selection_anchor::a_million_rows_keep_the_anchor_bookkeeping_constant` |
| 2.08 | 1.70 | 1.67 | — | new | platform-windows lib | `icons::sized_tests::shell_artwork_is_rendered_at_every_edge_it_is_drawn_at` |
| 1.47 | 1.50 | 2.06 | 2.38 | on record | ui test integration | `command_line_layout::command_editor_and_controls_match_the_footer_across_themes_and_font_sizes` |
| 2.03 | 2.03 | 2.03 | 2.08 | on record | engine lib | `engine_comparator_policy_tests::settings_return_after_shutdown_grace_and_runtime_exit_retires_on_cleanup_worker` |
| 1.95 | 1.86 | 2.03 | — | new | engine lib | `copy_operations::tests::scheduler::immediate_start::a_paused_copy_into_unconfirmed_folders_holds_back_a_sibling_and_says_why` |
| 2.00 | 1.81 | 1.91 | — | new | engine lib | `copy_operations::tests::scheduler::immediate_start::a_copy_into_unconfirmed_folders_waits_for_a_paused_sibling_and_says_why` |
| 1.71 | 1.59 | 1.99 | 2.57 | on record | ui test integration | `tree_move::f6_moves_nested_native_directories_from_left_panel` |
| 1.71 | 1.61 | 1.99 | 1.83 | on record | ui test integration | `tree_copy::captured_native_copy_passes_unchecked_directory_links_to_preparation` |
| 1.99 | 1.87 | 1.87 | 3.13 | on record | engine lib | `copy_operations::tests::scheduler::immediate_start::same_named_item_waits_alone_while_the_rest_of_the_copy_proceeds` |
| 1.70 | 1.68 | 1.97 | 2.04 | on record | ui test integration | `tree_copy::native_nested_folders_copy_and_deselect_through_f5` |
| 1.81 | 1.80 | 1.96 | — | new | ui lib | `scroll_input::pane_tests::reveal_tests::refreshes_and_toggles_that_keep_the_row_leave_a_scrolled_list_alone` |
| 1.82 | 1.77 | 1.95 | 2.40 | on record | ui test integration | `tree_watch::external_root_and_expanded_branch_changes_preserve_right_tree` |
| 1.52 | 1.48 | 1.94 | 2.20 | on record | ui test integration | `tree_folder_sizes::explicit_size_sort_and_filter_preserve_calculated_descendant_values` |
| 1.51 | 1.56 | 1.93 | 2.34 | on record | ui test integration | `panel_selection::selected_rows_keep_their_fill_when_focused_in_every_theme_and_view` |
| 1.79 | 1.71 | 1.90 | 1.78 | on record | ui test integration | `tree_copy::archive_nested_folders_copy_and_deselect_through_f5` |
| 1.50 | 1.49 | 1.90 | 2.28 | on record | ui test integration | `hover_feedback::hover_looks_keep_each_themes_contrast` |
| 1.52 | 1.54 | 1.89 | 2.33 | on record | engine lib | `operations::metadata_tests::traversal::windows::recursive_hidden_attribute_reaches_1000_entries` |
| 1.82 | 1.74 | 1.88 | 2.34 | on record | ui test integration | `tree_watch::external_root_and_expanded_branch_changes_preserve_left_tree` |
| 1.87 | 1.85 | 1.86 | 1.90 | on record | engine lib | `watch::tests::a_folder_that_keeps_changing_is_hinted_at_most_once_per_cooldown` |
| 1.73 | 1.63 | 1.86 | 2.34 | on record | ui test integration | `tree_user_command_process::tree_user_command_actual_native_per_item_preserves_unicode_selection_and_retires_children` |
| 1.85 | 1.86 | 1.83 | 1.97 | on record | compare-ui test integration | `find::find_keys_search_both_files_center_hits_and_clear_stale_results_without_a_close_button` |
| 1.66 | 1.60 | 1.86 | 2.26 | on record | ui test integration | `tree_user_command_process::tree_user_command_actual_native_once_preserves_nested_unicode_path_and_retires_child` |
| 1.78 | 1.75 | 1.85 | — | new | ui lib | `scroll_input::pane_tests::reveal_tests::left_and_right_show_the_cursor_row_in_brief_and_thumbnails` |
| 1.80 | 1.31 | 1.37 | — | existed | explorer bin explorer | `window_backend::native::tests::native_debug_layers_are_explicit_and_indirect_validation_is_retained` |
| 1.44 | 1.41 | 1.80 | 2.39 | on record | ui test integration | `tree_checksums::tree_checksums_ctrl_f8_right_selected_nested_folder_and_root_file_recalculate` |
| 1.78 | 1.74 | 1.66 | — | new | engine lib | `copy_operations::tests::preserve::a_streamed_selection_preserves_every_item_without_a_bound` |
| 1.43 | 1.51 | 1.78 | 1.34 | on record | viewer-engine lib | `worker::tests::overlap::page_keys_read_only_the_rows_past_the_shown_page::source` |
| 1.53 | 1.56 | 1.76 | 2.71 | on record | engine lib | `copy_operations::tests::scheduler::immediate_start::the_busy_copy_publishes_into_output_that_a_waiting_copy_prepared` |
| 1.72 | 1.69 | 1.76 | 2.67 | on record | engine lib | `copy_operations::tests::scheduler::immediate_start::wait_for_earlier_copy_still_waits_for_the_busy_copy` |
| 1.72 | 1.68 | 1.74 | — | new | ui lib | `scroll_input::pane_tests::rows_tests::a_touchpad_scroll_resting_mid_folder_leaves_no_blank_band_below_the_header` |
| 1.71 | 1.73 | 1.71 | — | new | viewer-ui lib | `web_wheel_tests::the_wheel_speed_scales_a_notch_but_not_touchpad_pixels` |
| 1.72 | 1.61 | 1.69 | 1.07 | on record | ui lib | `scroll_input::pane_tests::real_file_panes_keep_fractional_wheel_positions_during_model_refresh` |
| — | — | 1.72 | — | new | compare-ui lib | `scroll_parity_tests::the_scrolling_glide_and_speed_settings_apply_as_in_the_text_viewer::speed_very_fast` |
| 1.44 | 1.59 | 1.70 | 1.89 | on record | engine lib | `operations::metadata_tests::traversal::recursive_edit_ignores_shared_targets_held_by_other_jobs` |
| 1.48 | 1.66 | 1.70 | — | new | engine lib | `copy_operations::tests::streamed::a_target_that_is_a_selected_item_not_read_yet_is_never_offered_for_replacement` |
| — | — | 1.70 | — | new | compare-ui lib | `scroll_parity_tests::the_scrolling_glide_and_speed_settings_apply_as_in_the_text_viewer::speed_very_slow` |
| 1.39 | 1.44 | 1.70 | 1.34 | on record | viewer-engine lib | `worker::tests::overlap::page_keys_read_only_the_rows_past_the_shown_page::text` |
| 1.33 | 1.36 | 1.68 | 1.94 | on record | ui test integration | `comparator_transaction::touch_external_comparator_workflow_saves_raw_paths_and_cancel_rolls_back` |
| 1.43 | 1.45 | 1.67 | — | new | viewer-engine lib | `worker::tests::row_groups::a_page_of_long_rows_grows_to_hold_the_prepared_screens` |
| 1.36 | 1.32 | 1.66 | 2.14 | on record | ui test integration | `tree_move::top_level_move_removes_the_row_and_preserves_the_expanded_tree_session` |
| 1.29 | 1.32 | 1.64 | 1.90 | on record | ui test integration | `operations_native_ui::real_f5_multiselection_toolbar_copy_and_f6_move_preserve_bytes_and_clear_selection` |
| 1.41 | 1.34 | 1.63 | — | new | ui test integration | `caption_glyphs::every_shown_character_has_a_glyph_in_the_application_font` |
| 1.56 | 1.45 | 1.38 | — | new | ui lib | `scroll_input::pane_tests::kinetic_tests::scrolling_speed_scales_a_notch_and_leaves_touchpad_motion_alone` |
| — | — | 1.55 | — | new | compare-ui lib | `scroll_parity_tests::wheel_notches_move_the_text_as_far_and_as_smoothly_as_in_the_text_viewer::reverse_mid_glide` |
| 1.21 | 1.15 | 1.54 | 1.95 | on record | ui test integration | `busy_folder_native::a_large_burst_replaces_the_rows_and_still_converges_without_the_loading_state` |
| — | — | 1.52 | — | new | compare-ui lib | `scroll_parity_tests::wheel_notches_move_the_text_as_far_and_as_smoothly_as_in_the_text_viewer::one_notch` |
| 1.18 | 1.20 | 1.52 | 2.02 | on record | ui test integration | `tree_checksums::tree_checksums_ctrl_f8_left_native_file_fallback` |
| 1.28 | 1.10 | 1.52 | 1.85 | on record | ui test integration | `tree_folder_sizes::archive_overlapping_selection_has_independent_ancestor_and_descendant_totals` |
| 1.27 | 1.27 | 1.50 | 1.79 | on record | ui test integration | `tree_folder_sizes::selected_descendants_report_exact_bytes_without_measuring_unselected_parent` |
| 1.20 | 1.21 | 1.48 | 1.86 | on record | ui test integration | `busy_folder_native::a_file_that_jumps_to_the_top_moves_without_refilling_the_rows_it_passed` |
| 1.47 | 1.40 | 1.47 | 2.73 | on record | engine lib | `copy_operations::tests::scheduler::immediate_start::reading_the_busy_copys_output_waits_for_it_with_its_reason` |
| 1.17 | 1.19 | 1.46 | — | new | ui test integration | `delete_dialog_polish::delete_confirmations_fit_their_content_at_every_font_and_density` |
| 1.25 | 1.31 | 1.46 | 1.98 | on record | ui test integration | `tree_user_commands::tree_user_command_right_selected_nested_folder_and_root_file_launch_in_display_order` |
| 1.46 | 1.46 | 1.37 | — | new | engine test integration | `find_backend::default_name_search_finds_a_late_match_after_more_than_one_million_entries` |
| 1.42 | 1.41 | 1.46 | — | new | compare-ui lib | `scroll_parity_tests::both_panes_stay_aligned_during_and_after_a_glide` |
| 1.25 | 1.12 | 1.44 | 2.06 | on record | ui test integration | `tree_user_commands::tree_user_command_left_unmarked_nested_folder_once_uses_native_focus` |
| 1.29 | 1.28 | 1.44 | — | new | engine lib | `user_command_output::tests::a_flood_of_output_stays_within_the_bounds` |
| 1.20 | 1.27 | 1.43 | — | new | viewer-engine lib | `worker::tests::row_groups::the_camera_moves_on_while_a_refill_reads` |
| 1.08 | 1.15 | 1.42 | 1.95 | on record | ui test integration | `panel_fit_and_empty::an_empty_folder_says_so_below_parent_which_still_goes_up` |
| 1.13 | 1.15 | 1.42 | — | new | ui test integration | `options::tab_walk::archives_tab_cycle_reaches_the_footer_and_wraps_in_visual_order` |
| 1.03 | 1.10 | 1.39 | 1.73 | on record | ui test integration | `tree_prefix_search::archive_quick_filter_preview_cancel_and_clear_keep_ancestor_authority` |
| 1.15 | 1.13 | 1.39 | — | new | ui test integration | `panel_scroll_costs::detailed_fling_through_50000_entries_reads_only_entering_rows` |
| 1.09 | 1.28 | 1.38 | 1.21 | on record | engine lib | `operation_throughput::tests::three_hours_keep_the_first_activity_and_each_series_peak_in_bounded_uniform_buckets` |
| 1.09 | 1.13 | 1.38 | 1.77 | on record | ui test integration | `panel_fit_and_empty::a_narrow_panel_shows_short_names_whole_and_drops_time_and_date_first` |
| 1.37 | 1.34 | 1.34 | 1.53 | on record | viewer-ui lib | `raster_frontier_tests::keys_and_wheel_keep_their_travel_across_corrected_surfaces_at_slow_reads` |
| 1.28 | 1.20 | 1.36 | 1.92 | on record | ui test integration | `tree_folder_sizes::native_bulk_measures_visible_folders_without_reordering_the_tree` |
| 1.11 | 1.22 | 1.35 | 2.06 | on record | ui test integration | `busy_folder_native::small_change_bursts_update_rows_in_place_without_the_loading_state` |
| 1.35 | 1.34 | 1.34 | 1.51 | on record | viewer-ui lib | `raster_frontier_tests::keys_and_wheel_keep_their_travel_at_slow_page_reads` |
| 1.30 | 1.34 | 1.32 | 4.54 | on record | engine lib | `copy_operations::preparation::planning_tests::planned_native_roots_and_discovery_release_each_real_source_before_the_next` |
| 1.20 | 1.28 | 1.33 | 1.85 | on record | ui test integration | `tree_folder_sizes::space_measures_its_original_descendant_while_other_folders_are_marked` |
| 1.04 | 0.99 | 1.33 | 1.60 | on record | ui test integration | `tree_clipboard::unselected_parent_is_excluded_and_focused_nested_folder_is_the_fallback` |
| 1.33 | 0.97 | 1.28 | — | new | engine lib | `copy_operations::tests::unprotected_source::a_streamed_copy_from_a_folder_without_exact_lookups_into_an_exact_folder_completes` |
| 1.15 | 0.99 | 1.32 | 1.55 | on record | ui test integration | `tree_folder_sizes::archive_bulk_reads_metadata_and_keeps_all_three_panel_layouts` |
| 1.31 | 1.28 | 1.25 | — | new | engine lib | `copy_operations::tests::unprotected_source::copies_of_items_without_exact_lookups_into_an_exact_folder_complete` |
| 1.13 | 1.15 | 1.31 | 1.56 | on record | ui test integration | `tree_command_insertion::native_tree_command_insertions_honor_persisted_remaps_without_default_fallbacks` |
| 1.30 | 1.03 | 1.02 | 2.18 | on record | engine lib | `copy_operations::tests::moves::windows::move_of_the_64_root_limit_verifies_and_removes_every_pinned_file` |
| 1.30 | 1.27 | 1.23 | — | new | ui lib | `scroll_input::pane_tests::kinetic_tests::a_modifier_a_click_or_a_notch_stops_momentum_and_only_alignment_follows` |
| 1.28 | 1.29 | 1.30 | 1.33 | on record | engine test integration | `selected_folder_sizes::selected_status_reads_cached_totals_and_tracks_global_and_individual_scans` |
| 0.99 | 0.99 | 1.29 | 1.62 | on record | ui test integration | `archive_badges::archive_vector_fallback_is_visible_and_associated_artwork_wins_in_every_view` |
| 0.93 | 0.88 | 1.26 | — | new | ui test integration | `commander_dialogs::comparison_dialog_and_progress_in_every_theme` |
| — | — | 1.26 | — | new | compare-ui lib | `scroll_parity_tests::wheel_notches_move_the_text_as_far_and_as_smoothly_as_in_the_text_viewer::five_quick_notches` |
| 1.07 | 0.98 | 1.24 | 1.63 | on record | ui test integration | `tree_prefix_search::unicode_typing_and_match_navigation_use_real_native_tree_publications` |
| 0.98 | 0.96 | 1.23 | 1.34 | on record | ui test integration | `tree_metadata::native_left_tree_attributes_use_captured_descendant` |
| 0.97 | 1.23 | 1.10 | — | new | engine lib | `copy_operations::tests::scheduler::immediate_start::paused_copy_of_the_files_links_point_to_does_not_hold_back_a_sibling_folder_copy` |
| 1.23 | 1.14 | 1.10 | 1.73 | on record | engine lib | `copy_operations::archive_source::size_tests::a_selection_of_more_than_4096_members_is_taken_whole` |
| 1.20 | 1.05 | 1.21 | 1.87 | on record | ui test integration | `tree_filters::archive_tree_inactive_panel_filter_preserves_authority_and_last_valid_result` |
| 1.13 | 1.20 | 1.12 | 2.51 | on record | viewer-engine lib | `worker::tests::long_rows::rows_mixing_short_and_very_long_ones_keep_every_gesture_s_travel::text_wrapped` |
| 0.90 | 0.98 | 1.19 | 1.46 | on record | ui test integration | `panel_search_render::product_prefix_color_and_compression_badge_survive_themes_density_fonts_and_row_input` |
| 1.19 | 1.17 | 1.17 | 1.39 | on record | viewer-ui lib | `raster_frontier_tests::keys_and_wheel_keep_their_travel_across_raster_and_page_frontiers` |
| 1.19 | 0.71 | 0.70 | 1.23 | on record | platform-windows lib | `shell_menu::tests::shell_menu_real_shell_resolves_unicode_multiselection_without_invoking` |
| 1.16 | 1.19 | 1.12 | 2.51 | on record | viewer-engine lib | `worker::tests::long_rows::rows_mixing_short_and_very_long_ones_keep_every_gesture_s_travel::source_wrapped` |
| 1.19 | 1.17 | 1.18 | 1.42 | on record | viewer-ui lib | `raster_frontier_tests::keys_and_wheel_keep_their_travel_across_a_corrected_surface` |
| 1.00 | 0.89 | 1.19 | 1.54 | on record | ui test integration | `tree_prefix_search::branch_collapse_rebuilds_matches_and_escape_preserves_collapsed_state` |
| 1.00 | 0.86 | 1.18 | 1.44 | on record | ui test integration | `tree_prefix_search::insert_clears_search_then_marks_the_captured_match_and_advances` |
| 1.17 | 1.17 | 1.17 | 1.17 | on record | engine test integration | `watch_native::a_resort_of_rows_a_check_found_current_reads_nothing` |
| 1.17 | 1.17 | 1.17 | 1.17 | on record | engine test integration | `watch_native::an_event_driven_refresh_lists_its_directory_once` |
| 1.17 | 1.17 | 1.17 | 1.17 | on record | engine test integration | `watch_native::a_resort_without_the_ui_rows_lists_its_directory_once` |
| 1.17 | 1.00 | 1.01 | 2.41 | on record | engine lib | `copy_operations::tests::scheduler::immediate_start::immediate_copy_into_the_same_folder_progresses_beside_a_busy_copy` |
| 1.00 | 0.99 | 1.17 | 1.53 | on record | ui test integration | `tree_bulk_selection::native_left_clear_removes_nested_folder_marks` |
| 0.91 | 0.94 | 1.16 | 1.37 | on record | ui test integration | `tree_command_insertion::virtual_and_control_names_do_not_insert_or_execute_and_parent_keeps_name_only` |
| 0.99 | 1.05 | 1.15 | 1.51 | on record | ui test integration | `tree_bulk_selection::native_right_clear_removes_nested_folder_marks` |
| 1.08 | 1.01 | 1.15 | 1.71 | on record | ui test integration | `tree_prefix_search::rapid_typing_escape_and_full_queue_converge_without_reopening_folders` |
| 0.89 | 0.93 | 1.15 | 1.53 | on record | ui test integration | `tree_bulk_selection::native_left_tree_shortcuts_distinguish_files_and_all_entries` |
| 0.84 | 0.85 | 1.14 | 1.33 | on record | ui test integration | `comparator_transaction::compact_external_comparator_workflow_saves_raw_paths_and_cancel_rolls_back` |
| 1.14 | 0.92 | 0.96 | 2.33 | on record | engine lib | `copy_operations::tests::scheduler::immediate_start::paused_followed_copy_does_not_hold_back_a_sibling_folder_copy` |
| 1.02 | 0.98 | 1.14 | 1.49 | on record | ui test integration | `tree_bulk_selection::native_right_tree_shortcuts_distinguish_files_and_all_entries` |
| 0.91 | 0.92 | 1.14 | 1.40 | on record | ui test integration | `native_operation_windows::native_operation_controls_remain_reachable_at_320_by_200_across_appearance_preferences` |
| 1.13 | 1.12 | 1.12 | 1.24 | on record | viewer-ui lib | `raster_frontier_tests::a_page_restyled_around_the_lookahead_does_not_stop_a_burst` |
| 1.06 | 1.03 | 1.13 | 1.39 | on record | ui test integration | `tree_folder_sizes::files_only_selection_never_falls_back_to_measuring_all_folders` |
| 1.09 | 0.95 | 1.13 | 1.63 | on record | ui test integration | `operations_native_ui::windows_delete::locked_directory_delete_reports_an_error_and_succeeds_after_release` |
| 0.90 | 0.91 | 1.13 | — | new | ui test integration | `metadata_editor::only_the_hovered_time_picker_arrow_lights_up_and_it_shows_in_every_theme` |
| 1.13 | 0.60 | 0.68 | 1.92 | on record | platform-windows lib | `user_command_terminal_tests::cmd::custom_cmd_companions_round_trip_native_argv_and_raw_environment` |
| 1.11 | 1.04 | 1.12 | — | new | engine lib | `copy_operations::input::root_bound_tests::a_file_list_of_more_native_roots_than_one_copy_takes_is_refused_at_admission` |
| 1.11 | 1.12 | 1.12 | — | new | explorer bin explorer | `thumbnails::pulse_tests::late_and_out_of_order_frames_play_only_in_order` |
| 1.11 | 1.02 | 1.04 | 1.99 | on record | engine lib | `copy_operations::tests::scheduler::immediate_start::one_disk_slot_interleaves_a_second_copy_with_a_busy_copy` |
| 1.02 | 1.04 | 1.10 | — | new | engine lib | `copy_operations::tests::scheduler::immediate_start::paused_copy_keeping_links_does_not_hold_back_a_sibling_folder_copy` |
| 1.10 | 1.08 | 1.09 | 1.79 | on record | compare-engine lib | `text::align::tests::many_changes_between_anchors_align_in_bounded_time` |
| 0.97 | 0.87 | 1.10 | 1.41 | on record | ui test integration | `tree_metadata::native_left_tree_case_uses_captured_descendant` |
| 0.80 | 0.89 | 1.09 | 1.26 | on record | ui test integration | `tree_command_insertion::native_tree_name_and_full_path_append_focused_raw_text_from_files_and_command` |
| 1.04 | 1.09 | 1.01 | — | new | engine lib | `copy_operations::tests::archive::a_cancelled_selection_closes_with_only_the_members_it_took` |
| 0.80 | 0.82 | 1.09 | 1.27 | on record | ui test integration | `tree_bulk_selection::archive_left_tree_shortcuts_distinguish_files_and_all_entries` |
| 0.87 | 0.91 | 1.08 | 1.52 | on record | ui test integration | `action_icon_colors::action_glyphs_share_the_dimmed_theme_tint_and_ignore_native_artwork` |
| 0.88 | 0.87 | 1.08 | 1.35 | on record | ui test integration | `filename_colors::all_themes_states_and_views_render_readable_filename_colors_with_preserved_cues` |
| 0.92 | 0.83 | 1.08 | 1.43 | on record | ui test integration | `tree_filters::actual_engine_receipt_after_navigation_cannot_apply_to_replacement_pane` |
| 1.08 | 1.06 | 0.99 | 1.94 | on record | engine lib | `copy_operations::tests::scheduler::immediate_start::immediate_copy_into_another_folder_progresses_beside_a_busy_copy` |
| 1.08 | 1.03 | 0.67 | — | new | engine lib | `copy_operations::tests::archive::a_cancelled_archive_copy_leaves_neither_stages_nor_its_staging_folder` |
| 0.85 | 0.94 | 1.08 | 1.55 | on record | ui test integration | `tree_rename::f2_native_right_expanded_descendant_routes_typed_capture` |
| 1.07 | 1.07 | 1.07 | 1.06 | on record | engine lib | `watch::tests::hidden_tests::rows_are_checked_with_the_choice_they_were_listed_with` |
| 0.94 | 0.83 | 1.07 | 1.34 | on record | ui test integration | `tree_bulk_selection::archive_right_clear_removes_nested_folder_marks` |
| 0.97 | 0.89 | 1.07 | 1.38 | on record | ui test integration | `tree_rename::f7_native_expanded_tree_keeps_existing_create_directory_route` |
| 0.87 | 0.83 | 1.06 | 1.29 | on record | ui test integration | `options_caption_fit::options_captions_stay_whole_at_the_minimum_size_with_large_fonts` |
| 0.85 | 0.82 | 1.06 | 1.34 | on record | ui test integration | `archive_path_bar::a_shortened_archive_path_keeps_the_archive_and_the_current_folder_whole` |
| 0.82 | 0.85 | 1.05 | 1.19 | on record | ui test integration | `archive_options::archives_topic_has_individual_archive_and_document_checkboxes` |
| 0.84 | 0.82 | 1.05 | 1.29 | on record | ui test integration | `tree_bulk_selection::archive_left_clear_removes_nested_folder_marks` |
| 0.81 | 1.00 | 1.04 | 1.32 | on record | ui test integration | `tree_filters::real_branch_overtaking_filter_reports_failure_and_keeps_draft_retryable` |
| 1.03 | 0.99 | 1.04 | 1.64 | on record | ui test integration | `tree_rename::f2_native_left_expanded_descendant_routes_typed_capture` |
| 0.97 | 0.95 | 1.04 | 1.45 | on record | ui test integration | `tree_metadata::native_right_tree_case_uses_captured_descendant` |
| 0.96 | 0.95 | 1.03 | 1.34 | on record | ui test integration | `tree_metadata::native_right_tree_attributes_use_captured_descendant` |
| 0.94 | 0.96 | 1.02 | 3.77 | on record | viewer-engine lib | `worker::tests::long_rows::rows_of_8000_bytes_keep_every_gesture_s_travel::source_wrapped` |
| 1.00 | 0.98 | 1.02 | 1.15 | on record | compare-engine lib | `capture_tests::bytes::the_hex_view_of_a_text_pair_finishes_its_scan_after_progress_frames` |
| 0.72 | 1.02 | 0.70 | — | new | engine lib | `copy_operations::tests::archive::a_change_to_a_member_never_taken_is_not_reported_as_a_cancelled_job_closes` |
| 1.02 | 1.00 | 0.97 | 3.21 | on record | viewer-engine lib | `worker::tests::long_rows::rows_of_1200_bytes_keep_every_gesture_s_travel::text_wrapped` |
| 1.02 | 1.02 | 1.01 | — | new | engine test integration | `navigation::requested_free_space_follows_a_change_of_the_shown_volume` |
| 1.01 | 1.02 | 1.01 | — | new | filesystem-host lib | `session::exchange_budget_tests::a_slow_drip_of_source_reads_ends_at_the_time_budget_and_frees_waiting_requests` |
| 0.80 | 0.81 | 1.01 | 1.39 | on record | ui test integration | `tree_editor::tree_editor_f4_routes_native_left_root_file_through_real_capture` |
| 1.01 | 1.00 | 0.98 | 3.21 | on record | viewer-engine lib | `worker::tests::long_rows::rows_of_1200_bytes_keep_every_gesture_s_travel::source_wrapped` |
| 1.01 | 0.99 | 0.99 | — | new | viewer-render lib | `rows::tests::coloured_long_rows_in_groups_answer_and_paint_as_one_layout` |
| 1.00 | 0.99 | 1.00 | 1.92 | on record | engine lib | `archive_sizes::tests::a_folder_of_more_than_100000_subfolders_is_sized_completely` |
| 0.81 | 0.76 | 1.00 | — | new | viewer-engine lib | `worker::tests::source_scroll::three_page_downs_at_once_move_the_page_past_the_first_row` |
