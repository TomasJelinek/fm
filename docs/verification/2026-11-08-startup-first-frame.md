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

## Renderer startup recovery (2026-11-11)

The default renderer remains unchanged. Failure during backend selection
(including required-WGPU adapter preflight), initial window creation, or native
initialization before the first successful `show()` produces a typed
`NativeStartupFailed` outcome. After bounded engine and runtime cleanup, the
application starts its own executable once with the original raw OS arguments
and working directory, plus `--software`. It invokes no shell, reports a failed
software child, and creates no extra Windows console. Explicit `SLINT_BACKEND`
choices and an existing `--software` argument disable automatic recovery.
Failures after a successful first reveal never restart the application, so
active operations cannot be replayed.

A single shared gate protects the existing backend settings worker. Font and
preference reads proceed immediately. Writes, including normalization saves
triggered by loading preferences, wait for the first successful reveal. The UI
only signals reveal or abort; it never waits. A pending save has a 30-second
deadline. Aborting startup or dropping its attempt owner wakes blocked saves and
rejects them without modifying saved preferences. The failed hidden attempt also
skips final geometry capture and preference reconciliation/flush. Once revealed,
ordinary preference saves and normal shutdown persistence remain enabled.

Cleanup uses the existing bounded shutdown deadlines. It does not promise that
every already-running OS read has terminated: Tokio cannot forcibly stop an
in-progress blocking call. The failed attempt's preference gate remains closed,
its UI is never revealed, and no interactive operation is replayed in the
software process. This recovery does not switch an already-visible window to a
different renderer after device loss.

Regression coverage is in the app's `startup_recovery`, `startup_gate`, and
`startup_settings` modules: default-only eligibility, selector failure, lossless
arguments/cwd, cleanup ordering, one-attempt failure reporting, concurrent
reveal/abort/deadline behavior, and actual settings-service reads/writes.
The coordinator observed the retry/gate tests fail before implementation and the
old executable fail the no-adapter default-startup smoke. The final Windows gate
passes all 13 new startup cases (39 app tests passed, 15 interactive tests ignored),
with strict all-target Clippy passing.

Final Windows native qualification passed on app build `80e7d310`
(SHA-256 `80e7d310f755218bdb652b5fe2602608f320372aae305a155c8f1f4ba1d4d7e6`).
Each process used in-memory smoke preferences and exited 0:

| Process-local configuration | Observed failure/recovery | Software restarts |
| --- | --- | --- |
| `WGPU_BACKEND=webgpu`, no renderer override or CPU-adapter bypass | Required-WGPU adapter preflight failed; automatic software startup succeeded | 1 |
| Same unavailable backend, plus `SLINT_WGPU_CPU=1` | Adapter preflight was bypassed; native WGPU surface creation failed; automatic software startup succeeded | 1 |
| Same unavailable backend, plus explicit `--software` | Software startup succeeded directly | 0 |

The three screenshots are byte-identical, each 825,642 bytes, with SHA-256
`a4013c797684b2d29b68038612fee30795a7fc6db9c3863dbe836c42ae56e664`.
Visual inspection confirms the requested System32/repository panes and normal
application controls. The no-adapter and late-device stderr logs each contain
exactly one retry message; explicit software has no stderr output. Standard
backend, native-renderer, and software-renderer startup smokes also passed.
These checks qualify startup recovery, not a frame-rate guarantee.

Evidence is under `_build/qa/smooth-scroll-20261111/`:
`final3-gpu-fallback-smokes.ps1`,
`final3-{no-adapter,late-device,explicit-software}.{stdout,stderr}.log`, and
`final3-{no-adapter,late-device,explicit-software}-startup.png`.
The fixture preserves environment overrides within each child process and checks
exit status and snapshot publication.
