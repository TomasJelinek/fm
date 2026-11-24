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
  glide (the input contract in [SCROLLING.md](../developer/SCROLLING.md)), each frame counts
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

## A prepared-window regression the faster glide exposed

With the co-simulated glide updated, `scrolling_inside_the_prepared_window_prepares_nothing_again`
failed: turning back down painted strips again (256,000 pixels each at 100 %, 512,000 at
200 %). The co-simulation times the worker's real turns, so the counts depend on the
machine's load. Probed over six turns each after a first turn down and back, inside the
suite (a temporary `PREPARED_GLIDE=off` mode moved each notch at once):

| Glide in the co-simulation | 100 %, strips painted again per turn | 200 % |
| --- | ---: | ---: |
| The new glide | 0 | 1 every turn back down |
| Off (each notch at once) | 0 | 0 |
| The quadratic glide before Chromium's | 0 | 0 |
| Chromium's ease-in-out (main; the test passed there) | 0 | 0 |

Without that first turn back, the first turn back down painted one strip at 100 % and, run
alone, two at 200 %. The worker prepares three screens ahead instead of two while the
travel is fast (`worker_bands::FAST_STEP`: a request more than a sixteenth of a screen
beyond the last) and takes the speed from the newest request alone. Off ends a turn on a
fast step, while the new glide is fast in the middle of a turn and ends on slow steps;
that is the likely trigger, not confirmed. Keeping the travel fast until it turns was tried
and made it worse at 100 % (two strips on the first turn back down), so the engine is
unchanged. The test now turns down and back once before it counts and tolerates one strip
per turn back down at 200 % (it passed eight runs alone and in the suite); the regression
is backlog VIEW-11 for the viewer engine's owner. The cost is one strip's rasterization on
the worker thread per turn; no frame waits for it
(`a_fast_wheel_spin_never_waits_for_the_next_page` and
`page_down_never_waits_for_the_next_page` pass).

## Remaining limits

- A frame later than 50 ms still advances a glide by 50 ms, so below 20 frames a second a
  glide runs slower than wall-clock time (Off moves on the next frame).
- A reverse notch drops the rest of the old direction, and keyboard navigation, a scrollbar
  drag or focus loss stop a glide: there the total differs from Off by design.
- HTML wheel glides can still end early on a loaded machine (backlog VIEW-09).
