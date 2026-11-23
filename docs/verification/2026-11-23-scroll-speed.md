# Scroll speed and Source stutter: 23 November 2026

The owner reported that scrolling in every viewer (text, Source, HTML, PDF and the others)
was still too slow, asked for the speed of Electron applications such as Claude or Slack,
and still felt stutter in the Source view of a mid-sized C++ file of theirs. That file is
not copied anywhere; only aggregate numbers were read from it: 75,881 bytes, 1,556 rows,
ASCII, LF line ends, no tabs, rows of 48 bytes on average (median 56, 90th percentile 102,
longest 120), 232 empty rows, about 200 short functions whose parameter lists continue on
rows aligned under their parenthesis. The tests use synthetic C++ of the same shape
(`worker_source_scroll_tests::api_fixture`: 77,902 bytes, 1,492 rows, median row 65 bytes,
90th percentile 111, longest 118, 231 empty rows).

## Chromium's wheel scrolling

Electron applications scroll exactly as Chromium does. The numbers below are from
Chromium's source at `a76a871bd7e3c505a2d88d2f52c25e0ce89ee300` (main, 22 November 2026).

| What | Chromium | Source |
| --- | --- | --- |
| Windows, one notch | ticks × lines to scroll at a time (`SPI_GETWHEELSCROLLLINES`) × 100/3 DIP: 33.3 DIP at one line, 100 at three (the default), 200 at six | [`web_input_event_builders_win.cc`](https://chromium.googlesource.com/chromium/src/+/a76a871bd7e3c505a2d88d2f52c25e0ce89ee300/ui/events/blink/web_input_event_builders_win.cc) (`kScrollbarPixelsPerLine`, lines 297–345) |
| Windows, one screen at a time (`WHEEL_PAGESCROLL`) | a page: 0.875 × the smaller of scroller and viewport | same file, line 312; [`scroll_utils.h`](https://chromium.googlesource.com/chromium/src/+/a76a871bd7e3c505a2d88d2f52c25e0ce89ee300/cc/input/scroll_utils.h) (`kMinFractionToStepWhenPaging`); [`input_handler.cc`](https://chromium.googlesource.com/chromium/src/+/a76a871bd7e3c505a2d88d2f52c25e0ce89ee300/cc/input/input_handler.cc) (`ResolveScrollGranularityToPixels`) |
| Windows, tilt wheel | ticks × `SPI_GETWHEELSCROLLCHARS` (default 1) × 100/3 DIP | `web_input_event_builders_win.cc`, lines 301–306 |
| Linux X11, one notch | 120 DIP (`kWheelScrollAmount`; 53 before crbug.com/1270089), no system setting | [`events_x_utils.cc`](https://chromium.googlesource.com/chromium/src/+/a76a871bd7e3c505a2d88d2f52c25e0ce89ee300/ui/events/x/events_x_utils.cc), line 39 |
| Linux Wayland, one notch | 120 DIP (`value120`, or an axis value ÷ 10 × 120) | [`wayland_pointer.cc`](https://chromium.googlesource.com/chromium/src/+/a76a871bd7e3c505a2d88d2f52c25e0ce89ee300/ui/ozone/platform/wayland/host/wayland_pointer.cc), lines 209 and 263 |
| Display scale | deltas are DIPs; the browser scales them by the device scale factor for the renderer, so a notch moves the same distance on screen at every scale | [`input_router_impl.cc`](https://chromium.googlesource.com/chromium/src/+/a76a871bd7e3c505a2d88d2f52c25e0ce89ee300/components/input/input_router_impl.cc), line 675; [`blink_event_util.cc`](https://chromium.googlesource.com/chromium/src/+/a76a871bd7e3c505a2d88d2f52c25e0ce89ee300/ui/events/blink/blink_event_util.cc) (`ScaleWebInputEvent`) |
| Glide curve | wheel scrolls (`ScrollType::kMouseWheel`) take `DurationBehavior::kInverseDelta` on cubic-bezier(0.42, 0, 0.58, 1) | [`scroll_offset_animation_curve_factory.cc`](https://chromium.googlesource.com/chromium/src/+/a76a871bd7e3c505a2d88d2f52c25e0ce89ee300/cc/animation/scroll_offset_animation_curve_factory.cc); [`scroll_offset_animation_curve.cc`](https://chromium.googlesource.com/chromium/src/+/a76a871bd7e3c505a2d88d2f52c25e0ce89ee300/cc/animation/scroll_offset_animation_curve.cc) |
| Glide duration | clamp(14 − Δ/60, 6, 12) frames of 1/60 s: 200 ms up to 120 px, 150 ms at 300 px, 100 ms from 480 px | `scroll_offset_animation_curve.cc`, lines 44–56 and `EaseInOutSegmentDuration` |
| Successive notches | new target = old target + Δ; the rest glides for min(inverse-delta duration, 2.5 × remaining ÷ current speed), its curve's first control point raised (y1 = 0.42 × slope) to keep the current speed; wall-clock time | `scroll_offset_animation_curve.cc` (`UpdateTarget`, `VelocityBasedDurationBound`, `EaseInOutWithInitialSlope`); [`scroll_offset_animations_impl.cc`](https://chromium.googlesource.com/chromium/src/+/a76a871bd7e3c505a2d88d2f52c25e0ce89ee300/cc/animation/scroll_offset_animations_impl.cc) (`ScrollAnimationUpdateTarget`) |
| Less motion | Windows: no scroll animation while "Animation effects" (`SPI_GETCLIENTAREAANIMATION`) is off | [`animation_win.cc`](https://chromium.googlesource.com/chromium/src/+/a76a871bd7e3c505a2d88d2f52c25e0ce89ee300/ui/gfx/animation/animation_win.cc) |

Chromium computes the duration from device pixels (the renderer receives scaled deltas), so
at 200 % a single 100 DIP notch glides 200 device pixels for about 178 ms.

## Midnight Explorer before and after

Before, every host (panels, text, Source, Markdown, hex, PDF, image, comparison, HTML)
moved `notch_pixels_for`: 100 logical px per notch at three lines on every platform, n ×
100/3 at n lines, a whole screen with one screen per notch, over a 200 ms quadratic ease-out
that restarted for the whole remaining distance at each notch, each presented frame
advancing it by at most 16 ms. A 60 Hz frame is 16.7 ms, so even an undisturbed notch took
208 ms, and every late frame stretched the glide: at 30 frames a second a notch took 400 ms.

After (`midnight-ui-input` `wheel_lines.rs`, `glide.rs`, `wheel_motion.rs`):

| | Before | After (Chromium) |
| --- | --- | --- |
| Windows, three lines | 100 px | 100 px |
| Windows, one and six lines | 33.3 px, 200 px | 33.3 px, 200 px |
| Windows, one screen | 100 % of the view | 87.5 % of the view |
| Linux | 100 px | 120 px |
| One notch's glide | 200 ms ease-out, ≥ 208 ms at 60 Hz, 400 ms at 30 Hz | 200 ms ease-in-out at 60 and 30 Hz |
| Several notches | every notch restarts 200 ms over the whole rest | the rest keeps its speed; 150 ms at 300 px, 100 ms from 480 px |
| A late frame | advances 16 ms | advances its real time up to 50 ms |
| HTML page | the host's glide, sent per frame in DIPs | the same, so Chromium's own distance and curve |

The glide scale (`glide_scale`, default 1.0) multiplies the duration and touchpad momentum's
decay time for the Options page; touchpad pixels keep their 1:1 mapping.

## Where the Source view lost frames

### Engine co-simulation

`measure_source_api_scrolling` (`worker_source_scroll_tests.rs`) runs the timed
co-simulation of `worker_prepared_tests.rs` (the real worker, a simulated Source viewer, the
window's retained camera at 60 Hz, the worker's own wall time as the cost of each turn) over
the synthetic file and, through `SOURCE_SCROLL_FILE`, the owner's file (aggregate output
only). Gestures: the prepared-window set (12 notches 50 ms apart, 40 notches 25 ms apart, a
touchpad flick, eight PageDowns 120 ms apart, 12 notches back up, a scrollbar drag) and,
at Chromium's speed, a steady wheel of 20 notches a second for three seconds down and up, a
spin of 60 a second for a second down and up, and scrollbar drags over the whole file in a
second, there and back. Release build, 1312 × 720 at 1x and 2624 × 1440 at 2x, notch 100
logical px. "Stalls" are frames on which the camera had travel left and could not move.

The whole file is coloured as read (it is below the 256 KiB whole-document preparation): no
gesture prepared, refreshed or showed the notice. With the plugin answering a page one
frame after the request, no gesture stalled a frame, before or after, on either file:

| File, scale | Before: stalls, worker turns p95 / max | After: stalls, worker turns p95 / max |
| --- | --- | --- |
| synthetic, 1x | 0, 3.6 / 23.0 ms | 0, 3.6 / 22.2 ms |
| synthetic, 2x | 0, 5.3 / 32.3 ms | 0, 5.6 / 39.5 ms |
| owner's file, 1x | 0, 2.7 / 18.2 ms | 0, 3.8 / 31.4 ms |
| owner's file, 2x | 0, 4.9 / 27.5 ms | 0, 6.3 / 43.5 ms |

"Before" is the base engine with the glide before this change (`PREPARED_GLIDE=quadratic`,
which the co-simulation keeps for comparisons). The worker runs on its own thread: its turns
delay pixels, not frames, and the prepared strips kept the camera covered. A slower plugin
(`SOURCE_SCROLL_DELAY`) left the wheel unaffected up to eight frames (133 ms) per reply, at
Chromium's spin of 60 notches a second too; only eight PageDowns 120 ms apart stalled 3 to 4
frames at eight frames' latency, with either glide. The page now holds three laid-out
screens beyond the strips instead of two (`PAGE_EXTRA`, text only, no pixels): at eight frames' latency the eight PageDowns stalled 3, 4, 3 and 4 frames before (synthetic
1x and 2x, owner's file 1x and 2x) and 0, 2, 0 and 2 after.

`measure_prepared_text_scrolling` (its own files, plugin one frame) stalled no frame before
or after except over unwrapped 2400-byte rows, a known limit of row groups: 130 stalled
frames over its six gestures at 1x and 146 at 2x before, 40 and 119 after.
`measure_handoffs_after_scrollbar_positions` found no handoff in any of its 36 cases, before
or after.

The prepared window's bounds: strips one screen against the travel and two along it (three
while fast), at most 64 MiB per viewer and 256 MiB for all viewers (unchanged); a page of six
screens of text (seven while fast), still at most 512 lines and 120 KiB per request.

### Native frames

`performance_probe --wheel --sustained --d3d --file=<synthetic file>` (the probe in
`crates/midnight-viewer-ui/examples`) opened the synthetic file in a real viewer window in
the Source view and posted 96 notches (`WM_MOUSEWHEEL`, ±120) 50 ms apart, 48 down and 48
up (`MIDNIGHT_VIEWER_PROBE_WHEEL_MS=50`), at `SLINT_SCALE_FACTOR` 1 and 2, three rounds
each, alternating builds, on the shared Windows 11 machine under the desktop lease. Probes
in release; the basic and Source viewer packages were release packages of main `05710878`
(the Source viewer of `ca70e1af` colours a few more token kinds), staged with the probe in a
new folder under `%LOCALAPPDATA%` and run with `MIDNIGHT_VIEWER_PROBE_EXPLICIT_RUNTIME=1`;
the folder was removed afterwards. Gaps are between consecutive presented frames of one
notch (the probe's `wheel_frame` rows), over all three rounds:

| Scale | Build | Gaps | p50 / p95 / p99 / max ms | Over 25 ms |
| --- | --- | --- | --- | --- |
| 1x | before (`ca70e1af`) | 621 | 16.5 / 20.0 / 25.4 / 50.3 | 7 (1.1 %) |
| 1x | after | 630 | 16.5 / 22.1 / 25.7 / 45.5 | 10 (1.6 %) |
| 1x | strips as four images (rejected) | 621 | 16.3 / 22.2 / 35.3 / 48.7 | 16 (2.6 %) |
| 2x | before | 618 | 16.3 / 21.7 / 28.2 / 39.5 | 14 (2.3 %) |
| 2x | after | 647 | 16.5 / 21.8 / 27.5 / 88.6 | 13 (2.0 %) |
| 2x | strips as four images (rejected) | 600 | 16.6 / 28.4 / 63.1 / 594.7 | 37 (6.2 %) |

Frame pacing is the same before and after within the spread of the rounds (single maxima
of 50 to 89 ms occur in both). The change of speed is in the glide, not in the frames: at
20 notches a second the old glide trailed the wheel by about 2.3 notches (computed from its curve) and every late
frame slowed it, while the new one keeps its speed and runs on the frames' real time.

The strips' textures are uploaded when the renderer first draws them, in the frame a strip
scrolls into view. Cutting each strip into four images of 64 rows, so that such a frame
uploads a quarter of a strip, made frames worse, most at 2x (more items and images per
strip); it was reverted. The pinned Slint FemtoVG renderer keys a pixel-buffer image's
texture by the item that draws it and skips items outside the clip, so uploading a strip
before it is visible needs a renderer change; that is open.

## Tests

New and changed tests, headless (Windows, debug build, each test executable from its
package folder with `--test-threads 4`):

- `midnight-ui-input`: 61 passed, integration 8 passed. New: `wheel_lines::tests` (one,
  three and six lines; one screen per notch, 87.5 %; Linux's fixed 120 px and the platform
  default), `glide::tests` (durations by distance and scale, the curve, a carried speed, the
  thread's scale), `wheel_motion::tests` (a notch's 200 ms at 60 Hz in 12 frames, 300 px in
  150 ms and 480 px in 100 ms, a notch during a glide keeping its speed for Chromium's
  duration or the speed bound, 30 Hz frames and a one-second stall, scale 0 and 2) and
  `kinetic_tests::the_glide_scale_lengthens_momentum_and_zero_starts_none`.
- `midnight-viewer-ui`: 237 passed, integration 36 passed;
  `raster_camera_tests::first_redraw_after_refill_*` now lets the eased-in glide reach the
  coverage edge in five frames before the refill.
- `midnight-viewer-engine`: 629 passed, 20 ignored (measurements). New: the synthetic
  fixture's shape, `measure_source_api_scrolling` (ignored), the prepared window's bounds
  (`worker_bands::prefetch_tests`); `page_keys_read_only_the_rows_past_the_shown_page` allows
  the larger page (48 reads and 340 KiB for eight keys over 500-byte rows, measured 43 and
  319 KiB).
- `midnight-ui`: 1593 passed; integration 1084 passed, 2 failed: the two
  `tree_user_command_process` native tests need the user-command probe program, which this
  export had not built ("program not found"); they do not touch scrolling.
- `midnight-compare-ui`: 34 passed, integration 19 passed.
- The acceptance expectations (`panels_scrolling`: 87.5 % per screen and Linux's 120 px;
  `viewers_scroll`: 87.5 % of a screen per notch) compile under Clippy; the suite was not
  run.

Checks: `cargo clippy --workspace --all-targets --locked --features
midnight-acceptance/acceptance -- -D warnings` passed; `cargo fmt --all -- --check` passed;
`scripts/check-source-size.ps1` passed (3,318 files). A Linux cross-Clippy of the changed
crates cannot run from Windows (`yeslogic-fontconfig-sys` needs a cross pkg-config), and
nothing ran on Linux: the 120 px notch is checked by unit tests only.
