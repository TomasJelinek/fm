# Test execution time — 2026-11-18

## Scope

How long the Windows workspace tests take to run before and after the deep code
review, and which tests and test executables dominate. The baseline is main
`3a27eeb2`, measured on 17 November; the result is the review's integration tip
`d0f44c55`, measured on 18 November. Build and link time are not included.

## Method

1. The 18 November run built its test executables with `cargo test --workspace
   --locked --offline --no-run --message-format=json-render-diagnostics`; the 17
   November run used `cargo test --workspace --all-targets --locked --offline
   --no-run --message-format=json`, which also built the 29 examples and the
   `midnight-ui-bindings` library (`test = false`) as test executables. Both used
   the debug profile without debug information; the `compiler-artifact` records list
   the executables.
2. Each test executable then runs alone, one after another, from its package
   directory as `cargo test` runs it, with libtest's default thread count, JSON
   output and `--report-time` (enabled for the test process only, through
   `RUSTC_BOOTSTRAP=1`). The wall time of each executable and libtest's execution
   time of each test are recorded; a package's time is the sum over its executables.
3. UI software-render snapshots were off, as in CI (`MIDNIGHT_UI_RENDER_DIR` unset).
   The 18 November run set `HOST_TEST_INSTALL_ROOT` to a protected directory that
   only the user, SYSTEM and Administrators can access (as CI's private test
   installation); the 17 November run did not.

The runner is an orchestrator script, not a repository tool; its results are in the
integration worktree's `_build/qa/timing-baseline/` and `_build/qa/timing-final/`
(`results.json` and one log per executable).

## Environment

Windows 11 Enterprise 10.0.26200, Rust 1.99.1, debug builds, NTFS on D:, Microsoft
Defender real-time protection on, a shared developer machine; one run per tip.

## Results

| | `3a27eeb2` | `d0f44c55` |
| --- | ---: | ---: |
| Test executables | 378 | 66 |
| Sum of executable wall times | 2,147.5 s | 238.5 s |
| Passed / failed / ignored tests | 5,373 / 63 / 100 | 6,155 / 0 / 127 |

The `3a27eeb2` column includes 30 executables that only its `--all-targets` build
produced: 29 examples (42.4 s, 9 passed tests) and the `midnight-ui-bindings`
library (1.2 s, no tests). Without them the baseline is 348 executables, 2,103.9 s
and 5,364 / 63 / 100 tests; `midnight-explorer` is then 12.7 s,
`midnight-image-viewer-host` 4.8 s and `midnight-ui-input` 7.2 s.

| Package | `3a27eeb2` s | `d0f44c55` s | `3a27eeb2` passed/failed/ignored | `d0f44c55` passed/failed/ignored |
| --- | ---: | ---: | --- | --- |
| `midnight-compare-engine` | 3.7 | 3.6 | 31/0/0 | 54/0/1 |
| `midnight-compare-ui` | 60.2 | 20.6 | 8/0/0 | 16/0/0 |
| `midnight-core` | 31.8 | 4.3 | 230/0/2 | 234/0/2 |
| `midnight-elevation` | 2.5 | 3.0 | 7/0/0 | 24/0/0 |
| `midnight-engine` | 350.9 | 28.0 | 1762/0/15 | 2050/0/24 |
| `midnight-explorer` | 23.0 | 14.6 | 69/0/19 | 111/0/21 |
| `midnight-filesystem-host` | 8.2 | 9.4 | 48/0/7 | 74/0/9 |
| `midnight-image-viewer-host` | 6.1 | 5.1 | 23/10/9 | 33/0/9 |
| `midnight-native-filesystem-host` | 4.5 | 4.4 | 136/0/3 | 137/0/4 |
| `midnight-platform` | 8.9 | 8.7 | 17/0/0 | 18/0/0 |
| `midnight-platform-common` | 90.5 | 10.6 | 240/0/0 | 244/0/0 |
| `midnight-platform-linux` | 13.1 | 2.5 | 0/0/0 | 1/0/0 |
| `midnight-platform-windows` | 28.7 | 26.4 | 315/0/15 | 391/0/23 |
| `midnight-plugin-host` | 6.6 | 4.0 | 47/51/6 | 108/0/7 |
| `midnight-thumbnail-host` | 25.4 | 4.1 | 51/0/13 | 59/0/13 |
| `midnight-ui` | 1355.6 | 36.7 | 1826/0/6 | 1938/0/6 |
| `midnight-ui-bindings` | 1.2 | — | 0/0/0 | — |
| `midnight-ui-input` | 8.4 | 8.2 | 32/0/0 | 32/0/0 |
| `midnight-viewer-engine` | 11.6 | 4.3 | 259/0/4 | 293/0/4 |
| `midnight-viewer-render` | 33.5 | 6.8 | 58/0/0 | 69/0/3 |
| `midnight-viewer-ui` | 45.9 | 17.2 | 167/0/0 | 169/0/0 |
| `midnight-web-viewer-host` | 4.3 | 0.4 | 6/0/0 | 36/0/0 |
| `midnight-work-budget` | 1.2 | 0.1 | 13/0/0 | 21/0/0 |
| `midnight-worker-platform` | 21.9 | 15.6 | 28/2/1 | 43/0/1 |
| **Total** | **2,147.5** | **238.5** | 5,373/63/100 | 6,155/0/127 |

The 63 failures of the baseline run were 51 plugin host and 10 image viewer host
library tests and one test each in the Windows spool and store tests of
`midnight-worker-platform`; their times are included in the baseline total. 61 of
them came from that run's environment: without `HOST_TEST_INSTALL_ROOT`, 44 plugin
host tests panicked on its absence and the other 17 failed because plugin discovery
refused the fallback fixture root below `_build/tmp` as writable by another account;
the two `midnight-worker-platform` tests did not find `pwsh`, which their junction
fixture started at `3a27eeb2` (`9c8c2851` creates it natively). All of them passed
on 18 November, one under a new name: the plugin host test
`manifest_removal_removes_capability_and_routing_respects_modes` is now
`discovery_publishes_manifest_modes_and_extensions_until_removal`; `6846ab33`
removed the unused plugin host routing API with the test's routing assertions
(routing is the viewer engine's), and `cdeac897` added checks of the discovered
modes and extensions.

The slowest tests of the baseline and their time at `d0f44c55`. Per-test times are
libtest's, measured while the other tests of the same executable ran beside them; the
thumbnail cache test now runs in the library's unit tests:

| `3a27eeb2` s | `d0f44c55` s | Package | Test |
| ---: | ---: | --- | --- |
| 77.41 | 5.64 | `midnight-engine` | `operations_real_windows::real_directory_corpus_matches_independent_sha256_for_both_policies` |
| 18.12 | 2.87 | `midnight-ui` | `themes::all_themes_are_selectable_and_keep_text_readable` |
| 17.44 | 1.62 | `midnight-ui` | `dialog_layout::dialogs_keep_actions_visible_in_all_themes_and_densities` |
| 14.82 | 0.45 | `midnight-ui` | `find_windows::find_actions_and_details_remain_accessible_in_every_theme_and_density` |
| 12.66 | 0.07 | `midnight-thumbnail-host` | `cache::duplicate_files_are_counted_toward_the_global_record_limit` |
| 11.98 | 0.43 | `midnight-ui` | `checksum_windows::checksum_actions_keep_320_by_200_layout_in_every_theme_and_density` |
| 10.49 | 4.08 | `midnight-ui` | `native_operation_layout::native_operation_layout_preserves_long_unicode_content_and_action_bounds` |
| 7.74 | 1.54 | `midnight-ui` | `mouse::actual_mouse_selection_and_activation_preserve_entry_identity` |
| 7.04 | 1.56 | `midnight-ui` | `command_line_layout::command_editor_and_controls_match_the_footer_across_themes_and_font_sizes` |
| 6.10 | 0.45 | `midnight-engine` | `copy_operations::tests::cancel_queue::queued_cancel_removes_only_unpublished_output_after_blocked_native_io` |

What changed: most crates link their integration tests into one `integration`
executable (`010f3b62`; the exceptions are in
[test execution](../developer/TEST_EXECUTION.md)), so one process runs a crate's
integration tests in parallel instead of one process per test file. UI tests advance
virtual clocks instead of sleeping, and the every-theme geometry sweeps still lay out
and check every theme but software-render only their first theme of each page, size
and font, because themes change colours only; tests that check pixels still render
every theme (`9bbc0241`). Tree fixtures became faster (`84963283`), and slow
obsolete fixtures were removed (`64a8e9c1`).

## Remaining cost

The slowest executables at `d0f44c55`:

| Executable | Wall s | Passed | Ignored |
| --- | ---: | ---: | ---: |
| `midnight-ui::test:integration` | 24.15 | 747 | 1 |
| `midnight-explorer::bin:me` | 14.63 | 111 | 21 |
| `midnight-engine::lib:midnight_engine` | 14.11 | 1629 | 16 |
| `midnight-engine::test:integration` | 13.89 | 421 | 8 |
| `midnight-compare-ui::test:integration` | 12.55 | 10 | 0 |
| `midnight-ui::lib:midnight_ui` | 12.50 | 1191 | 5 |
| `midnight-viewer-ui::lib:midnight_viewer_ui` | 9.51 | 149 | 0 |
| `midnight-compare-ui::lib:midnight_compare_ui` | 8.02 | 6 | 0 |

The four crates that still build several integration test executables, one per file
(`midnight-platform-windows` 12, `midnight-worker-platform` 11,
`midnight-filesystem-host` 3 and `midnight-image-viewer-host` 2) spent 43.3 s in
28 executables, which passed 119 tests and ignored 18, and across the workspace 12
executables
that pass no test on Windows (three contain no tests, four only Linux tests and five
only ignored opt-in tests) took
18.1 s. libtest reports at most 0.2 ms of suite time for each of them, so these 18.1
s are spent starting and ending the processes. The fastest executable took 0.06 s;
why most others spend up to 13.8 s outside their tests (for example on-access
scanning of freshly built executables) was not attributed. Across all 66 executables
libtest's suite times sum to 39.5 s of the 238.5 s (324.6 s of 2,147.5 s at
`3a27eeb2`); the rest is spent outside the test harness, most in the largest
executables (`me` 13.8 s, `midnight-ui` integration 12.2 s, `midnight-ui` library
11.0 s).

## Limitations

- Windows debug builds only; the Linux tests were not timed.
- One run per tip on a shared machine, so differences of a few seconds are within noise.
- Build and link time, which fewer executables also reduce, was not measured.
- The 17 November run overlapped this worktree's own test-consolidation builds and runs
  (`_build/qa/core-pilot.log`, `ui-consolidate.log` — an 11 m 47 s build from 10:20:46 —,
  `ui-int-run1.log`, `engine-consolidate.log`, `engine-int-run1.log`): 183 of its 378
  executables (978.6 s) ran beside them, 132 (723.0 s) beside the UI build, among them the
  corpus test (77.41 s) and `themes` (18.12 s); those baseline times are upper bounds.
  Nothing ran beside the 18 November run.
- The separate plugin and API workspaces were not timed.
