# Test timeout races (TEST-03) — 2026-11-18

## Scope

Two fixed-deadline waits each failed once in a Linux workspace test run of
`ac4c2e87` (TEST-03 in the [backlog](../IMPLEMENTATION_BACKLOG.md)):

- `navigation::blocked_left_listing_does_not_prevent_right_completion`
  (`midnight-engine` integration tests): `left worker entered provider: Elapsed(())`
  after its 3 s wait, `crates/midnight-engine/tests/navigation.rs` line 231.
- `operations_native_ui::real_copy_conflict_shows_both_entries_in_the_native_locale`
  (`midnight-ui` integration tests): `UI timed out after 15s: left= right= notice=
  copy=None operation=None`, `crates/midnight-ui/tests/operations_native_ui.rs`
  line 88, the wait for the Copy dialog after F5.

Both were races inside the tests. Neither involves state shared with other tests
of the same binary, such as the process-wide disk-I/O gate: each reproduces with
only its own test in the process, and an injected delay reproduces each with its
identical message. This record gives the causes, the test-only fixes, an audit of
the other engine test waits and the before/after evidence. Source: main `cf8138e4`
and the TEST-03 fix commits on top of it.

## Causes

### A held listing behind a full event channel

The test creates its event channel with 8 slots and then waits up to 3 s for the
left listing to enter the provider (a `Notify`) without reading events. A fresh
Engine sends 7 events that nobody asked for: 5 fallback icons from the desktop
icon preload, `NetworkAvailability` and `ArchiveAvailability`. The left
navigation's auxiliary task adds `StorageKindUpdated` and `SpaceUpdated`. The
listing task's first step is `outbox.send(ListingStarted)`, before its listing
permit and before the provider call. When 8 of those 9 events are queued before
that first poll, the send waits for room that the test never makes: the listing
never reaches the provider, and the stall is permanent, not slow.

### A saved-tab restore after the test's click

Installing the UI sends `LoadPreferences` before both panels' first navigations.
The harness saves both panel paths as tabs, so applying the Engine's
`PreferencesLoaded` restores them (`restore_tab_session`): each panel is replaced
by a new pane that is listed again, even though it shows the same folder
(`activate_tab`, `replace_pane_location`). `Harness::new` waited only for the first
listings. When the preferences were applied after that wait, inside the settle of
`focus()` and so after the row click, the replaced panel had no cursor and no
selection, and F5 returned without a message (`request_selection_action`), so no
Copy dialog appeared. The production application keeps its window hidden until the
preferences are applied; the harness has no such gate. The product behaviors are
backlog rows TABS-01 (the replaced panel) and NOTICE-03 (no message).

The kept fixture of the historical failure shows a `Harness::new` of 2.24 s instead
of about 0.2 s (fixture created at 15:43:15.528, `destination/file-03.bin` written
at 15:43:17.764). No trace of that run exists, so its path is attributed by the
mechanism and the reproduction below, not observed.

## Fixes (tests only)

- `navigation.rs`: `entered_within` waits for the provider signal while it reads
  the Engine's events, and fails at once if a listing finishes or fails meanwhile;
  all four waits of the file use it, with the unchanged 3 s guard.
  `uninterruptible_old_reads_cannot_exhaust_the_other_panes_workers` now also waits
  for the third listing's `ListingStarted` before its 100 ms window, so that window
  can pass only because the pane's two listing permits are held, not because a full
  channel holds the listing back.
- The same pattern, found by the audit below: `navigation_fallback.rs`
  (`cancelled_uncancellable_failure_emits_no_fallback_and_failed_root_stops`, 16
  slots) reads events while it waits; `find_actor.rs`
  (`pending_find_can_cancel_while_two_native_calls_are_blocked_and_session_admission_is_bounded`,
  16 slots; each session publishes its start before it opens its root) drops the
  events it does not check while it waits; `find_sort_after_stop.rs`
  (`stop_publishes_matches_below_one_batch_and_orders_them_automatically`, 16
  slots; the worker publishes its start and, every 250 ms, progress with a partial
  batch before the pause) keeps the Find updates read while it waits for the pause.
- `crates/midnight-ui/tests/support/engine_startup.rs` (new): the startup tabs
  and whether the preferences are applied and their restore has replaced the
  startup tab of both panels. `operations_native.rs` waits for that and for both
  restored listings; `tree_filters.rs`, which applies `PreferencesLoaded` itself,
  waits for the restore as well, because a restore navigation that the command
  queue refused is sent again with a later event. Comparing tab identities covers
  that retry, which `window_geometry_ready()` alone does not.
- `Harness::focus` (operations harness) asserts after its settle that the cursor
  is on the clicked row and otherwise fails at once with the cursor position and
  row, instead of a later 15 s wait for the key's result.

## Audit of engine test waits

Every wait in `crates/midnight-engine/tests` and in the engine's own test modules
that start an Engine was checked for a wait on a provider, service or task signal
while the test does not read a bounded event channel that the Engine fills.

| Waits | Result |
| --- | --- |
| The four waits of `navigation.rs` (8, 32, 32 and 16 slots) and the three tests above (16 slots each) | Fixed: the awaited call follows a send to the event channel, with little headroom or, in `find_sort_after_stop`, a time-driven number of sends. |
| `clipboard.rs` and `clipboard_retirement_tests.rs` (session copied/dropped/closed), `desktop.rs` (properties, open, system location, helper reaping), `file_actions.rs` (clipboard owner pumps), `folder_sizes.rs` (14 waits) and `folder_size_warnings.rs` (scan lanes enter the directory), `engine_configuration.rs`, `engine_lifecycle_tests.rs`, `engine_command_admission_tests.rs`, `engine_comparator_policy_tests.rs` and `archives/retirement_tests.rs` (worker, retirement and drop markers; settings load) | Cannot block: nothing sends to the event channel before the awaited call. The clipboard copies before it reports; desktop workers call the service before they send; folder-size lanes hand results to the emitter through their own 8-slot channel and read the directory first; the retirement tests do not run the Engine or wait only for its retirement. |
| `tree_controller.rs` (8 waits, 32 and 64 slots), `tree_filters.rs` (64), `watch_native.rs` (2 waits, 32), `volumes.rs` (64), `find_actions.rs` (`a_blocked_result_validation_does_not_stall_another_validation`, 32) and the icon and context gates of the tests in `src/desktop.rs` (64 and 32) | Cannot block: the test has read the events of its earlier commands, so at most the 7 startup events, the 2 auxiliary events of a navigation and a few events of the awaited request can be queued, far fewer than the slots. |
| `tree_recursive_controller.rs`, `find_current_directory.rs` and all loops over `next(&mut events)` | Not affected: they read events while they wait. |

## Method

- Linux binaries were built in the WSL mirror with
  `wsl-cargo.sh <worktree> orch test -p midnight-engine --test integration --locked --offline -j 4 --no-run`
  (and `-p midnight-ui`) and run directly from the package directory, with the
  wrapper's environment (`TMPDIR`, toolchain on `PATH`), by a loop that records the
  1-minute load average before each run. Windows binaries came from
  `cargo test -p midnight-engine -p midnight-ui --test integration --locked --offline -j 4 --no-run`
  and ran from the package directory with the toolchain on `PATH`. Before and after
  binaries differ only in the changed test files; the baselines are builds of the
  original files at `cf8138e4`.
- Injected delays use gdb in non-stop mode, so one thread sleeps in `usleep` while
  all other threads keep running. Engine: a breakpoint on the first poll of the
  listing task (`TaskFuture<run_inner::{closure#0}::{closure#34}>::poll`) holds it
  for 300 ms, and a breakpoint counts `Chan<Event>::send`. UI: the harness
  `Settings::load_preferences` is held for the stated time, and the trace
  timestamps `Harness::focus`, `Presenter::activate_tab`,
  `request_transfer_dialog`, `request_selection_action` and
  `dispatch_selected_action`.
- Build state: `cf8138e4` does not compile `midnight-engine` (two calls of the
  removed `RetirementDomain::try_reserve_nowait`) or its integration tests (the
  checksum test modules include `src/` files whose `crate::retirement`,
  `crate::copy_nested_retirement` and `crate::providers` paths do not resolve in the
  consolidated binary). Every run used a local, uncommitted workaround: the two
  calls renamed to `try_reserve`, and `tests/integration.rs` declaring those three
  names at the crate root. The workaround changes no code that these tests
  exercise; the engine binaries therefore run the 32 `retirement` unit tests once
  more than the source declares.
- The after runs below used the sources before two cosmetic edits (a line join in
  `navigation.rs` and `matches!` in `navigation_fallback.rs`). The committed
  sources were rebuilt and run again: on Linux `navigation::` passed 50 of 50 runs,
  `navigation_fallback::`, `find_actor::` and `find_sort_after_stop::` 20 of 20 and
  the full engine binary 2 of 2; on Windows `navigation::` passed 20 of 20, those
  three modules 5 of 5 and `operations_native_ui::` 5 of 5.
- `cargo fmt -p midnight-engine -p midnight-ui -- --check` passes. Windows Clippy
  (`-p midnight-engine -p midnight-ui --all-targets`) reports nothing in the changed
  files; with `-D warnings` it stops at code that predates these changes: four
  unused functions in `actor_thread.rs`, the unused `providers` imports of the two
  checksum test modules and two compile errors in the engine's library tests
  (`preferences/tests.rs`, `tree_controller/worker_refresh_tests.rs`).

## Environment

Windows 11 Enterprise 10.0.26200 with 20 logical CPUs and 32 GB, shared with other
build and test work (CPU load 3–100 % during the runs); WSL 2 Ubuntu 24.04.4
(kernel 6.6.87.2) with 20 CPUs and 15 GiB on the same machine; Rust 1.99.1; debug
test profile without debug information; gdb 15.1.

## Results

Engine, `navigation::blocked_left_listing_does_not_prevent_right_completion`:

| Run (Linux unless noted; 1-minute load) | Before (`cf8138e4`) | After |
| --- | --- | --- |
| The test alone (`--exact`), quiet | 5 of 200 failed (load 0.5–0.7) | 0 of 200 failed (load 2.4–2.7) |
| The test alone beside full UI binary runs | 9 of 200 failed (load 17–25) | 0 of 200 failed (load 13–20) |
| `navigation::` (8 tests) | 1 of 100 failed (load 0.2–3.8; binary built before the workaround's crate-root modules) | 0 of 200 failed (load 0.7–3.1) |
| The test alone, listing task's first poll held for 300 ms | 5 of 5 failed; 8 sends at resume and none after | 0 of 5 failed; 9 sends at resume, 20 in all |
| Full engine binary (456 tests, 7 ignored) | 5 of 5 passed (load 10–25) | 5 of 5 passed (load 13–15) |
| Windows `navigation::` | 0 of 40 failed | 0 of 40 failed |
| Windows full engine binary (471 tests, 8 ignored) | 5 of 5 passed | 5 of 5 passed |

Every failure was the identical panic at `navigation.rs:231` after 3.0–3.4 s.

UI, `operations_native_ui::real_copy_conflict_shows_both_entries_in_the_native_locale`
(Linux, the test alone under gdb, `Settings::load_preferences` delayed):

| Delay | Before: failed of runs | After: failed of runs |
| --- | --- | --- |
| 40 ms | 3 of 12 | 0 of 8 |
| 50 ms | 7 of 12 | 0 of 8 |
| 60 ms | 5 of 12 | 0 of 8 |
| 70 ms | 3 of 10 | 0 of 8 |
| 80, 100, 120, 140, 160 ms | 0 of 20 | not run |
| 115, 125, 135 ms | 0 of 15 | 0 of 15 |

Every failure was the identical timeout at `operations_native_ui.rs:88`. Failing
traces show `Harness::focus`, then two `activate_tab` calls inside its settle, then
F5 reaching `request_transfer_dialog` and `request_selection_action` but never
`dispatch_selected_action`; with the fix, both `activate_tab` calls come before
`Harness::focus` and F5 dispatches. The window depends on how fast `Harness::new`
runs: here 40–70 ms, in the investigation 115–135 ms, in its independent check
65–75 ms.

| Run | Before | After |
| --- | --- | --- |
| Linux `operations_native_ui::` (13 tests) | 0 of 50 failed (load 6–10) | 0 of 50 failed (load 0.1–2.6) |
| Linux full UI binary (764 tests, 1 ignored) | 4 unrelated failures (below) in each of 5 runs | the same 4 in each of 9 runs |
| Windows full UI binary (766 tests, 1 ignored) | 3 unrelated failures in each of 5 runs | the same 3 in each of 5 runs |

Without an injected delay the UI race did not occur in these runs, as in the
investigation: the preferences normally arrive within milliseconds of the first
listings, before the harness's startup wait ends.

At `cf8138e4` these UI tests fail in every run, independently of TEST-03 and of
these changes: `comparator_transaction::leaving_picker_page_releases_busy_controls_even_before_a_picker_reply`,
`options_footer::footer_keeps_actual_save_failure_visible_above_its_actions` and
`panel_selection::selected_rows_keep_their_fill_when_focused_in_every_theme_and_view`
on both platforms, and
`comparator_options::long_values_wrap_and_error_fields_remain_reachable_across_sizes_fonts_and_themes`
on Linux.

## Limitations

- Neither failure reproduced on Windows, before or after; the code paths and event
  counts are the same, so the Windows runs show only that the fix breaks nothing.
- The historical UI failure is attributed by mechanism and injected-delay
  reproduction; no natural reproduction of it exists.
- The audit's "cannot block" verdicts for 32- and 64-slot channels rest on today's
  7 unsolicited startup events; a change that adds startup events (for example
  fallback art for the 36 action icons) needs the same audit again.
- `cf8138e4` does not build without the local workaround above, and its full UI
  binary has the unrelated failures listed above.
- The product follow-ups are recorded, not changed: TABS-01, NOTICE-03 and IO-03
  (the process-wide disk-I/O gate held across native calls that cannot be
  cancelled).
