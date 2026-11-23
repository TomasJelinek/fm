# Comparison scrolling like the text viewer: 23 November 2026

The owner reported that the text comparison scrolls differently from the text viewer
(speed, glide) and asked for the same scrolling as any other text viewer. Both windows
already used the shared wheel glide (`midnight_ui_input::WheelMotion` through
`NativeScroll`), but the comparison handed every glide frame to its worker, which moved
its text by whole rows and painted a new frame for each step. This record measures both
before and after the change; the design is in
[file comparison](../developer/FILE_COMPARISON.md#scrolling-text-and-bytes).

## Method

`midnight-compare-ui` `scroll_parity_tests` opens the real comparison window on Slint's
software platform with its own engine and worker, two text files of 3,000 lines, a
900 × 600 window at scale 1 and 14 px monospace (rows of 23 px), and a controlled clock.
It sends native events to the window's own `NativeScroll` (wheel lines, touchpad pixels
with their phases), presses the scroll keys through the window's key handler and moves the
position bar. Every 8 ms frame (`WHEEL_FRAME`) it records:

- the **expected** offset: what the text viewer shows for the same input. The text viewer
  moves its text by exactly what the shared machinery delivers (`raster_camera`): each glide
  frame's step, each touchpad and momentum delta, a scroll key's whole distance at once
  (28 px a line, 40 px a column, the content height a page), stopping at the text's edges.
  The harness sums what the window's shared `NativeScroll` delivered and adds the viewer's
  key distances;
- the **shown** offset: where the compared text stands on screen (the position bar's
  fraction of the scrollable extent, and the shown horizontal offset).

For the old window the harness waits on every frame until the worker has painted what it
was sent: a worker that keeps up with every frame, the old design's best case.

Commands (Windows 11, debug test build):

```text
cargo test -p midnight-compare-ui --lib measure_scroll_parity -- --ignored --nocapture
cargo test -p midnight-compare-ui --lib measure_large_scroll_costs -- --ignored --nocapture
cargo test -p midnight-compare-ui --lib scroll_parity
```

The "before" rows ran the same harness on the base commit (the shown horizontal offset did
not exist there, so the horizontal rows are not measured before).

## Offsets

Largest and final difference between shown and expected offset, in logical pixels, and how
the shown text moved:

| Input | Before: largest / final | Before: moving frames, largest step | After: largest / final | After: moving frames, largest step |
| --- | --- | --- | --- | --- |
| One notch (100 px) | 27.0 / 8.0 | 4 frames, 23 px (rows) | 0.4 / 0.0 | 23 frames, 7 px |
| Five quick notches | 46.5 / 6.0 | 11, 69 px | 0.8 / 0.0 | 26, 39 px |
| Three notches, one back mid-glide | 35.3 / 23.0 | 6, 46 px | 0.5 / 0.0 | 31, 19 px |
| Scrolling glide Off, two notches | 16.0 / 16.0 | 2, 92 px | 0.0 / 0.0 | 2, 100 px |
| Scrolling glide Very long | 13.5 / 7.0 | 9, 23 px | 0.5 / 0.0 | 84, 4 px |
| Scrolling speed Very slow | 16.8 / 8.0 | 4, 23 px | 0.5 / 0.0 | 31, 5 px |
| Scrolling speed Very fast | 31.0 / 9.0 | 11, 46 px | 0.6 / 0.0 | 28, 26 px |
| Touchpad flick and momentum | 21.9 / 11.5 | 39, 138 px | 0.7 / 0.5 | 90, 136 px (the finger's first frame) |
| Down three times | 15.0 / 15.0 (23 px rows) | 3, 23 px | 0.0 / 0.0 | 3, 28 px |
| Page Down twice | 12.0 / 12.0 (whole rows) | 2, 483 px | 0.0 / 0.0 | 2, 489 px (the view) |
| Horizontal notch, twice | not measured | – | 0.7 / 0.0 | 32, 11 px |
| Right twice (40 px each) | not measured | – | 0.0 / 0.0 | 2, 40 px |
| Position bar at 0.1, 0.1003, 0.25 | 20.7 / 17.3 (whole rows) | 3 | 0.4 / 0.2 (to the pixel) | 3 |

Before, a notch's 100 px arrived as four 23 px row jumps, one frame late, and ended 8 px
short (the rest waited, unseen, for the next scroll); Down moved one row (23 px) and Page
Down the whole visible rows (483 px), where the text viewer moves 28 px and the view
(489 px). After, every frame's offset is within 1 px of the text viewer's. The parity tests
(`wheel_notches_move_the_text_as_far_and_as_smoothly_as_in_the_text_viewer`,
`the_scrolling_glide_and_speed_settings_apply_as_in_the_text_viewer`,
`touchpad_motion_and_its_momentum_move_the_text_as_in_the_text_viewer`,
`scroll_keys_and_horizontal_scrolling_move_the_text_as_in_the_text_viewer`,
`the_position_bar_moves_the_text_to_the_pixel`,
`both_panes_stay_aligned_during_and_after_a_glide`) fail on the base commit (all six) and
pass after.

## Cost of large comparisons

A million lines of text (26 MB per file) and 100 MiB compared as bytes, a fast spin of five
notches and a touchpad flick, 90 frames each, debug build (software renderer):

| Files, gesture | Worker frames before → after | UI time presenting them before → after (longest one) |
| --- | --- | --- |
| 1,000,000 lines, five notches | 15 → 3 | 43.9 ms (3.1) → 34.8 ms (12.2) |
| 1,000,000 lines, flick | 45 → 4 | 128.0 ms (3.3) → 46.5 ms (12.4) |
| 100 MiB bytes, five notches | 11 → 4 | 32.2 ms (3.0) → 43.3 ms (12.7) |
| 100 MiB bytes, flick | 45 → 4 | 128.9 ms (3.3) → 46.6 ms (13.0) |

The worker now paints a surface three views tall (7.6 MB at this size) a few times per
gesture instead of one view on every frame; per-frame UI work between surfaces is setting
two clip rectangles, with no allocation. Copying a new surface into a Slint image costs
about four times one view's copy (12 ms in this debug build, a frame or two at 125 Hz);
it happens about once per half a view of travel. The software renderer now redraws the
moving text on every frame (as it does for the text viewer), which dominates the debug
build's frame time (18 ms a frame during the flick, 9.5 ms before, when the text moved
only on 45 of 90 frames). No release-build or GPU-renderer frame times were measured.

## Acceptance

Windows run `20261123-211009` (debug, software platform): `viewers_compare` PASS with no
skipped check (scrolling the compared text: Down 28 px, Page Down and 12 Up 373 px, one
native wheel notch through the comparator's new acceptance seam 100 px, a Slint pixel delta
60 px, the position bar at 0.750), `options_comparators` PASS. `viewers_scroll` passed 61 of
62 checks; the one failure was the HTML viewer opening `long.html` ("Chromium transport
stopped: failed to fill whole buffer"), in a path this change does not touch.

## Not verified

How it feels on real hardware (a mouse wheel, a precision touchpad) and frame times in a
release build or on the GPU renderer.
