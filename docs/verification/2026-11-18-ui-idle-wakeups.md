# Explorer idle wakeups — 18 November 2026

Scope: PERF-F0 (measurement method), PERF-F1 (five repeated UI timers) and PERF-F2
(the thumbnail and visible/retry pulses) of integration batch 10b, measured on the
batch branch from base `93d36411`. Windows only; no Linux X11 run was made for this
record.

Status: the numbers below are indicative, not acceptance evidence. They come from
debug builds with the software renderer, mostly in a locked session, with a 10 s
warmup and a 30 s window, and S3 and S5 were not measured (see Conditions). The
PERF-F1 S1 target is met; S3 is unmeasured. The PERF-F2 target (S1, S2 and S4 at most
5/s) is **not met** while the window is active: a caret blinker outside the converted
timers remains (see below), and its fix needs a decision outside this batch. After
that decision, the measurements are to be repeated with the PERF-F0 protocol: release
build, default renderer, interactive desktop, 15 s warmup, 60 s window, S1 to S5, and
Linux X11.

Decision (orchestrator, 18 November 2026): the PERF-F2 budget is amended to at most
5/s while the window is inactive or no text input has focus. With the window active,
the remaining wakeups (about 9.7/s, down from about 80/s) are the caret blink of the
focused type-to-search input (Slint's text cursor blinker, about 5.7/s) and the 250
ms drag safety net (4/s). An idle focus scope that forwards the first key to the
search input would remove the blink but must keep IME composition start, so it stays
in the backlog (UI-PERF-01). The explorer owns the process's single winit
application handler (`native_events::Observer`); any other event-loop consumer is
composed into it.

## Method

`me` reads `MIDNIGHT_IDLE_WAKEUP_PROBE=<warmup_s>:<seconds>[:thumbnails]`. The probe
uses in-memory default preferences (never the user's file; `thumbnails` selects the
Thumbnails view on both panes), counts winit wakeups by `StartCause` in the explorer's
application handler, waits the warmup after the event loop starts, measures for the
given seconds and prints one line to stderr:

```text
idle_wakeups=N timer_wakeups=T event_wakeups=E seconds=S per_second=R timer_lateness_ms_mean=M timer_lateness_ms_max=X
```

The wakeup that ends the measurement is excluded. Timer lateness is how long after
its requested deadline each timer wakeup ran; small values mean the OS honoured the
requested deadlines.

Only the preferences are isolated. The probe uses the plugins installed next to the
executable and the real preview cache (`%LOCALAPPDATA%\Midnight Explorer\cache\thumbnails\v1`
on Windows, `$XDG_CACHE_HOME/midnight-explorer/thumbnails/v1` on Linux), so set
`LOCALAPPDATA` or `XDG_CACHE_HOME` to a scratch directory for every run, as the harness
below does. The first baseline S2 runs of this record were made before the harness did
so, and stored previews of the fixture images in the real cache.

Harness (not part of the repository): a PowerShell script copied each build's
`_build\debug\me.exe` into a private runtime under the user profile, next to the
`filesystems\native.windows` and `thumbnailers` packages of an existing private
deployment (plugin trust needs a private installation; the checkout drive is not
private), set `LOCALAPPDATA` to a directory of that runtime (the thumbnail cache
follows it), and ran serially, three runs per scenario, median reported:

```powershell
$env:MIDNIGHT_IDLE_WAKEUP_PROBE = '10:30'                       # S2 adds ':thumbnails'
me.exe --software --left <fixture>\left --right <fixture>\right # S2: images-left / -right
```

Fixtures: two quiet local folders of twelve text files and one folder (S1, S4) and
two folders of eight 64x64 PNG images (S2). S4 minimized the window with
`ShowWindow(SW_MINIMIZE)` before the warmup ended. The harness also sampled
`\Thread(me/0)\Context Switches/sec` and the process CPU time over the window.

### Conditions

- The session was locked during every run (`LogonUI` running, no foreground window)
  except the last six (`428ddfa0`, below), which ran after the desktop had been
  unlocked and was in use. Windows throttles processes without visible windows, so
  the harness opted each measured process out of power throttling
  (`SetProcessInformation` with `ProcessPowerThrottling`, execution speed and timer
  resolution, state off); mean timer lateness then stayed below a few milliseconds.
- With the default `femtovg-wgpu` renderer the locked session also stalls the UI
  thread: the baseline build measured 10 to 54 wakeups per second across its nine
  runs, and the 16 ms drag-bridge poll never showed. The software renderer does not
  present through the GPU, so the reported numbers use `--software`; the timers do
  not depend on the renderer. The default-renderer runs are listed below.
- Debug builds (`dev` profile, `CARGO_PROFILE_DEV_DEBUG=0`), the same profile for every
  step, on a machine shared with other agents' builds. Cadence does not depend on the
  profile; CPU percentages do.
- S3 (an open native picker) and S5 (a selected animated thumbnail) need interactive
  input and were not run; their cadences are covered by the virtual-clock tests below.
- Re-measure on an unlocked desktop with the GPU renderer to confirm (same command
  without `--software`).

## Results

Software renderer, unthrottled, 10 s warmup, 30 s window, median of three runs
(wakeups per second; the UI-thread context switches are the median of the
per-run averages). Each step's binary was built from the working tree just before
the named commit was made; the geometry step and `467deea6` were built from their
commits:

| Step | Commit | S1 | S2 | S4 | S1 UI-thread switches/s |
| --- | --- | ---: | ---: | ---: | ---: |
| Baseline (probe only) | `87b04163` | 80.5 | 82.8 | 78.9 | 177 |
| PERF-F1 folder-size pulse | `0a067551` | 78.8 | | | 96.7 |
| PERF-F1 image preparation | `cefa54a2` | 73.7 | | | 87.1 |
| PERF-F1 window geometry | `b364e305` | 74.4 | | | 174.7 |
| PERF-F1 drag bridge and settings picker | `ddff65f8`, `55f61bef` | 19.5 | 19.3 | 19.7 | 32.7 |
| PERF-F2 thumbnail and retry pulses | `a06f343a` | 9.7 | 9.7 | 9.8 | 21.6 |
| UX-F3, WP4-13, geometry settle fix | `467deea6` | 11.0 | 11.0 | 15.0 | 22.8 |

Reading the steps:

- Slint timer deadlines have millisecond resolution and a repeated timer re-arms
  from the time it ran, so the three 100 ms timers (geometry poll, thumbnail pulse,
  retry pulse), started together at startup, ran in the same wakeups. Removing the
  geometry poll alone therefore removed callbacks but no wakeups (73.7 to 74.4,
  within run-to-run spread); removing the other two in PERF-F2 removed that shared
  wakeup (19.5 to 9.7).
- The settings-picker change has no idle effect while no picker is open, so the
  drag-bridge row measures the 16 ms poll becoming the 250 ms safety net.
- UX-F3, WP4-13 and the geometry settle fix add no timer: the `467deea6` S1 and S2
  runs had 282 to 288 timer wakeups per run, as the PERF-F2 runs (282 to 288). Their
  event wakeups were higher, 38 to 53 per run, and so was a re-run of the PERF-F2
  binary at the same time (S1 11.1/s, 283 to 288 timer and 42 to 54 event wakeups
  per run), so the difference is the machine, not the change.
- Two of the three `467deea6` S4 runs had 415 and 470 timer wakeups (the third 288).
  This is consistent with a race in the geometry sampler: a window minimized after
  its first placement sample but before that sample's save was due kept the sampler
  polling every 100 ms while minimized, because a minimized window cannot be
  captured. `428ddfa0` stops the sampler while the window cannot be sampled; its
  test fails without the change (see below).
- Event wakeups (cancelled waits: window messages and cross-thread wakes) were 2 to
  21 per 30 s run in the earlier runs, except the three geometry-step runs (439, 41
  and 55), which overlapped a build started on the same machine. Timer wakeups
  dominate every row.

PERF-F1 acceptance (S1 and S3 at most 26/s): S1 met at 19.5/s; S3 was not measured
(see below). PERF-F2 acceptance (S1, S2 and S4 at most 5/s): **not met** as launched.

### The remaining 5.7 wakeups per second

A diagnostic build (timestamps of every timer wakeup, not committed) showed the
remainder after PERF-F2 as two sources only:

- the drag bridge's 250 ms safety net (4 per second), and
- every 530 ms a burst of three timer wakeups about 17.6 ms apart (5.7 per second).

530 ms is this machine's caret blink time (`GetCaretBlinkTime()`, the Windows
default), which the Slint 1.18.1 winit backend doubles into its cursor flash cycle;
the cursor blinker toggles every half cycle. Keyboard focus in a file pane is the
filename-search `TextInput` (`file-pane.slint` `focus-list()` focuses
`PanelSearch`'s `receiver`), whose caret is 0 px wide while the query is empty.
Slint's `TextCursorBlinker` still toggles its visibility every 530 ms; each toggle
invalidates the item and starts the backend's timer-based frame throttle (one
monitor-refresh interval, about 16.7 ms), which wakes twice more before it stops.
Slint starts the blinker when the input gains focus and whenever its cursor moves,
including `set-selection-offsets` on the unfocused input (`PanelSearch` calls it when
its viewport width or font size changes), and removes it only when the input loses
focus, which includes deactivating the window. In the locked session the window was
never activated, so Slint never delivered that focus loss.

Attribution check: a diagnostic build of `467deea6` (not committed) dispatched
`WindowEvent::WindowActiveChanged(false)` 3 s after start, which is what a real
deactivation delivers. In each S1 run the left pane's filename search had focus
before (`focused-pane` 0) and none after (-1). Three 30 s runs gave 5.2, 5.1 and 5.1
wakeups per second (median 5.1): 118 to 120 timer wakeups per run, exactly the 250 ms
drag safety net (4.0/s), and 34 to 38 event wakeups per run (about 1.2/s). These event
wakeups were more than the 2 to 21 of the earlier runs; the runs overlapped this
batch's own clippy run, and their source was not identified.

So once the window has been deactivated, and as long as no later viewport width or
font change re-arms the blinker, only the 250 ms safety net remains as a timer; while
the window is active with keyboard focus in a file pane, S1, S2 and S4 stay at about
9.7/s. Stopping the blink of the empty filename search is a change outside PERF-F2
(see the batch report).

### Runs of `428ddfa0` on the unlocked desktop

The last runs, of `428ddfa0` (minimized-window sampler fix), started after the desktop
had been unlocked and was in use, so they are not comparable with the rows above and
GUI measurement stopped there:

- S4: 5.0, 8.3 and 9.1 wakeups per second with 120, 121 and 130 timer wakeups per run
  (4.0 to 4.3/s, the drag safety net); minimizing on a live desktop deactivates the
  window, which also stops the caret blinker. The event wakeups (30, 127 and 143)
  came from desktop activity.
- S1: 47.8, 13.5 and 10.9 wakeups per second, with 1092, 249 and 44 event wakeups
  and 341, 156 and 284 timer wakeups; the desktop was in use during the runs.

### Default renderer in the locked session

The same builds without `--software`, median of three. These are not representative
because the locked session stalls GPU presentation (see Conditions):

| Step | S1 | S2 | S4 |
| --- | ---: | ---: | ---: |
| Baseline | 20.2 | 21.1 | 20.9 |
| PERF-F1 folder-size pulse | 30.4 | | |
| PERF-F1 image preparation | 10.8 | | |
| PERF-F1 all five timers | 7.8 | 8.0 | 8.9 |
| PERF-F2 | 5.4 | | |

## Requested idle cadence by construction

| Timer | Base `93d36411` | After, idle | Runs faster only while |
| --- | --- | --- | --- |
| Drag bridge poll (`midnight-ui`) | 16 ms, always | 250 ms safety net | a gesture, hover, cancellation, shutdown, handoff or refused drag command is pending (16 ms); invalidation, refresh and native release, wheel or touch queue one zero-delay republish turn |
| Settings picker check (`midnight-ui`) | 25 ms while a picker is open | none while closed, 1 s while open | a cancellation is pending (25 ms) |
| Folder-size presentation | 250 ms, always | none | a size-only event arms one 250 ms single shot |
| Viewer image preparation | 200 ms, always | none | settings apply (prepared once) and an exit request (revoked once), both on existing callbacks |
| Window geometry sampler | 100 ms, always | none | a native resize, move or scale change of the main window or a restore, until the placement settled and its save was accepted (100 ms); never while the window is minimized or hidden: a pending save, or the retry of a refused one, waits for the next geometry event, such as restoring the window |
| Thumbnail pulse | 100 ms, always | none | a pass reports work: a changed view or scale, polled frames, an animated thumbnail, a hidden window, a refused command or the wait for the cache outcome (100 ms) |
| Visible-sync and retry pass | 100 ms, always | none | a pass reports work: an exit request waiting for live jobs, a hidden window, a refused or flushed send, an unavailable owned scope (100 ms); live jobs without an exit request do not keep it running |

The thumbnail and retry pulses are woken by the main window's `presentation-changed`
callback (refresh, layout and visible-row changes), by refused sends, by native
visibility changes and by the thumbnail service's ready notifier; the geometry sampler
by the explorer's winit application handler. A thumbnail pass wakes the retry pass
only when it moved a pane's row range, which is also that pane's tree viewport.

These three refinements (the minimized-window retry, the exit-only retry pass and
the viewport-only wake) were made after the measurements above. None of them changes
an idle scenario: S1, S2 and S4 have no jobs, no refused save and no moving viewport.

## Per-timer cadence evidence

Virtual-clock tests prove each converted cadence independently of the OS:

- Size presentation: `size_refresh::tests::size_flood_from_both_panes_paints_four_times_per_second_and_idle_wakes_nothing`.
- Image preparation: `viewers::preparation_tests::loaded_settings_prepare_the_spare_once_and_an_exit_request_revokes_it_once`,
  `a_failed_settings_read_prepares_and_revokes_nothing`,
  `dropping_the_preparation_releases_the_service_without_an_event_loop_callback`.
- Window geometry: `window_state::wake_tests::the_sampler_stops_when_quiet_and_runs_after_geometry_events`,
  `a_window_moved_back_to_its_saved_geometry_settles_without_a_save`,
  `a_window_minimized_before_its_save_stops_sampling_until_the_next_geometry_event`,
  `a_refused_save_waits_without_polling_while_the_window_cannot_be_observed`,
  `a_refused_save_stays_pending_until_its_retry_is_accepted`,
  `native_events::tests::only_geometry_events_of_the_bound_main_window_wake_its_samplers`,
  `a_shown_or_hidden_main_window_wakes_once_per_change`. On a desktop (ignored by
  default), `window_state::native_tests::native_moves_and_resizes_reach_the_sampler_through_the_application_handler`
  moves and resizes a real window and expects each placement saved, with only the
  application handler waking the sampler.
- Drag bridge (`midnight-ui`, `drag_bridge::tests`): `an_idle_bridge_polls_slowly_and_a_row_drag_polls_quickly_until_it_ends`,
  `invalidation_republishes_targets_on_the_next_turn_and_unchanged_refreshes_keep_them`,
  `native_release_wheel_and_touch_republish_without_invalidating`,
  `external_hover_is_presented_on_the_next_turn_and_polls_quickly_while_it_lasts`,
  `a_refused_drag_command_keeps_the_quick_retry_until_it_is_sent`,
  `bridges_of_two_windows_on_one_thread_republish_only_their_own_targets`.
- Settings picker (`midnight-ui`, `settings_picker_presentation::tests`):
  `an_open_picker_is_checked_once_a_second_without_cancelling`,
  `an_invalidated_picker_preempts_the_slow_check_and_retries_every_25_ms`,
  `a_borrowed_presenter_retries_the_check_at_the_cancellation_cadence`. These call
  `wake` directly. The Options callbacks that invalidate an open picker (editor and
  Custom Command draft edits, a topic change, Cancel) do not call it yet; that change
  belongs to files outside this batch, and until it lands such a cancellation waits
  for the 1 s check instead of 25 ms.
- Pulses: `pulse::tests::a_pulse_ticks_while_busy_stops_when_quiet_and_wakes_without_postponing`,
  `a_wake_from_inside_a_quiet_tick_runs_one_more_tick`,
  `a_refusal_wakes_every_watching_pulse_and_is_reported_once`;
  `thumbnails::pulse_tests::a_static_view_stops_polling_and_a_presentation_change_runs_one_pass`,
  `an_animated_thumbnail_keeps_the_pulse_until_it_is_deselected`,
  `a_refused_command_icon_view_is_retried_without_another_wake`,
  `refreshes_callbacks_and_layout_changes_invoke_presentation_changed`,
  `the_cache_location_is_published_without_another_wake`,
  `an_unreported_cache_location_keeps_the_pulse_polling`,
  `only_a_pass_that_moves_a_row_range_wakes_the_retry_pass`;
  `retry_pass::tests::a_refused_acknowledgement_keeps_the_pass_running_until_it_is_accepted`,
  `an_unavailable_owned_scope_keeps_the_pass_running`,
  `an_unreported_unavailable_scope_for_the_flushes_or_the_acknowledgements_keeps_the_pass_running`,
  `a_hidden_main_window_closes_viewers_and_keeps_the_pass_running`,
  `a_live_job_without_an_exit_request_leaves_no_work`,
  `an_exit_request_keeps_the_pass_running_until_live_jobs_end_then_hides_the_window`;
  `command_reclaim::tests::refused_owned_sends_and_unavailable_scopes_report_work_for_the_retry_pass`.
- Probe parsing: `idle_probe::tests::the_probe_variable_names_warmup_and_measurement_seconds`,
  `smoke_settings::tests::probe_preferences_are_in_memory_defaults_with_an_optional_thumbnails_view`.

S3 (an open picker: 25 ms before, 1 s after) and S5 (a selected animated thumbnail:
100 ms by design) rely on these tests; neither was measured with the probe.

## Desktop checks of the geometry wake

Run on the unlocked Windows desktop of this record's machine, one test per process
(`me-*.exe <test> --exact --ignored --test-threads=1` from `crates\midnight-explorer`),
on the review follow-up build:

- `window_state::native_tests::native_moves_and_resizes_reach_the_sampler_through_the_application_handler`
  (software renderer): passed in 2.2 s. It saved the first placement (385, 385,
  3015 x 1845), then the moved one (425, 415) and the resized one (2955 x 1805); the
  application handler reported four geometry events. With the handler's geometry
  wake removed (a temporary change, not committed) the same test failed after its
  15 s deadline with only the first placement saved.
- `window_state::native_tests::native_maximized_restore_keeps_normal_bounds_and_repeated_capture_has_no_drift`:
  passed.
- `window_state::startup_native_tests` (`femtovg-wgpu`, DX12):
  `native_first_frame_waits_for_saved_theme_and_monitor_geometry`,
  `native_first_frame_uses_primary_when_saved_monitor_is_removed`,
  `native_first_frame_restores_maximized_state_and_normal_bounds` and
  `native_failed_preferences_show_defaults_without_overwriting_settings`: passed.

No probe measurement was repeated for the review follow-up.
