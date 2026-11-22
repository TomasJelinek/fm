# Pixel scrolling

The application must move scrollable content by fractional logical pixels, rather
than selecting or revealing one row for each wheel event. This applies to file
panels, settings lists, Find results, Jobs, menus and viewers. A position trace
alone does not establish smoothness: frame cadence and rendered pixels also
need verification on the native renderer.

GPU acceleration is optional. Retained viewer content uses ordinary clipped Slint
images and continuous transforms, with the same camera/input contract on GPU and
CPU renderers. A lack of a usable GPU must not disable browsing, viewer controls
or file operations. Qualify software output separately; GPU timing cannot stand
in for CPU timing, and a successful startup does not prove every font/glyph case.

## Input contract

- Unphased mouse-wheel line deltas receive a short, interruptible ease-out.
  Only undelivered displacement accumulates; reversal discards the old tail.
- Precise pixel deltas and phased touchpad/touch gestures follow the finger
  without easing. Linux X11 and Wayland report no momentum after the lift, so the
  app synthesises it once, in the shared kinetic scroller below. Slint's own
  Flickable fling on a phased (Wayland) lift would be a second, different one:
  on Linux the shared observer therefore delivers a gesture that began with
  `Started` itself, and X11 finger lines (which Slint would ease like notches), as
  a phase-less wheel, so Slint neither eases nor flings them. Other precise pixels
  keep Slint's own route; Slint captures no such gesture and has no fling to add
  (Winit on Wayland begins a gesture with `Moved` right after a wheel notch). Do
  not add a second inertia animation to an operating-system gesture: Windows
  precision touchpads and macOS deliver their own momentum and get none from us.
- Dragging a scrollbar sets its position directly. It cancels outstanding wheel
  motion; keyboard navigation, content changes and focus loss must also prevent
  movement from an obsolete gesture.
- A wheel over the scrollbar must behave consistently with a wheel over the
  content. It must not change a closed combo box repeatedly by redispatching
  one notch as several selection actions.
- Nested lists and open menus own their input. An animation must stop when its
  original list, tab, directory, document or popup is replaced.

### File panels

File panels use a floating-point viewport offset during the entire gesture, and
every view (Details, Brief, Thumbnails) draws every row the viewport intersects:
a row cut by the panel's top or bottom edge shows its visible part, so no offset
leaves a blank band (`panel-list.slint` `band-visible`). The Thumbnails view also
requests thumbnails for one extra row beyond each edge while that range fits the
panel's share of the thumbnail request budget (`file-pane.slint`).

Details and Brief rows settle on a row boundary once a user scroll has come to
rest: after the wheel glide, the touchpad's fingers and momentum (the system's on
Windows and macOS, the app's on Linux), a held button or touch, and Slint's own
wheel easing or fling have all ended and the list has not moved for 100 ms, it
glides to the nearest row over 200 ms on the wheel glide's ease-out curve. That is
at most half a row; the exact bottom bound stays (its top row may be cut and is
drawn). The window's `scroll-interaction-active` (wheel glide, held buttons,
touches and phased touchpad gestures, momentum) holds the alignment; new input,
a press, a cursor reveal or any programmatic position stops it at once. Windows
reports no touchpad phases, so fingers resting on the pad for longer than the
glide and the idle interval (about 300 ms) count as a rest there; Wayland's phased
gestures hold the alignment until the fingers lift. The thumbnail grid keeps its
pixel position: rounding its tall bands would undo a small gesture. Other lists
and document viewers do not acquire panel row alignment.

A notch over a panel moves the viewers' notch distance: `midnight_ui_input::
notch_pixels_for`, the shared line distance (`LINE_PIXELS`) at Windows' default
three lines per notch, in proportion to other line counts, or one screen (the
list's visible height). A whole notch moves the rows nearest to that distance, at
least one (one thumbnail band), on the text viewer's 200 ms glide, and lands on a
row boundary even from a cut position, so no alignment follows it (`scroll_input.rs`
`wheel_distance`). Fractional lines, such as a Windows precision touchpad's and its
system momentum or a high-resolution wheel's steps, move their share of the notch
distance with the same glide, as in the text viewer; their rest then settles as
above.

The panels read Windows' lines per notch (`SPI_GETWHEELSCROLLLINES`,
`wheel_setting.rs`, as the viewer does) and its animation effects
(`SPI_GETCLIENTAREAANIMATION`, `motion_preference.rs`) with the window's first
native event and whenever it regains the focus. With animation effects off, panel
wheel input moves at once and the row alignment is immediate. Other systems keep
three lines per notch, and their reduced-motion settings are not read yet. The viewers
honour the animation setting too ([wheel distance](#wheel-distance)).

From 19 to 21 November 2026 the Details and Brief views drew whole rows only and
nothing aligned the rest position, so every fractional rest (a notch of 60 px over
22 px rows, any touchpad scroll, a cursor revealed at the bottom edge) left a
blank band of up to one row below the column header or at the list's bottom, and
rows popped in and out at both edges while scrolling.

## Wheel distance

One wheel notch (one winit line) moves content by `midnight_ui_input::notch_pixels_for`:
the shared line distance `LINE_PIXELS`, 100 logical px (Chromium's 100/3 px per line),
at Windows' default of three lines per notch, in proportion to other settings (1 to 100
lines), or one screen (the content's visible height) when Windows scrolls one screen at
a time. It was Slint's own 60 px until 21 November 2026, when the user asked for faster
scrolling with the same feel: the glide's 200 ms ease-out and the momentum decay are
unchanged, so a notch travels 5/3 as far on the same curve. A Windows precision
touchpad reports its finger motion and its system momentum as wheel lines, so its
travel grows by the same factor.

Each host reads the system setting and supplies it; nothing queries it per notch. The
viewer reads `SPI_GETWHEELSCROLLLINES` (`midnight-viewer-ui/src/wheel_setting.rs`)
with the window's first native event and whenever the window gains the focus; the
panels do the same (above). A `NativeScroll` glides notches with the setting its host
gives it (`set_wheel_setting`; three lines until then). Linux and macOS have no such
setting and keep three lines. Touchpad pixels and X11 finger lines are not notches:
they follow the finger, lines at `LINE_PIXELS`.

When the system asks for less motion (Windows: Animation effects off,
`SPI_GETCLIENTAREAANIMATION`), the viewers do not glide. They read the setting through the
panels' `motion_preference` module (`midnight_ui::system_reduces_motion`, which the
application gives the viewer windows with `ViewerWindows::set_motion_preference`), with
each window's first native event and whenever it gains the focus. A wheel notch and a
scroll key then move their whole distance on the next frame in the Text, Source, Markdown
and hex views, over PDF and image tiles and over an HTML page, and the shared
`NativeScroll` synthesises no touchpad momentum (`set_reduce_motion`); a touchpad's own
system momentum stays the system's. Linux and macOS report no such setting yet.

Tests: `midnight-ui-input` `wheel_lines::tests` (the default, proportions, bounds, one
screen per notch); `midnight-viewer-ui` `text_presentation_tests` and `web_wheel_tests`
(a notch glides `image_motion::notch_pixels` in the text and HTML viewers; with less
motion it moves whole on the next frame and a touchpad flick has no momentum) and
`image_key_tests` (a notch and a key move a tiled image at once with less motion).

The acceptance suite runs on Slint's software platform, where winit never runs, so it
reaches this path through `midnight_viewer_ui::acceptance_seams` (feature
`acceptance-seams`, which only `midnight-acceptance` enables): `wheel` and `focus`
hand the window's own native observer the events winit would deliver, the wheel frames
follow by themselves while the window moves, and `override_settings` replaces the lines
per notch and the reduced-motion setting for the test process until its guard drops.
The panels' seam in `midnight-ui` behaves the same way. Tests: `acceptance_seams_tests`
(a seam notch moves the text as a native notch does, frame for frame).

## Touchpad momentum (kinetic scrolling)

`KineticScroll` (`midnight-ui-input/src/kinetic.rs`) is the only momentum model.
It is pure policy (no Winit or Slint types) and is owned per window by
`NativeScroll`, which the main window, viewer, comparator and every dialog already
install. Feeding it is `kinetic_native.rs`:

- Wayland touchpads send phased pixel deltas (`Started`/`Moved`) and `Ended` when
  the fingers lift. X11 (XInput2 smooth scrolling) sends unphased fractional line
  deltas and no lift; a gap of three report intervals (30–60 ms) ends that stream.
  The observer delivers a pixel gesture that began with `Started` and X11 finger
  lines itself. Precise pixels outside such a gesture keep Slint's route; their
  lift still starts the shared momentum.
- Wheel notches keep the eased wheel glide and stop any momentum: whole lines, and
  on X11 the 1/120-line steps of high-resolution wheels (libinput's v120 units, for
  example 8 × 0.125 line per detent). A fractional line that is not a whole number
  of such steps is finger motion, and so is a whole line inside a recent finger
  stream.
- Velocity is the finger's displacement over its newest 50 ms (at least three
  reports spanning at least 16 ms), so a stroke that slows down before the lift
  reports its final speed. Reports dispatched together after a UI stall share one
  timestamp: their speed is unknown and never flings. A finger that rested more
  than 40 ms before lifting has stopped. Speed is capped at 6000 px/s.
- A Wayland lift flings from 250 px/s. On X11 a rest looks exactly like a lift, so
  an unphased stream coasts only after a clear flick: at least 700 px/s, and its
  newest two reports at least three quarters of the stroke's average (not slowing
  to a stop). A deliberate stop at a steady high speed can still coast there.
- Momentum decays exponentially with a 0.8 s time constant, longer than iOS's
  default deceleration (0.5 s) after hands-on feedback: total travel is velocity ×
  0.8 s, so a 2000 px/s flick glides about 1600 px, 71% of it within the first
  second, and then eases out over about four seconds. It ends below 12 px/s, 0.2 px
  per 60 Hz frame, so the dropped 9.6 px crawl is invisible. Panels, viewers and the comparator share these values, so the
  same flick glides as far everywhere.
- Each frame integrates the decay exactly over its real time, so neither travel
  nor speed depends on the refresh rate: a late redraw (a missed vsync, a slow
  debug frame) advances up to 50 ms, and only a longer stall resumes without
  catching up. An X11 stream's first momentum frame covers one 16 ms frame, not
  the gap that ended the stream.
- Any mouse button press or release, touch, key press, new wheel notch, pinch,
  focus loss, pointer exit, a pointer move of more than 16 px, or any modifier
  change stops it at once; new finger motion takes over. While a modifier is held,
  finger motion still reaches the content once per report (Ctrl zooms images) but
  never gets momentum.
- Target: momentum starts only over content that scrolls along it, resolved once
  per fling — the host's `KineticSurface` (viewer content, comparison content) or
  the innermost Flickable under the origin that scrolls along the fling's axis, as
  Slint routes a wheel; an axis that Flickable cannot scroll stops. Anywhere else,
  such as a toolbar spin box that acts once per wheel event, there is none.
- Frames are delivered on the native redraw at the gesture's origin as an
  ordinary wheel: `PointerScrolled` (applied without Slint's own wheel easing) or,
  for the browser surface, a synthetic native wheel through the host's sink, in
  the finger stream's units (pixels, or lines for X11, which the HTML viewer
  converts with the shared line distance, `LINE_PIXELS`, as it does the finger's
  lines).
  Backend-driven viewers therefore see one scroll request per frame.
- Content edges: an axis whose content position (Flickable content offset or the
  surface's reported position) has not moved for 120 ms while momentum pushes it
  stops; content never passes its bounds because the owners clamp. There is no
  bounce. Chromium reports no scroll position, so the HTML surface reports the
  newest page frame presented: momentum stops 120 ms after the page stops painting
  (see [HTML pages](#html-pages)). A surface without a known position (comparator
  image panning) runs out with the decay (4.1 s from 2000 px/s, at most 5 s).

The file panels report momentum as scroll interaction, so nearest-row alignment
waits until it ends. X11 touchpad lines over a panel are delivered directly
(`LINE_PIXELS` per line) instead of being eased like a notch. The comparator
content also glides wheel notches through `NativeScroll::glide_content`, since it
takes pixel deltas without easing of its own; a notch with Ctrl, Alt or Super held
is not glided, because image comparisons zoom once per such wheel event.

`NativeScroll::synthesize_momentum` overrides the platform default; tests use it
so the Linux policy is exercised on every OS.

Tests: `midnight-ui-input` `kinetic::tests` (newest-motion velocity, rest and
unknown spans, reversal, exponential decay with its travel, travel and speed
independent of the frame rate, slow release, delayed frames, cancellation, X11 gap
release without a catch-up jump and clear-flick rule, per-axis edge stall) and
`kinetic_native::tests` (Wayland phases, precise pixels outside a phased gesture,
X11 finger lines versus whole and high-resolution wheel steps, interruptions
including a button release and modifiers, platform default);
`midnight-ui`
`scroll_input::pane_tests::kinetic_tests` (real panels with Slint receiving
unconsumed reports with their phases: single fling without Slint's own, X11 line
stream, high-resolution detent, click, top edge, no momentum outside lists; the
text viewer's test flick frame for frame against the shared decay, a Windows
touchpad stream frame for frame against the viewer's wheel glide, the notch's
curve and row landing, the thumbnail grid's band notch and kept touchpad position,
a modifier, click or notch stopping momentum with only the alignment after it,
reduced motion) and `rows_tests` (the reported mid-folder rest in both panels,
rows covering the list at fractional offsets in each view at three scales and
font sizes, keyboard reveals, refreshes and sorting while scrolled, rows painting
their model state after a flick);
`tests/panel_scroll.rs` (partly visible rows drawn and clickable, band coverage
over offsets, row heights and viewport heights, the idle alignment and what holds,
interrupts or bounds it); `tests/panel_scroll_costs.rs` (a 3000 px/s fling of the
shared model through 50,000 entries in Details and Thumbnails reads each row once
as it enters the view, and the grid's thumbnail target range stays within the
pane's half of the request budget);
`midnight-viewer-ui` `kinetic_tests` (text viewer through the native hook to
backend scroll requests, X11 lines, toolbar) and `web_wheel_tests` (HTML page:
momentum stops where the page stops painting); `midnight-compare-ui` `scroll_tests`
(notch glide, Ctrl+wheel, flick, document end, click, momentum in lines for a
sink, toolbar).

Not verifiable without hardware: the X11 gap and clear-flick rules against real
touchpad and TrackPoint report timing (TrackPoint on-button scrolling cannot be
told apart from a touchpad on X11), how a real flick feels, and momentum from a
real touchpad inside the live HTML viewer (the provider's response to the same
per-frame deltas was measured; see [HTML pages](#html-pages)).

## HTML pages

Chromium scrolls the page and reports no scroll position, so the viewer's web input
(`midnight-viewer-ui/src/web_input.rs`) runs the text viewer's motion and hands it to
the page one frame at a time:

- A wheel notch over the page glides through the shared `WheelMotion` exactly as in
  the text viewer: the viewer's notch distance (`image_motion::notch_pixels`, see
  [Wheel distance](#wheel-distance)) over 200 ms, only undelivered distance adding
  up, a reversal dropping the old remainder. Each native redraw (8 ms wakes)
  sends that frame's share as one physical-pixel `Wheel` command, so the page travels
  as far, at the same scale, over the same time. A command the backend queue refuses
  goes out again on the next frame before the glide's clock moves on.
- The provider starts Chromium with `disable-smooth-scrolling`. CEF's windowless
  wheel events are not precise deltas, and Chromium eases each of them over about
  170 ms (one 120 px event painted as 2, 7, 16, … 120 px over 12 frames); every
  per-frame delta would be eased again. Keyboard scrolling now jumps at once too, like
  the text viewer's.
- Touchpad pixels and X11 finger lines (at `LINE_PIXELS` per line, as in the text
  viewer) reach the page at once, one command per report; Linux momentum arrives
  through the shared kinetic scroller's sink, one command per frame.
- A wheel with Ctrl, Alt or Super acts once: unglided, with the previous 40 px line,
  so Chromium's own Ctrl+wheel zoom (120 units per step) keeps its rate. Shift+wheel
  glides and Chromium scrolls it horizontally.
- Edges: the page paints a frame only when it moves. Once it has painted since a
  glide or touchpad stream began, 120 ms without a newer presented frame ends the
  glide and stops momentum, instead of sending the rest of the deltas into the edge.
  Before that first frame the position is unknown for up to a second: the first
  scroll of a freshly loaded page took Chromium 350–650 ms to paint (4 of 96 measured
  gestures), and a glide or momentum is not cut short meanwhile. A page that has not
  painted a second after the scroll began is taken to be at its edge already. A
  press, key, touch, pinch, focus loss, touchpad motion, an acting wheel, a wheel
  outside the page or an open popup stops the glide at once, as do a new document,
  mode or layout.

Measured on Linux X11 (CEF 156 debug provider with `diagnostic-timing`, driven over
MWV1 without the host, 640 × 400 CSS px of a 30,000 px page, notches 24 ms apart,
medians of four fresh pages; the text viewer's distances come from the real viewer
UI under a test clock, and the new HTML commands were checked equal to them frame by
frame). These runs predate the faster notch of 21 November 2026 (60 → 100 px at the
system's default, [Wheel distance](#wheel-distance)); the page follows the text viewer's
distance, and its timing was not measured again:

| Scale, notches | Text viewer | HTML before: travel, done | HTML after: travel, done | Page behind text viewer (median) |
| --- | --- | --- | --- | --- |
| 1x, 1 | 60 px, 200 ms | 40 px, 172 ms | 60 px, 214 ms | 51 → 9 ms |
| 1x, 3 | 180 px, 248 ms | 120 px, 254 ms | 180 px, 259 ms | 71 → 9 ms |
| 1x, 10 | 600 px, 416 ms | 400 px, 397 ms | 600 px, 433 ms | 83 → 9 ms |
| 2x, 1 | 120 px, 200 ms | 80 px, 194 ms | 120 px, 212 ms | 59 → 16 ms |
| 2x, 3 | 360 px, 248 ms | 240 px, 236 ms | 360 px, 275 ms | 68 → 17 ms |
| 2x, 10 | 1200 px, 416 ms | 800 px, 378 ms | 1200 px, 437 ms | 61 → 15 ms |

Before, the host sent 40 px per notch at once and Chromium eased it; the same per-frame
deltas with Chromium's easing still on lag 90–161 ms. A touchpad stream of 12 px
reports every 10 ms lagged the finger by 130 ms (1x) and 119 ms (2x) before and by 7
and 13 ms after. Chromium keeps up with one delta per frame: at the measurement's
8 ms cadence about two deltas share each painted frame, and the provider's wheel
trace (`wheel_paint_elapsed_us`, wheel dispatch to the next view paint) stays at a
median of 2.3–4.4 ms and at most 17 ms, also on a long text page (3.3 ms median, 8.6 ms
at most); the host therefore sends one delta per redraw without further coalescing.
Ctrl+wheel still zooms to 110% after three notches, a horizontal or Shift wheel moves
120 px sideways, and sixty 1 px events move 60 px. On Windows CEF reads `WHEEL_DELTA`
units scaled by the system's wheel lines; the provider's Windows adapter converts
(plugin README).

Tests: `midnight-viewer-ui` `web_wheel_tests` (one, three and ten notches, a reversal
and a horizontal notch sent frame by frame exactly as the text viewer scrolls, at 1x
and 2x; the glide and momentum stop once the page stops painting, are not cut short
before its first frame, and stop a second after a scroll that never paints began; X11
lines; Ctrl+wheel unchanged); the provider's `native::input` tests (sub-DIP
accumulation, a glide's last DIP despite float rounding, a gesture's rest carried and
dropped at its end, a notch within one wheel unit's scroll at 1–20 lines on Windows)
and the opt-in
`native_runtime` `chromium_applies_each_wheel_event_at_once` (one event scrolls its
whole distance in the next frame and stops at the top; it fails against the old
provider with frames at 2, 7, 16, … 120 px).

## Rendering and bounded work

Native redraw advances the discrete-wheel clock immediately before rendering;
a timer only requests frames. A delayed frame must not consume an arbitrary
amount of animation time and skip all intermediate positions. Pending wheel
displacement is bounded, and direction changes remain responsive.

File lists remain virtualized. Scrolling must not regenerate or format every
entry in a directory. Rendering caches, if enabled, belong to the visible
virtualized cells and must invalidate on selection, metadata, icon and theme
changes. Do not retain a texture for every entry in the directory.

Viewer parsing, shaping and rasterization remain on backend workers. Text,
source, binary and Markdown inputs keep bounded viewport pages; smooth input
is not permission to read or shape the entire file. While a new page is pending,
subsequent movement belongs to that continuation, including reversals and
horizontal movement. A reply must not restore the camera from before those
inputs. Worker backpressure must neither lose the final pixels nor accumulate
an unbounded queue of animation frames.

## Verification

Use real components with a controlled animation clock to check fractional
movement, accumulated distance, reversal, direct pixel input, scrollbar
interruption, nested ownership and idle panel alignment. Also send native
Windows wheel messages to an owned test window and observe completed redraws.
A successful test that merely sees four intermediate positions is not evidence
of a 60 Hz result.

For native measurements, record window size, DPI scale, monitor refresh rate,
renderer/backend policy and build profile. Keep compilation and other probes
out of the timed interval. Use actual directory metadata as well as synthetic
large models, and distinguish a UI fixture from the complete backend-driven
application. Capture selection/theme invalidation and fractional pixel positions
after timing so GPU readback does not perturb the benchmark.

Observe Winit redraw dispatch and the following event-loop callback. Do not use
Slint's rendering notifier for FemtoVG-WGPU timing: it changes flush behavior
and can cause extra shader compilation. Dispatch/submission duration is not GPU
completion or monitor scanout. Report median, p95 and worst frame gaps instead
of claiming smoothness solely from average FPS.

See [native measurements](../NATIVE_SCROLL_VERIFICATION.md) for reproducible
commands, local evidence and remaining performance limitations.

### Acceptance seam

The acceptance suite renders the application in software windows, where no Winit
adapter delivers native events, and the panels take a wheel notch, the system's
lines per notch and its animation setting only from native events. `midnight-ui`
therefore has an acceptance-only seam: the feature `acceptance-seams`, which only
`midnight-acceptance` enables. The application and every other package build
without it (`cargo tree -p midnight-explorer -e features -i midnight-ui` lists no
`acceptance-seams`). Its module `midnight_ui::acceptance_seams` has:

- `wheel(&MainWindow, at, NativeWheel) -> Option<bool>`: a `CursorMoved` to `at`,
  then the `MouseWheel` (`NativeWheel::Lines` for a mouse wheel or a Windows
  precision touchpad, fractions allowed; `NativeWheel::Pixels` with a `NativePhase`
  for a pixel touchpad), handed to the observer closure that `on_winit_window_event`
  calls. `scroll_input::install` stores that closure for the seam, so there is no
  second path. What the observer leaves alone reaches Slint as Winit's backend
  converts it: a pointer move, then a wheel of lines × 60 or physical pixels ÷
  scale, with its phase. It returns whether the observer consumed the wheel, or
  `None` for a hidden window.
- `focus(&MainWindow, bool) -> Option<()>`: `Focused` through the same closure, then
  `WindowActiveChanged` when the window's activation changes.
- `override_settings(SystemSettings { wheel, reduce_motion }) -> SettingsGuard`: the
  lines per notch (`WheelSetting::Lines(n)` or `Page`) and the request for less
  motion that the panels read, for the whole test process until the guard drops.
  The panels read them with their first native event and on each focus gain, as in
  production, so a scenario calls `focus(window, true)` after overriding.

Winit sends `RedrawRequested` before each frame it presents, and the panels advance
their wheel glide and touchpad momentum on it. The seam sends one through the same
closure every wheel frame (8 ms) while scroll interaction lasts and for 250 ms after
its last event, so the harness's ordinary waits see the motion. The observer marks
native shortcuts authoritative only when a real Winit adapter backs the window, so
the harness's Slint key events keep working after seam input.

Tests: `scroll_input::pane_tests::seam_tests` (with the feature: a seam notch moves a
panel frame for frame like the native-observer fixture, one notch per wheel setting
and with reduced motion, focus changes, input left to Slint, hidden windows) and
`acceptance_seams::tests` (nested overrides); without the feature a `compile_fail`
doctest shows that the module does not exist.

## Fluent scrollbar and command-strip input

The main window's `ScrollView` and `ListView` come from
`crates/midnight-ui/ui/scroll-view.slint`, not from the SDK. They wrap the SDK
`Flickable` with one shared scroll bar component that keeps Fluent 1.18.1's
geometry, colors and input behavior; the SDK's own `ScrollView` is inlined with both
bars at every use ([record](../verification/2026-11-18-slint-compile-time.md)). The
bar receives the scroll position one way and reports moves through a callback: a
run-time two-way binding on a `ListView`'s `content-y` would stop the SDK's list
layout from correcting that position. Like Fluent's, its wheel handler moves the
content without emitting `scrolled`. The viewer and comparator windows' own files
still import the SDK widgets; shared controls they reuse, such as `CompactPicker`,
bring the shared scroll view with them.

A bar whose policy is constantly `always-off` is still instantiated with its items at
every use. `VScrollView` and `HScrollView` in the same file are `ScrollView` without
the horizontal or the vertical bar, for sites that never show it, and
`ui/vlist-view.slint` exports a `ListView` over `VScrollView` for lists that never
scroll horizontally (Slint virtualizes only below a component named `ListView`).
These variants and `ScrollStrip` repeat `ScrollView`'s `Flickable` and bar bindings
and change together ([record](../verification/2026-11-19-slint-build-speed.md)).

`midnight-ui-input::NativeScroll` handles the paths that these scroll bars and
Slint Fluent 1.18.1 update immediately: wheel input on scrollbar overlays and the
application's vertical-to-horizontal command-directory strip. Ordinary `ScrollView`
and `ListView` content keeps the SDK's existing wheel and touch physics. The observer
replaces unphased `LineDelta` notches with fractional events on native redraw and,
on Linux, delivers touchpad finger motion itself (see Touchpad momentum); other
precise pixel input and scrollbar dragging retain their own handling. Closed combo boxes do not receive synthetic selection events.

Scrollbar detection checks a concrete, visible, enabled `TouchArea` aligned with
its `Flickable`, using Fluent's pinned 14-logical-pixel overlay geometry. Overflow
alone does not imply a visible bar. Clipping and the active popup scope constrain
the search; subsequent frames validate both the current cursor and the original
dispatch point. Weak item handles let closed components drop during active or
completed motion. The observer also emits the scroll notification omitted by the
SDK scrollbar wheel handler before changing the offset, including native pixel
and phased input that passes through unchanged. Find therefore releases
cursor-following without changing the selected result.

Main-window input gives the existing file-panel observer first refusal and uses
the same notification for its scrollbar wheel input. The
shared observer covers the remaining main-window surfaces, native dialogs and
working tools created by `DialogHost`, the comparator's file chooser and standard
scrollbar controls, and ancillary scrollbar controls in viewer windows. Existing viewer input
observers remain responsible for their content surfaces.

This integration assumes the pinned Winit backend: it does not override
`create_child_window_adapter`, so both standard SDK combo-box popups and custom
popups are embedded and deliver events through the owning native window. If a
future backend creates top-level popup adapters, those adapters must receive the
observer too. `tests/list_scroll.rs` covers real list geometry, nested ownership,
standard and custom dropdowns, both scrollbar axes, 2x DPI, hidden-bar policy,
command remapping, native-pixel cancellation, Find selection ownership and closed
component lifetimes. The panel input tests also cover both file-panel scrollbars
after explicit cursor reveal, preserving the panel's idle row alignment.

## Retained text and document presentation

The native viewer opts into `ViewerFrame.raster` for text, source, binary and
Markdown. The backend keeps the text prepared several screens around the window, as
immutable strips (`ViewerRasterBands`), and the frontend moves them at native redraw
cadence: moving over prepared strips needs no backend reply, no new pixels and no new
texture. Line numbers are separate strips pinned to the left edge, with the same
vertical camera as the body. Text/image mode, search and typography remain
independent. See [the prepared text window](#prepared-text-window) below.

Coordinates use the effective bounded raster viewport, not necessarily the full
physical window: the existing geometry cap can reduce large high-DPI viewports.
The frontend maps each axis using `info.viewport / content_size`. Each strip becomes
one image when it first arrives, and it keeps that image and its row in the window's
strip model while later frames still show it (`raster_presentation.rs`). Rows are only
appended and removed, at heights below a base that moves at most every 32 screens;
`raster-bands-origin` places the base below the page origin. The renderer uploads a
row's texture again whenever that row changes, and every row's when the model is
replaced: replacing the model for each new strip list re-uploaded all shown strips
several times a second. A camera at rest stands on a whole raster pixel, where strips
are exact; old replies cannot pull a newer gesture backward.

Absolute camera requests coalesce without crossing selection or context changes.
The frontend prefetches ahead, pauses at actual retained coverage, and preserves
remaining displacement. Discrete-wheel motion resumes in small steps after a
late refill; precise touchpad deltas accumulate without another inertia curve.
An interruption requests an exact final repaint outside callback borrows.
Selection carries the displayed pixel identity and camera, and only the current
layout can interpret it. Ordinary overlapping page refills preserve bounded
selected text; an invalid new selection cannot copy an older selection silently.

If a bounded page must change its source context or fall back to viewport rendering,
`RasterRebase` first requests the camera actually displayed by the frontend. No
speculative replacement frame is published. The frontend keeps its old pixels and
bounded remaining wheel intent until the matching sticky acknowledgement; a late
reply cannot overwrite a later search, resize or navigation. Discrete motion resumes
with a fresh animation clock, while precise input keeps its native momentum. A
corrective read failure releases the wait without claiming success. Within the
eight-event mailbox, one slot is reserved for the latest frame and one for raster
control, so ordinary event pressure cannot strand either side of this exchange.

A failed refill leaves the current picture visible and stops its pending
motion, while allowing later navigation to recover.

### Prepared text window

A strip is 256 rows of the surface's world in effective raster pixels, starting at a
multiple of 256 (`worker_bands.rs`), as wide as the window's body, or with horizontal
guards of 256 logical pixels on each side while unwrapped lines are wider than the
window. The worker keeps strips one screen against the direction of travel and two
screens along it, three while one camera request moves more than a sixteenth of a
screen (about four screens a second at 60 Hz). It drops strips only a further screen
beyond that window, so turning back finds them. A frame publishes the contiguous run
of strips around the camera; that run is the camera's coverage, within the page's
known limits.

The strips under the window and one beyond each of its edges are painted before a
frame is published; the rest are painted between inputs, at most 4 ms per worker
turn, nearest first along the direction of travel and then against it. A page holds
the prepared screens and two more along the travel: its request's line budget covers
them, still at most 512 lines and the byte budgets below. A refill starts while the
page holds fewer screens than the strips want along the direction of travel (a quarter
of a screen against it) and keeps a screen against it, so it comes every two screens
of travel and the window moves over prepared rows meanwhile. In the Text, Source and hex
views a camera request at rest (a key, the end of a glide) starts it too, so PageDown
finds the next page read; a Markdown page ends at whole blocks, which a reply may not
pass (`Continuation::seam`), so it reads on only while the camera glides or once the
camera leaves it. A refill, a restyle, a new selection or a new raster context (a
scrollbar position, a seek that waits for its page) keeps every strip whose rows it
shows the same way: each strip keeps a hash of the source text, line starts and world
positions of the rows whose ink can reach it (8 em around it), their colours, styles,
roles, document blocks, the selection over them and the gutter's numbers, and keeps its
pixels when the new page reproduces that hash. A strip at a page's edge lacks the ink of
the rows beyond it, so only a page with the same edge reproduces it; a page with tables
is painted again. A new font, size, scale, margin or Markdown preset paints every strip
again, since a font of the same line height keeps every row where it was. Text is laid
out once per page: rows are shaped again only when a refill joins them into its new
page, or when the width, font or zoom changes. Strip IDs are unique in the process, so
the window keeps each strip's conversion and image by ID across surfaces: a scrollbar
drag whose positions wait for their page paints, converts and uploads nothing again.

Strips of one viewer hold at most 64 MiB (`VIEWER_PREPARED_TEXT_BYTES`) and those of
all viewers 256 MiB (`VIEWER_PREPARED_TEXT_TOTAL_BYTES`), body and gutter together,
counted until the last owner of their pixels drops them (`raster_budget.rs`). The
window keeps one converted copy of the strips it shows. When a viewer's budget is
full, strips beyond the window go first and the window stops growing; the strips
under the window are painted when the budget allows. A text request asks for 12 KiB
(4 KiB for binary) and the rows of the window and its prepared screens, at most 512
lines; the prepared screens do not widen word wrapping or permit whole-file loading. A
provider can stop earlier at a byte budget of its own: the basic and source viewers
send at most about 8 KiB, which for rows of a few hundred bytes is no taller than the
viewport. The engine then reads the contiguous
next range (backwards, the previous one) and joins the replies into one page, up to
the request's line budget and at most 120 KiB (`PAGE_BYTES`, `worker_chain.rs`), so
such rows scroll like short ones. A joined backward page whose first row may be cut
drops that row: a row's tail never shows as a line of its own. An incomplete
byte-limited fragment that is still shorter than a viewport (a row longer than that
bound) temporarily retains the existing viewport renderer rather than inventing
continuous coverage or EOF.

Measured on 21 November 2026, single runs on a loaded machine
([record](../verification/2026-11-21-prepared-text-window.md)). In the timed
co-simulation (`worker_prepared_tests.rs`, release) with the former 60 px notch, a
40-notch wheel spin over a 100 MB file stalled 13 frames before and none after at 1x,
and 31 and none at 2x. The worker's time for the gesture fell from 664 to 29 ms, and
from 908 to 58 ms at 2x. Prepared strips held at most 35 MiB at 1x and 62 MiB at 2x
(the budget is 64 MiB). With the 100 px notch the same spin, about five screens a
second, stalls 3 frames at 1x and 4 at 2x. Unwrapped rows wider than the window still
stall as before (88 of 103 frames for 2400-byte rows, 93 of 109 before): a page of at
most 120 KiB holds too few of them to prepare screens ahead. In the native probe,
frames later than 25 ms within a wheel notch fell from 54 to 6 for Text at 1x, from 68
to 3 at 2x, and from 98 to 4 for Source at 2x. Source at 1x is unresolved: one run
under build load had 25 late frames, against 15 before.

On 22 November the co-simulation's window was made to glide like the real camera, whose
requests are settled only once their wheel motion ends: with it the fast spin stalls no
frame before or after, so its 3 and 4 stalled frames came from the model. Since then a
Source scrollbar drag whose positions wait for their page paints no strip again (5
strips over a 61-frame drag before), and eight PageDowns over 100 MB stall no frame (3
to 6 before) ([follow-up](../verification/2026-11-22-text-scrolling-follow-up.md)). Unwrapped
rows wider than the window still stall: every refill shapes its whole page of at most
120 KiB again for at most a guard (256 px) of travel.

Native GPU allocation stalls are addressed separately by the pinned FemtoVG
[upload cache](../../vendor/README.md). In-flight buffers become reusable only
after successful GPU completion and remapping. The pool is bounded, adds no
blocking wait or extra submission, and keeps input/presentation independent of
source I/O.

See the [implementation and acceptance plan](../superpowers/plans/2026-11-11-retained-viewer-raster.md)
and [viewer measurements](../verification/2026-11-11-viewer-pixel-scroll.md).
Backend/camera tests and a correct transform do not by themselves qualify native
60 Hz performance; sustained refills, idle restart and real rendered pixels
remain part of the measurement gate.
## Scrollbar coverage and small source blocks

A scrollbar position describes the scrollable visual range, excluding the visible
extent. It is not a fraction multiplied directly by the file's total byte length.
The frontend inverts the accepted raster frame's mapping only when the requested
camera is finite and fully covered, with no pending position/context transition.
Covered drags immediately translate retained pixels and send absolute camera
requests; release sends a settled request even when the final position is unchanged.
Uncovered targets use the bounded backend positioning path. They are never clamped
to a cached edge and reported as though the requested position were displayed.

The backend can position within an already shaped page without reading or parsing
again. It checks read authority before canceling a speculative correction and
records the new origin before resetting the raster context. Deferred positions use
the same path. Partial pages use their actual loaded bounds; file length alone
cannot prove complete coverage.

Text/source files no larger than 8 KiB already fit a fixed source block. Their
request may use the existing 512-line ceiling instead of splitting that block
solely because the window is short. Byte budgets remain unchanged; larger files
still request viewport-sized fragments. This does not introduce file-sized memory
allocation or eager loading for large files.

The exact user JSON/XML cases pass wheel and scrollbar checks with both GPU and
software rendering. Some late frame gaps remain, so the result is not a uniform
60 Hz guarantee. See the final qualification in
[viewer verification](../verification/2026-11-11-viewer-pixel-scroll.md).
