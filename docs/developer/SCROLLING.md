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

File panels use a floating-point viewport offset during the entire gesture.
Nearest-row alignment is an idle finishing animation only. It must not run
while a finger, mouse drag or wheel coast is active. One thumbnail-wheel notch
must travel at least one thumbnail band so settling cannot undo the gesture.
Other lists and document viewers do not acquire panel row snapping.

Details and Brief views draw whole rows only. The Thumbnails view draws every row
the viewport intersects, so a row cut by the panel edge shows its tiles, and it
requests thumbnails for one extra row beyond each edge while that range fits the
panel's share of the thumbnail request budget (`panel-list.slint` `partial-bands`,
`file-pane.slint`).

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
  converts with the text viewer's 60 px line, as it does the finger's lines).
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
(Slint's 60 px per line) instead of being eased like a notch. The comparator
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
stream, high-resolution detent, click, top edge, no momentum outside lists);
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
  the text viewer: 60 logical px per line over 200 ms, only undelivered distance
  adding up, a reversal dropping the old remainder. Each native redraw (8 ms wakes)
  sends that frame's share as one physical-pixel `Wheel` command, so the page travels
  as far, at the same scale, over the same time. A command the backend queue refuses
  goes out again on the next frame before the glide's clock moves on.
- The provider starts Chromium with `disable-smooth-scrolling`. CEF's windowless
  wheel events are not precise deltas, and Chromium eases each of them over about
  170 ms (one 120 px event painted as 2, 7, 16, … 120 px over 12 frames); every
  per-frame delta would be eased again. Keyboard scrolling now jumps at once too, like
  the text viewer's.
- Touchpad pixels and X11 finger lines (at 60 px per line, as in the text viewer)
  reach the page at once, one command per report; Linux momentum arrives through the
  shared kinetic scroller's sink, one command per frame.
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
frame):

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
Markdown. The backend paints a viewport plus guards of up to 256 raster pixels;
the frontend translates those immutable pixels at native redraw cadence. Line
numbers have a separate image pinned to the left edge, with the same vertical
camera as the body. Text/image mode, search and typography remain independent.

Coordinates use the effective bounded raster viewport, not necessarily the full
physical window: the existing geometry cap can reduce large high-DPI viewports.
The frontend maps each axis using `info.viewport / content_size`. It reuses the
uploaded body and gutter while their pixel identity is unchanged. A matching
latest settled acknowledgement restores the backend's exact fractional glyph
phase; old replies cannot pull a newer gesture backward.

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

A failed guard load leaves the current picture visible and stops its pending
motion, while allowing later navigation to recover.

Body and gutter together are capped at 6 Mi pixels (24 MiB RGBA) per snapshot.
Shared admission permits at most three snapshots per worker and twelve across a
Service, including closed workers awaiting retirement. This caps retained CPU
raster storage at 288 MiB, separately from UI/GPU images and layout caches; each
snapshot is painted in its shared allocation, so building one adds no second
snapshot-sized buffer. Dead weak allocation owners are reaped off the UI
thread. A text request asks for 12 KiB (4 KiB for binary) and the rows of the
viewport and its guards, at most 512 lines; guards do not widen word wrapping or
permit whole-file loading. A provider can stop earlier at a byte budget of its own:
the basic and source viewers send at most about 8 KiB, which for rows of a few
hundred bytes is no taller than the viewport. The engine then reads the contiguous
next range (backwards, the previous one) and joins the replies into one page, up to
the request's line budget and at most 64 KiB (`worker_chain.rs`), so such rows scroll
with full guards like short ones. A joined backward page whose first row may be cut
drops that row: a row's tail never shows as a line of its own. An incomplete
byte-limited fragment that is still shorter than a viewport (a row longer than that
bound) temporarily retains the existing viewport renderer rather than inventing
continuous coverage or EOF.

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
