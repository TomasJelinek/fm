# Scrolling glide speed: 24 November 2026

The owner reported: "Scrolling: gliding seems to slow the scrolling speed. I want you to keep
the scrolling speed and add glide." With Options › Appearance › Scrolling glide on, spinning
the wheel or holding a scroll key covered less ground over time than with glide Off.

## Root causes

All wheel and key glides in the panels, dialogs, viewers and the comparison run through one
model, `midnight_ui_input::WheelMotion` (`wheel_motion.rs`, curve and durations in
`glide.rs`). Measured on a simulated clock against Off (`wheel_motion::speed_tests`), it
slowed scrolling in four ways:

1. **Distance was dropped.** A notch or key during a glide added its distance to what was
   still to come, but the sum was clamped to 2,400 px. A held Page Down (600 px, Windows'
   repeat rate) piles up more than that at Long and Very long: 1.5 s of it moved 19,068 px
   instead of 19,200 at Long and 11,014 px at Very long, 43 % short. A wheel set to one screen
   per notch or a free-spinning wheel can reach the bound too.
2. **Every glide from rest eased in.** Chromium's cubic-bezier(0.42, 0, 0.58, 1) moves about
   1 % of a notch in the first 16 ms frame and half of it only after half the glide (100 ms
   at Default, 400 ms at Very long). Every first notch of a spin, every key press and every
   resumed glide started almost still, so the view trailed Off by most of a glide from the
   first input on. Before 23 November the glide was a quadratic ease-out, which started at
   speed.
3. **A spin was never paced.** Each notch during a glide got the full glide duration for the
   remaining distance (bounded only by 2.5 × remaining ÷ speed), whatever the spin's rate, so
   a spin or a held key trailed Off by the distance a whole glide needs: at Default a spin
   every 33 ms was about 200 px (70 ms of input) behind, a held Down arrow about 90 ms.
4. **A retarget could lose speed.** With the 2.5 bound the curve's start slope needed a first
   control point above 1, which was clamped: the rest restarted at 95 % of the current speed.

Not causes (checked): each notch was added to the target, not to the animated position, and
no notch was coalesced; key repeat never waited for a glide (the scroll keys push at once);
touchpad momentum only adds to the finger's motion, which reaches the content directly at
every stop (`kinetic::tests::a_flick_coasts_on_top_of_the_finger_at_every_glide_stop`).

One behaviour ran the other way: a notch between frames was credited with the time since the
previous frame, so its glide started up to a frame before the input (the motion depended on
the frame rate). The fix starts each glide at its input's own moment.

## The fix

`WheelMotion` (one change for every host):

- **Every input's whole distance** is kept; a bound of 10^9 px only keeps the arithmetic
  finite.
- **Curve:** cubic-bezier(0.42, 1, 0.58, 1), Chromium's time axis without the ease-in. A glide
  leaves at 2.4 times its average speed and eases out to rest without overshoot; a 100 px
  notch moves 19 px on its first 60 Hz frame. Duration: Chromium's (200 ms up to 120 px,
  100 ms from 480 px) times the Scrolling glide scale, unchanged.
- **Input during a glide:** the glide is first brought to the input's moment (what was due
  by then is owed to the next frame), then the rest glides from there over the shortest of
  its own duration, twice the interval since the previous input (at least 50 ms) times the
  scale, and the time in which the current speed covers it on the curve. So the rest never
  starts slower than the glide moved, a spin or held key builds speed and trails Off by
  about twice its interval times the scale, and the last input's glide ends within twice the
  repeat interval (never later than one glide).
- **A glide restarted after waiting for content** (`restart_remaining`, the retained text
  camera at the edge of its pixels) advances its first frame by one 60 Hz frame, not up to
  50 ms: with a curve that leaves at speed, 50 ms would be half of the rest in one step.
- Unchanged: a reverse notch drops the old direction's rest (a deliberate difference from
  Chromium and from Off), keyboard navigation, a scrollbar drag and focus loss still stop a
  glide (the input contract in [scrolling.md](../developer/scrolling.md)), each frame counts
  at most 50 ms, and Off or Windows' Animation effects off move every input at once
  (`push_direct`).

## Measurements

`cargo test -p midnight-ui-input --lib speed_tests -- --ignored --nocapture
--test-threads=1` prints these tables. Inputs arrive 5 ms after a 60 Hz frame; each value is
the distance in pixels after the last frame at or before that time; "settled" is when the
last pixel arrived; "half" and "full" are the longest time from an input until half of it
and all of it had arrived. Before is main at 5eb5c126 with the same test file.

### Ten notches of 100 px, 33 ms apart

| Stop | 50 ms | 100 ms | 150 ms | 200 ms | 250 ms | 300 ms | 350 ms | 400 ms | 500 ms | Settled | Half | Full |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Off | 200 | 300 | 500 | 600 | 800 | 900 | 1000 | 1000 | 1000 | 317 | 15 | 15 |
| Very short, before | 126 | 283 | 431 | 582 | 731 | 882 | 1000 | 1000 | 1000 | 350 | 31 | 48 |
| Very short, after | 198 | 300 | 499 | 600 | 800 | 900 | 1000 | 1000 | 1000 | 333 | 15 | 45 |
| Short, before | 45 | 201 | 371 | 520 | 667 | 817 | 960 | 1000 | 1000 | 400 | 62 | 98 |
| Short, after | 163 | 300 | 476 | 600 | 780 | 900 | 1000 | 1000 | 1000 | 350 | 28 | 48 |
| Default, before | 12 | 75 | 211 | 390 | 566 | 726 | 871 | 971 | 1000 | 467 | 98 | 165 |
| Default, after | 103 | 278 | 435 | 581 | 739 | 883 | 998 | 1000 | 1000 | 367 | 45 | 65 |
| Long, before | 3 | 21 | 75 | 188 | 352 | 539 | 726 | 883 | 999 | 517 | 162 | 215 |
| Long, after | 57 | 204 | 356 | 505 | 659 | 808 | 948 | 998 | 1000 | 433 | 65 | 131 |
| Very long, before | 1 | 5 | 21 | 60 | 123 | 210 | 318 | 445 | 728 | 700 | 315 | 398 |
| Very long, after | 30 | 123 | 241 | 371 | 511 | 654 | 797 | 908 | 996 | 567 | 131 | 265 |

### Page Down (600 px) held for 1.5 s: a press, then Windows' repeat after 500 ms, 30 a second

| Stop | 100 ms | 600 ms | 750 ms | 1000 ms | 1250 ms | 1500 ms | 1600 ms | 1800 ms | Total | Settled |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Off | 600 | 2400 | 5400 | 9600 | 14400 | 18600 | 19200 | 19200 | 19200 | 1517 |
| Very short, before | 600 | 2400 | 5066 | 9600 | 14066 | 18600 | 19200 | 19200 | 19200 | 1533 |
| Very short, after | 600 | 2400 | 5387 | 9600 | 14387 | 18600 | 19200 | 19200 | 19200 | 1533 |
| Short, before | 600 | 2296 | 4983 | 9494 | 13983 | 18494 | 19200 | 19200 | 19200 | 1550 |
| Short, after | 600 | 2399 | 5220 | 9599 | 14220 | 18599 | 19200 | 19200 | 19200 | 1550 |
| Default, before | 597 | 1686 | 4526 | 9017 | 13516 | 18017 | 19200 | 19200 | 19200 | 1600 |
| Default, after | 600 | 2259 | 4981 | 9462 | 13981 | 18462 | 19200 | 19200 | 19200 | 1583 |
| Long, before | 274 | 965 | 3171 | 7872 | 12366 | 16861 | 18529 | 19068 | **19068** | 1700 |
| Long, after | 511 | 1835 | 4506 | 8999 | 13505 | 17999 | 19179 | 19200 | 19200 | 1650 |
| Very long, before | 70 | 699 | 1481 | 3652 | 6106 | 8614 | 9609 | 10903 | **11014** | 1900 |
| Very long, after | 307 | 1360 | 3631 | 8016 | 12509 | 17006 | 18587 | 19200 | 19200 | 1783 |

### Other inputs (after; before in brackets)

| Input | Default at 1000 ms | Very long at 1000 ms | Settled, Default | Settled, Very long |
| --- | ---: | ---: | ---: | ---: |
| Down arrow (28 px) held 1.5 s; Off 448 at 1000 ms, settled 1517 | 442 (374) | 374 (138) | 1583 (1700) | 1767 (2083) |
| 20 notches every 16 ms; Off settled 317 | 2000 (2000) | 2000 (2000) | 367 (433) | 517 (700) |
| 20 notches every 50 ms; Off 2000 at 1000 ms | 1980 (1879) | 1739 (1626) | 1067 (1133) | 1350 (1367) |
| 20 notches every 100 ms; Off 1000 at 1000 ms | 983 (960) | 817 (776) | 2100 (2100) | 2533 (2450) |

At every sampled time the new glide is at least as far as the old one. One case settles
later: notches 100 ms apart at Long and Very long (17 and 83 ms), because those intervals are
too long to pace a glide and Chromium's duration for the small rest after the last notch is
the longest; that last notch still arrives within one glide.

### Tests

`wheel_motion::speed_tests` runs single notches (100 px and one screen), 10 and 40 notches at
16, 33, 50 and 100 ms, a fast upward spin at 150 px, held Down arrow, Page Down and Page Up,
and two of them with input between frames, at every stop, and checks: the same total as Off;
each input's target within one glide and half of it within 60 % of one; nothing still moving
a glide after the last input; no frame moving back or further than Off moves in a frame and
a glide; the same motion at 30, 60 and 144 Hz within a frame; a spin or held key trailing Off
by at most twice its interval times the stop (plus a frame); a notch never slowing a glide
and a notch from rest moving at once. On main at 5eb5c126 five of the six fail (the bounded
jump test passes there too):

```text
every_input_moves_exactly_as_far_as_with_glide_off: held Page Down at x2: 19037.87 px, Off 19200 px
glide_reaches_each_target_within_one_glide_and_half_of_it_well_before: held Page Down at x2: an input's target took inf ms
glide_keeps_its_timing_at_30_60_and_144_hz: held Page Down at x2 (total differs)
a_spin_or_a_held_key_trails_glide_off_by_twice_its_interval_times_the_stop: held Down arrow at x0.5: 75 px at 583 ms, 9 px behind where Off was 50 ms before
a_new_notch_never_slows_a_glide_and_a_notch_from_rest_leaves_at_once: the first frame moved under the glide's average
```

With the fix all pass, with the unit tests of `glide.rs`, `wheel_motion.rs` and
`kinetic_tests.rs`.

### Tests that encoded the old curve

These tests asserted the ease-in or Chromium's per-notch durations, not a requirement, and
were updated to the new curve with the reason in each:

- `midnight-ui-input` `wheel_motion::tests`: a first frame under 2 px and the middle frame
  the fastest (now the first frame moves a sixth and is the fastest), a stall's first frame
  under 10 px (now the curve's first 50 ms), a burst capped at 2,400 px (now all 600,000 px
  arrive), 191.7 ms after a second notch (now 200 ms, the remaining 112.5 px's duration).
- `midnight-viewer-ui` `raster_camera_tests` and `text_presentation_tests`: per-frame step
  bounds sized for the ease-in (10 and 12 px, a sixth of a notch) now allow the curve's
  first frame (12 and 13.5 px, half a notch after 32 ms of backpressure);
  `web_wheel_tests`: a three-notch flick 24 ms apart moves for 13 frames of 8 ms, not 20;
  the page-edge stall check spaces its notches 40 ms apart and the one-second check sends
  notches until frame 123, so that the page's stall, not the paced glide's own end, still
  ends the glide.
- `midnight-compare-ui` `scroll_parity_tests::both_panes_stay_aligned_during_and_after_a_glide`:
  more than 12 moving frames instead of 20 for five notches 24 ms apart.
- `midnight-ui` `scroll_input_pane_tests::real_file_panes_keep_fractional_wheel_positions_during_model_refresh`:
  the first frame (32 ms after the notch) may move 40 % of the notch, not 20 px, and the row
  boundary check runs on every frame between row boundaries, not only above 20 px.
- `midnight-viewer-engine` `worker_prepared_tests`: the co-simulated glide follows the new
  model.

## A prepared-window regression the faster glide exposed, and its fix

With the co-simulated glide updated, `scrolling_inside_the_prepared_window_prepares_nothing_again`
failed: turning back down painted strips the window had already asked for. The test now
runs every glide model of the co-simulation (the current glide, Off, the quadratic glide
before Chromium's and Chromium's ease-in-out) at 100, 125, 150 and 200 % and reports every
turn that prepared anything. Strips painted during the turns back down, three turns each,
without the fix (three runs of the test alone, the same each time):

| Glide | 100 % | 125 % | 150 % | 200 % |
| --- | --- | --- | --- | --- |
| Current wheel glide | 0 (1 in one run inside the suite) | 1 on turn 0 | 2, 1, 1 | 3, 1, 1 |
| Off, quadratic, Chromium's | 0 | 0 | 0 | 0 |

The worker's log (temporary instrumentation of `Bands::travel`, `prepare_strips_turn` and
`evict_strips`, Wheel at 200 %) showed two causes, both in `worker_bands.rs`:

1. **The window shrank before it was painted.** The worker took the travel's speed from each
   camera request alone (`FAST_STEP`: more than a sixteenth of a screen). The new glide's
   requests near the threshold flipped between fast and slow within one gesture: at 223 px
   the travel was fast and the window wanted three screens ahead (strips up to 23); the
   turn painted strips 18 to 20 within its 4 ms (`TURN`); at 308 px the travel was slow, the
   window wanted two screens (strips up to 18), and strips 21 to 23 were never painted at
   rest. The next fast turn down painted them during the gesture.
2. **A slow step dropped what fast travel had prepared.** At rest the worker keeps strips
   within the window's longer side plus a screen in both directions, and that side came
   from the current speed. The turn up ended slow, so the kept side was three screens:
   strip 23, which the fast window had painted, was dropped at 74 px, and every fast turn
   down painted it again.

The fix (`Bands::travel`, `evict_strips`): travel stays fast until it turns, and the kept
side is fast travel's whatever the travel is now. Each alone does not suffice: the kept
side alone left turn 0 (cause 1: 1, 2 and 3 strips at 125, 150 and 200 %); sticky fast
travel alone was worse (2, 2, 3 and 4 strips on every turn at 100 to 200 %, because the
first slow step of a turn shrank the kept side). With both, every model at every scale
prepares nothing during the turns: three runs alone, the whole suite twice, and 18 copies
run six at a time on a loaded machine all passed. The test is back in its strict form (no
warm-up turn, no tolerance) and extended to the four glides and two more scales; it takes
0.86 s alone.

Cost elsewhere, from `measure_prepared_text_scrolling` (48 gestures: wheel, touchpad, Page
Down and scrollbar over four kinds of text at 100 and 200 %; the worker's real time, so the
numbers vary between runs):

| Engine | Runs | Frames that waited for pixels | Strips painted | Megapixels | Worker time | Peak strip memory |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Before | 2 | 120, 114 | 1,742, 1,775 | 966, 989 | 10.6 s, 10.6 s | 61.5 MiB |
| After | 3 | 61, 62, 91 | 1,716, 1,699, 1,689 | 944, 933, 926 | 10.9 s, 10.8 s, 12.2 s | 61.5 MiB |

Fewer strips are painted and fewer frames wait; the strip memory stays within its
budget (the kept side is a limit on what stays, not on what is painted, and a full budget
still evicts to the window). A travel that was once fast keeps preparing three screens
ahead until it turns, also when it goes on slowly: some single gestures paint a few more
strips ahead (twelve notches back up over the 100 MB text at 100 %: 13 instead of 7),
others fewer.

## Test runs (Windows, 24 November 2026, on main 8cd93bc5)

- `cargo test -p midnight-ui-input`: 73 unit tests pass (1 measurement ignored) and 8
  integration tests.
- `midnight-viewer-ui` (245 unit, 36 integration), `midnight-compare-ui` (43 unit, 19
  integration), `midnight-viewer-engine` (657 unit) and `midnight-ui`'s unit tests (1,719)
  pass, run one executable at a time. The viewer's edge-bounce test from 47cacc19
  (`raster_camera::tests::prepared_scroll_reaches_fractional_document_edges_without_bouncing_inward`)
  passes with the new curve, glided and direct. `midnight-ui`'s integration tests pass
  except three `tree_copy` tests that wait for a copy decision for 60 s
  (`archive_nested_folders_copy_and_deselect_through_f5`,
  `native_nested_folders_copy_and_deselect_through_f5`,
  `captured_native_copy_passes_unchecked_directory_links_to_preparation`); they passed on
  5eb5c126, fail the same way when run alone, and this change touches no copy code. The
  viewer engine's `office_real_generated_links_resolve_from_bootstrap_catalog_and_nested_sheets`
  failed in some runs with "Source changed since capture" on its temporary workbook and
  passed in others; it does not scroll.
- `cargo clippy --workspace --all-targets --locked --offline --features
  midnight-acceptance/acceptance -- -D warnings`, `cargo fmt --all -- --check` and
  `scripts\check-source-size.ps1` are clean (with the two-line fix of the viewer examples
  that main 8cd93bc5 needs to build them). A Linux cross-check from Windows stops at
  `yeslogic-fontconfig-sys` (Slint's font backend needs `pkg-config`); the changed code has
  no platform-specific parts.
- Acceptance with the 0.5.2 release's plugins: on 5eb5c126 (run `20261124-201319`)
  `panels_scrolling`, `viewers_scroll` and `viewers_compare` PASS; `journeys_pointer_only`
  failed once at its rubber band from the gutter (a pointer drag; nothing marked) and
  passed on its rerun (`20261124-202533`). On 8cd93bc5 with the engine fix (run
  `20261124-210550`) `viewers_scroll` and `panels_scrolling` PASS; `viewers_scroll` keeps
  its two standing SKIPs (the HTML page's Slint pixel delta and touchpad coasting).

## Remaining limits

- A frame later than 50 ms still advances a glide by 50 ms, so below 20 frames a second a
  glide runs slower than wall-clock time (Off moves on the next frame).
- A reverse notch drops the rest of the old direction, and keyboard navigation, a scrollbar
  drag or focus loss stop a glide: there the total differs from Off by design.
- HTML wheel glides can still end early on a loaded machine (backlog VIEW-09).
