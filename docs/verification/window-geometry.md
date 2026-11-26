# Window placement and persistent geometry

All text, source, Markdown, binary, image and PDF viewer windows, and the file
comparator, open using the main window's physical client size and outer-frame
position. Maximized and fullscreen state are copied before showing the child.
Where the platform reports absolute origins (Windows, X11), the child's client
origin is compared with the main window's once the window manager has placed it,
and a decoration-sized difference is corrected with one move; see
[Viewer placement over the main window](#viewer-placement-over-the-main-window).
On Wayland the compositor chooses the position.
The minimum client size is 320 × 200 logical pixels. Compact image/PDF and
comparison controls retain page, fit and zoom actions at that size.

Escape is an unconditional viewer/comparator close action. It is intercepted
before native controls and the configurable shortcut resolver, and is also
handled by the Slint capture callback for toolkit input. Search editors and
toolbar controls therefore retain ordinary typing while Escape still closes.
A configured `viewer.close` chord remains an additional close action.

The main window stores physical client size, outer-frame origin, normal bounds,
maximized/fullscreen flags, and monitor name plus desktop bounds in preferences
version 15. Older settings load without inventing geometry. The application
restores on the first 100 ms UI tick after the initial backend settings load
succeeds; initial placement can therefore briefly precede saved placement.
Geometry is never written over an unread or rejected settings file.

Unchanged monitor identities and bounds preserve exact physical coordinates.
When a uniquely named monitor moves or changes size, restoration preserves its
relative offset and clamps the complete window to the available area. If that
monitor is absent or ambiguous, the primary monitor is used; if the backend
cannot identify a primary monitor, the first available monitor is used.
Windows uses the display's work area, excluding the taskbar. Linux uses the
monitor bounds exposed by Winit. Wayland compositors control top-level placement
and may ignore requested positions or withhold the primary display; exact
coordinate restoration cannot be guaranteed there.

Normal bounds are kept separately while maximized/fullscreen. Minimized and
zero-size observations are ignored. Capture uses outer position consistently;
client-to-outer title-bar offsets are never added on successive launches.
UI callbacks capture only native geometry. Preferences serialization and file
writes remain on the existing backend worker. Saves coalesce after 600 ms idle
or 2 seconds of continuous changes, and full command queues are retried every
2 seconds. The newest geometry remains in preferences for the bounded shutdown
flush, which drains event backpressure before enqueueing the final save.

## Focused verification

- `midnight-ui-input/tests/placement.rs`: physical bounds across different DPI
  scales, negative origins, normal/maximized/fullscreen flags, before showing.
- `midnight-viewer-ui/tests/keyboard.rs`: actual manager/search focus with a
  remapped close chord; compact PDF layout at 320 × 200.
- `midnight-compare-ui/tests/keyboard.rs`: configured close and unconditional
  Escape while a real toolbar control has focus; three image modes at 320 × 200.
- `midnight-platform-common/tests/window_geometry.rs`: exact settings round trip,
  legacy migration, malformed bounds, unknown fields and foreign schema rejection.
- Presenter tests: delayed initial reads preserve unrelated saved fields; rejected
  reads cannot publish geometry over unknown settings.
- Explorer policy tests: unchanged monitor coordinates across repeated restores,
  removed-monitor primary work-area fallback and moved/resized monitor clamping.
- Explorer shutdown tests: final geometry save makes progress with both command
  and event queues full.

The native `midnight-ui-input` example `window_placement` reported identical
Windows normal bounds `(180, 150, 720, 460)` and identical maximized bounds
`(-14, -14, 3840, 2241)` for parent and child. These are outer x/y plus client
width/height in physical pixels on the tested desktop.

The ignored Windows explorer test
`native_maximized_restore_keeps_normal_bounds_and_repeated_capture_has_no_drift`
exercises the real main-window controller and window manager. Run it by itself
on an interactive desktop with `--ignored --test-threads=1 --nocapture`.
It passed on the Windows desktop: normal bounds `(180, 150, 720, 460)` survived
maximized startup, restoring to normal, twenty subsequent captures and the final
preferences save without position or size drift.

## Viewer placement over the main window

`WindowPlacement::capture` records the main window's outer-frame origin as Winit
reports it (Windows `GetWindowRect`, including the invisible DWM resize borders;
X11 client origin minus `_NET_FRAME_EXTENTS`) and its client origin and size.
`apply` requests that outer origin and client size for the child before it is shown.
A request and the reported origin agree only when the window manager places a
frame by the same extents that it reports:

- **Windows:** both paths use the window rectangle, so the child opens exactly over
  the main window, with no move.
- **X11, Mutter (GNOME on Xorg) and KWin:** `_NET_FRAME_EXTENTS` and the placement
  gravity both use the visible frame, so the request is exact (Mutter 46 source:
  `update_net_frame_extents` and `adjust_for_gravity` in `src/x11/window-x11.c`).
- **X11, Weston's XWayland manager (WSLg):** the reported extents include a 32 px
  shadow that placement does not, so each request landed 32 px up and to the left.
- **Wayland (GNOME's default session):** a client can neither read nor set a
  top-level position. Winit ignores the request and Mutter places the new window
  with its own policy (first free spot, otherwise cascaded from the work area's
  corner), which is the usual cause of a viewer that appears slightly shifted.
  Size and maximized/fullscreen state still match, and a maximized main window
  gives a maximized, exactly covering viewer. Winit 0.30 has no `xdg_toplevel`
  parent or positioning API, so nothing better is reachable without patching it.

The check after showing uses only the client origins, which are exact on every
backend that reports them; both windows carry the same decorations, so equal client
origins and sizes mean equal frames. It samples every 25 ms for at most 2 s, waits
until X11 reports the frame mapped and two samples agree, and then requests the
outer origin shifted by the client error. Errors over 128 px (a pre-placement
origin, or a manager that chose its own spot) are left alone. The offset is kept for
the UI thread, so later viewers open aligned without moving. Maximized and
fullscreen children are not moved. Unit tests in
`crates/midnight-ui-input/src/placement_tests.rs` model consistent, shadow-including
and Windows frames at scales 1.0, 1.25 and 2.25, plus maximized, fullscreen, Wayland
and headless captures.

Measured with the `midnight-ui-input` examples (client origin and size in physical
pixels, main window at the requested outer origin):

| Desktop | Case | Before | After |
| --- | --- | --- | --- |
| Windows 11, 225 % | normal 900 × 600 at (180, 150), 1201 × 777 at (37, 61) | 0 px: identical window rectangle, DWM frame bounds and client area | 0 px, no move |
| Windows 11, 225 % | maximized | 0 px | 0 px |
| WSLg X11 (Weston XWayland manager), 100 % | normal, two sizes | child client (−32, −32) px from the main window's | first viewer moved once by (+32, +32), then 0 px; later viewers 0 px with no move |
| WSLg X11, 100 % | maximized | 0 px | 0 px |

Not measured: GNOME itself (X11 or Wayland), fractional scaling on Linux and Windows
with monitors of different DPI. WSLg forces Slint onto X11, so its Wayland path
cannot be exercised there. To check a desktop, run
`cargo run -p midnight-ui-input --example window_placement`: it opens a window, two
children over it and then both maximized, prints the client bounds and ends with
`Placement matches.` On Wayland it compares sizes and states only; there, check by
eye that the child appears over the window.
