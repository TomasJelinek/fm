# UI performance fixes, 24 November 2026

The six open findings of the [UI review of 24 November](2026-11-24-review-ui.md) (backlog
row UI-PERF-01), fixed on the same day: each with a test that failed before the fix and
passes after it, and a count or timing before and after.

Environment: Windows 11, Rust 1.99.1, headless Slint software windows (no desktop
input). "Debug" is the workspace's test profile (`midnight-ui` and
`midnight-compare-ui` unoptimized; Slint's core optimized). "Optimized" is the release
profile (opt-level 3) without fat LTO, `cargo test --release`; its numbers are from
one run on a machine shared with other builds. Every benchmark is an ignored test that
reports numbers without thresholds.

## Checksums Select all

`checksum_windows.rs`, `Rows::marks_changed`. A selection change told the list of every
row whose mark changed, one `row_changed` each; each one notifies every view, also for a
row out of sight. Now a change of more than 256 rows (`MARKS_RESET`) resets the list once;
the list then reads again only the rows it shows. A click or a key that marks a few rows
still tells only those rows.

- Red → green: `checksum_windows::tests::bulk_mark_changes_notify_the_list_once_and_small_ones_per_row`
  (20,000 rows) failed with `(0 resets, 20000 rows)` for Ctrl+A and passes with one reset.
- Guard: `checksum_windows::selection::select_all_on_a_scrolled_list_keeps_its_rows_in_place`
  (integration, 3,000 rows, the list scrolled to its end): Ctrl+A and Ctrl+D keep the same
  rows at the same places and mark all, then none. It passed before and after.
- `checksum_selection_benchmark`, 100,000 rows:

| Measure | Before | After |
| --- | --- | --- |
| Notifications for Ctrl+A then Ctrl+D | 200,000 row changes | 2 resets |
| Ctrl+A, debug | 4.10 ms | 1.03 ms |
| Ctrl+D after Ctrl+A, debug | 6.99 ms | 1.43 ms |
| A click that drops every mark, debug | 4.94 ms | 1.37 ms |
| Ctrl+A, optimized | 1.1 ms (0.41 ms + the 0.70 ms of 100,000 row changes) | 0.41 ms |
| Ctrl+D after Ctrl+A, optimized | not measured | 0.30 ms |

The benchmark's window is never drawn, so no list is attached and a row change costs only
the notification; in a shown window each one also reaches the list's repeater. The
optimized "before" for Ctrl+A is the measured cost of the new path plus the measured
cost of the 100,000 row changes it no longer sends (`notify every row` in the benchmark,
0.70 ms; one reset rounds to 0.00 ms).

## Archive selected jumps

`presenter/selected_navigation.rs`. Alt+Up and Alt+Down in an archive folder looked up
every selected member on each key, O(selected). Now the first jump after the selection or
the listing changed builds the sorted display positions of the selected members once
(`SelectedPositions`), and every jump is a binary search over them, O(log selected). The
cache keeps both the selected set and the shown index weakly: every selection change
makes a new allocation (a new set, or `Arc::make_mut` moving a set the cache references
weakly), and a held weak reference keeps an address from being reused, so a stale cache
is never read.

- Red → green: `presenter::tests::selected_navigation::selected_navigation_archive_jumps_read_an_index_of_the_selection`
  (100,000 members, every other one selected, 1,000 jumps under one second, then a
  Ctrl+click between jumps seen in both directions, and Deselect all): 15.2 s before,
  under one second after (debug; the test passes).
- `selected_navigation_archive_jump_benchmark`, optimized, 100,000 members, 50,000
  selected: the old lookup of every selected member took 2.08 ms per jump; now the first
  jump after a change takes 5.2 ms (it builds the index) and every later one 252 ns.
  Alternating a selection change and a jump costs one build per jump, about 2.5 times the
  old jump; repeated jumps, the common case, are 8,000 times faster.

## Command output Find

`command_output_windows.rs`, `Rows::find`. Each search lower-cased every line into a new
string: 50,001 allocations for a search that matches nothing over the 50,000 retained
lines. Now each line is lower-cased into one reused buffer, ASCII lines byte by byte and
other lines character by character (`char::to_lowercase`, what `str::to_lowercase` writes
for every character but a capital sigma); a line with a Greek capital sigma, whose lower
case depends on its neighbours, still gets its own `str::to_lowercase`. An ASCII line is
skipped at once for a needle whose lower case is not ASCII. Matches are the same as
before: `find_ignores_case_as_unicode_lower_casing_does` compares with
`str::to_lowercase().contains` for length-changing mappings, the final sigma, the
Kelvin sign and ligatures.

- Red → green: `command_output_windows::tests::find_reads_every_line_without_allocating_per_line`
  failed with 50,001 allocations for a miss and passes with at most 4 (3 measured). It
  counts the allocations of the test's own thread through a test-only global allocator
  (`allocation_meter.rs`, `#[cfg(test)]`).
- `find_benchmark`, a miss over 50,000 lines (one of them with a capital sigma):

| Way | Debug | Optimized | Allocations |
| --- | --- | --- | --- |
| `Rows::find` before (each line cloned and lower-cased) | 11.45 ms | not measured | 50,001 |
| Lower-casing each line into a new string, alone | 4.72 ms | 3.10 ms | 50,001 |
| `Rows::find` after (one reused buffer) | 13.49 ms | 1.24 ms | 3 |
| Searching a cache of lower-cased lines | 2.01 ms | 0.34 ms | 1 (building it: 50,001, 2.9 MB) |

In the debug profile the new search is slower than the old one, because `midnight-ui` is
unoptimized there while `str::to_lowercase` comes optimized from the standard library;
optimized it is 2.5 times faster than lower-casing each line alone, without the
allocations.

A cache of lower-cased lines searches fastest, but costs 2.9 MB and 50,000 allocations to
build for 50,000 lines, and has to follow every line that arrives or is dropped; the
reused buffer costs neither, so it was chosen.

## Closed comparator windows

`midnight-compare-ui`, `OpenWindow::close`. A closed window stayed in its manager's list
(its component, timers, session and any output left in the session's mailbox) until
another comparator opened or the application exited. Now `close` queues a zero-delay
single-shot timer that takes the window out of the list on the next turn of the event
loop (`OpenWindow::release`), never inside the callback that closed it; dropping it
drops the component and the session, whose mailbox is dropped on a blocking thread. The
two-window limit counts open windows only.

- Red → green: `close_release_tests::a_closed_comparator_is_released_on_the_next_loop_turn`
  (Close button and title bar, with the worker's chooser unread in the mailbox) failed
  because the window was still alive after a loop turn, and passes: alive right after its
  own callback, released after one turn, the list empty, and a third window refused
  until one of two is closed.
- Windows kept after a close: 1 per closed window, up to the next open or exit, before; 0
  after one loop turn.

## Drag bridge wake-ups

`drag_bridge.rs`. The bridge polled every 250 ms for the whole run (`IDLE_POLL`) as a safety
net for region changes nothing reported. Now it arms no timer while idle. It polls every
16 ms only while a gesture, an external hover, a cancellation, a shutdown or a refused
drag command is pending (a failed presenter borrow arms that poll too), and a single-shot
timer retires the quick-drag note after 4 s. Every input of the published drop regions
queues one republish turn: refreshes, invalidations, native presses, releases, keys,
wheel and touch input, size and scale changes (Windows and Linux alike: winit events),
the list bounds' change handlers, and two new change handlers in `main.slint` over
everything else `rectangles` reads (`drag-regions-open`: a blocking dialog, an embedded
modal, the operation and copy forms, a scroll interaction; `drag-regions-shape`: a
zoomed panel and search results over a panel). Popups of this window open on input,
which already queues a turn; none opens on a timer (Slint's long-press context menu
exists on Android only). No safety net is left.

- Red → green: `drag_bridge::tests::an_idle_bridge_sleeps_and_every_region_change_wakes_it`
  failed with the idle timer armed and passes: each of seven region inputs withdraws and
  restores the right targets with no time passing, and ten idle seconds run no turn.
  The other bridge tests now expect no armed timer where they expected the 250 ms one, and
  the quick-drag note retires exactly 4 s after the release.
- Idle wake-ups of the bridge: 4 per second before, 0 after.
- Not run: the native drag-in and drag-out paths on a desktop (Windows OLE, Linux XDND and
  Wayland), which this change does not touch; the published regions they read change as
  before, only sooner.

## Jobs rows in place

`operation_presentation.rs`. Each refresh replaced the Jobs model whenever any row
differed, so every progress report rebuilt every row of the Jobs list. An in-place update
had been reverted because the Jobs window's Clear actions and selected-job lookup read
the rows through pure callbacks (`JobsView.has-finished`, `JobsView.find`) whose bindings
depended only on the model property, so they learned a finished job only from a replaced
model. Now those callbacks read the rows through the model's tracker
(`job_presentation::tracked_rows`), so their bindings follow rows changed in place; then
`navigation_models::update` changes the rows that differ in place and replaces the model
only when jobs come, go or change order.

- Red → green, the Jobs window first:
  `native_job_windows::native_jobs_follow_rows_changed_in_place` (integration) changes a
  row to finished in place; Clear all finished stayed disabled before and is enabled
  after.
- Red → green, the in-place update:
  `refresh_churn_tests::job_progress_updates_its_row_in_place_and_keeps_the_jobs_model`
  failed because a progress report replaced the model, and passes: three reports and the
  job's end keep the model, and clearing the finished job replaces it.
- `native_job_windows::native_jobs_clear_actions_preserve_active_work_and_repair_selected_details`
  (the test that failed with the reverted change) passes, as do the other native Jobs
  tests.
- Model replacements per progress report of an unchanged job list: 1 before, 0 after
  (one `row_changed` for the job's row).

## Commands and results

Debug test builds, `--locked --offline`, from the export root:

- Build: `cargo test --no-run -p midnight-ui -p midnight-compare-ui --locked --offline`.
- Red, at the base commit with only the new tests: the six red tests above failed
  (`midnight_ui` lib: 5 failed, 1 passed, the Unicode Find comparison; comparator lib:
  1 failed; `integration`: `native_jobs_follow_rows_changed_in_place` failed, the
  scrolled Select all guard and the reverted change's Jobs test passed).
- Green, with the fixes: `midnight_ui` lib 1,719 passed, 14 ignored; `midnight-ui`
  `integration` 1,227 passed, 10 ignored (two `tree_user_command_process` tests first
  failed because the test process had no `rustc` on its `PATH`, and passed with it);
  `midnight-compare-ui` lib 43 passed, 2 ignored; its `integration` 19 passed. All with
  bounded test threads (4 to 6) in software windows; no desktop or input test ran.
- Optimized benchmarks: `cargo test --release --no-run -p midnight-ui --lib` with
  `CARGO_PROFILE_RELEASE_LTO=false`, then `checksum_selection_benchmark`,
  `find_benchmark` and `selected_navigation_archive_jump_benchmark` with
  `--ignored --nocapture --test-threads=1`.
- `cargo clippy --workspace --all-targets --locked --offline --features
  midnight-acceptance/acceptance -- -D warnings`: clean on Windows.
  `cargo fmt --all -- --check` and `scripts/check-source-size.ps1`: clean
  (`main.slint` is 99,905 bytes).
- Linux: the changes contain no platform-specific code. A cross-clippy of
  `midnight-ui` and `midnight-compare-ui` for `x86_64-unknown-linux-gnu` from Windows
  stops in the build script of `yeslogic-fontconfig-sys`, which needs the Linux
  fontconfig through `pkg-config` (with `RUST_FONTCONFIG_DLOPEN`, `fontique` then fails
  to compile), so Linux was not checked; WSL was not available.
