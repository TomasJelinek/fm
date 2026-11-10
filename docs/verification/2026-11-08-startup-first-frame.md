# Startup first-frame restoration

## Cause and ordering

The main window previously called `show()` before entering the event loop. The
initial settings read was asynchronous, so its saved theme could only arrive
after a default frame. Geometry restoration was inside the 100 ms visible-window
sampler, which also required a completed successful settings read. This necessarily
allowed a visible default location/size before restoration.

The new startup task waits for the initial `PreferencesLoaded` response to be
applied and refreshed, then awaits the hidden native window, restores geometry
using the existing monitor policy, and shows the window once. Reading and writing
preferences remain on the existing backend settings worker. A failed read also
releases startup; the presenter's successful-read guard continues to prohibit
geometry saves over unread settings. A rejected initial request releases startup
after the existing error presentation instead of waiting forever.

The visible-window sampler now only captures and saves geometry. It cannot apply
an initial restore after a user has already seen or moved the window.

## Pinned backend behavior inspected

The local Slint 1.18.1 source establishes the ordering used here:

- `WinitWindowAdapter::new` registers an inactive window with hidden attributes.
- `SharedBackendData::create_inactive_windows` realizes it when the event loop
  resumes, including while the Slint window is hidden.
- `WinitWindowAccessor::winit_window` asynchronously waits for that native window.
- `WinitWindowAdapter::draw` skips hidden windows.
- `set_visibility` pre-renders before making the native window visible on Windows
  and X11. Wayland controls placement and has separate mapping semantics.

The native window reference returned by the await is dropped before show, so it
does not extend the native window lifetime after normal hide/close.

## Verification

Focused formatting completed with the repository's pinned Rust toolchain.
Cargo and native execution are coordinated with the shared application build;
the tests below are implemented but their execution results must be recorded
after that build. No native pass is claimed from source inspection.

Run the native cases individually on an interactive Windows desktop, because
winit permits one event-loop initialization per test process:

```powershell
cargo test -p midnight-explorer --bin me window_state::policy::tests
cargo test -p midnight-explorer --bin me window_state::startup_native_tests::native_first_frame_waits_for_saved_theme_and_monitor_geometry -- --ignored --exact --test-threads=1 --nocapture
cargo test -p midnight-explorer --bin me window_state::startup_native_tests::native_first_frame_uses_primary_when_saved_monitor_is_removed -- --ignored --exact --test-threads=1 --nocapture
cargo test -p midnight-explorer --bin me window_state::startup_native_tests::native_first_frame_restores_maximized_state_and_normal_bounds -- --ignored --exact --test-threads=1 --nocapture
cargo test -p midnight-explorer --bin me window_state::startup_native_tests::native_failed_preferences_show_defaults_without_overwriting_settings -- --ignored --exact --test-threads=1 --nocapture
cargo test -p midnight-explorer --bin me native_maximized_restore_keeps_normal_bounds_and_repeated_capture_has_no_drift -- --ignored --test-threads=1 --nocapture
```

The first-frame cases use the production startup controller and native
FemtoVG-WGPU/DX12 rendering. They deliberately delay preferences by 250 ms, assert
both Slint and the native window remain hidden without rendering, and inspect the
first `BeforeRendering` callback for the saved theme and physical rectangle. The
saved-monitor case selects a secondary monitor when available. The removed-monitor
case verifies the existing primary/work-area fallback. The failed-read case checks
that startup displays defaults and never saves geometry over the unread file.
The maximized case checks first-render maximization and the normal rectangle after
unmaximizing. The other geometry cases also check the settled rectangle, so a later
DPI/resize event cannot silently reintroduce size drift.

Winit's Windows maximization implementation temporarily calls native `SW_MAXIMIZE`
then `SW_HIDE` while applying flags to a hidden window. The probe checks native
visibility is false at the first pre-render, but does not observe every synchronous
Win32 visibility transition or prove a compositor never exposes one. This is a
specific native-validation limit, not a reason to bypass the preserved maximize
state or add a second visible restore.

Native Linux/X11 and Wayland have not been exercised by these Windows probes.
Wayland still delegates placement to the compositor; this change does not promise
absolute window positioning where the platform does not support it.

Coordinated verification: all four geometry-policy tests and all five native
Windows probes above passed. Native first-render logs record `native_visible=false`,
saved Nord theme, exact normal bounds, primary-monitor fallback and maximization.
Normal capture/restoration remained at (180,150), 720×460 across repeated samples.
Artifacts are `_build/qa/metadata-review/native_*.{out,err}.log` and `app-tests.log`.
