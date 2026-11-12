# Window placement and persistent geometry

All text, source, Markdown, binary, image and PDF viewer windows, and the file
comparator, open using the main window's physical client size and outer-frame
position. Maximized and fullscreen state are copied before showing the child.
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
