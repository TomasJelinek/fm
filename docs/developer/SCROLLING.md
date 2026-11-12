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
- Precise pixel deltas and phased touchpad/touch gestures retain native handling.
  Do not add a second inertia animation to an operating-system gesture.
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

`midnight-ui-input::NativeScroll` handles the paths that Slint Fluent 1.18.1
updates immediately: wheel input on scrollbar overlays and the application's
vertical-to-horizontal command-directory strip. Ordinary `ScrollView` and
`ListView` content keeps the SDK's existing wheel and touch physics. The observer
replaces only unphased `LineDelta` input with fractional events on native redraw;
precise pixel input, phased gestures and scrollbar dragging retain their own
handling. Closed combo boxes do not receive synthetic selection events.

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
raster storage at 288 MiB, separately from bounded construction copies, UI/GPU
images and layout caches. Dead weak allocation owners are reaped off the UI
thread. Text decoding remains capped at 12 KiB (4 KiB for binary) and 512 lines;
guards do not widen word wrapping or permit whole-file loading. An incomplete
byte-limited fragment shorter than a viewport temporarily retains the existing
viewport renderer rather than inventing continuous coverage or EOF.

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
