# Mailbox presentation on native Wayland

Linux, native Wayland (GNOME Shell 50.1, 59.94 Hz display), a discrete workstation GPU with
its driver 615.71.09, Slint 1.18.1 with FemtoVG-WGPU on WGPU 30 (Vulkan). The cause is in the
[FIFO presentation record](2026-11-25-wayland-fifo-presentation.md): in FIFO mode the
driver commits frames with commit-timing targets, and the compositor presented them up to
a second late.

## Change

- **Vendored adapter.** Slint's FemtoVG renderer crate is vendored with a present-mode
  selector hook ([vendor notes](../../vendor/README.md#slints-femtovg-wgpu-adapter)).
- **Selection.** `midnight-ui-input`'s `NativePresentation` installs the selector at the
  first native Wayland window, before that window's first frame. Native Wayland surfaces
  then present in Mailbox mode when they offer it. Everything else keeps Slint's `AutoVsync`
  (FIFO): XWayland/X11, Windows, and Wayland surfaces without Mailbox.
- **Pacing.** Rendering stays paced by Wayland frame callbacks.

The driver offers Mailbox and FIFO on the Wayland surface, and FIFO and Immediate under
XWayland.

## Measurements

The before build was `6093c0cc`; the after build was the same code with this change. Both
are debug probe builds. Runs were serial, on the discrete GPU unless noted.

**About probe.** `crates/midnight-ui/examples/native_about.rs`: MainWindow with 10,000 rows
per pane in a 1340×820 window. It opens the About box and measures its redraws for 5.5 s.

| Setup | Frames per second | Gap median | Gap p95 | Gap max | Idle window |
| --- | --- | ---: | ---: | ---: | --- |
| Wayland, before | 2.5 and 3.2 | 583.5 and 17.0 ms | 600.8 and 586.2 ms | 600.8 and 586.2 ms | 2 frames in 1 s |
| Wayland, after | 60.0 and 60.0 | 16.67 ms | 16.83–16.84 ms | 17.3–18.3 ms | 2 frames in 1 s |
| XWayland, before | 60.4 | 16.65 ms | 17.19 ms | 18.23 ms | 4 frames in 1 s |
| XWayland, after | 60.4 | 16.60 ms | 17.34 ms | 19.23 ms | 4 frames in 1 s |

- The idle window is the second before the About box opens. The same number of frames
  before and after means nothing extra is drawn while idle.
- Under XWayland the redraw span median is about 16.5 ms, because FIFO presentation blocks
  for the next refresh inside the redraw, unchanged by this change.
- A third, traced before run on Wayland reached 40.0 frames per second.
- In the traced runs (`WAYLAND_DEBUG=client`):
  - before, the driver sent 243 commit-timing targets and 244 FIFO barriers;
  - after, it sent none, with a frame-callback request in every frame: 362 for 373
    commits.

**Wheel probe.** `performance_probe --observer-wheel --sustained --wheel-trace`:
- 96 notches 50 ms apart, reversing after 48, in a 1000×700 window;
- long file: `crates/midnight-viewer-engine/src/worker.rs` at `6093c0cc`, 2,292 lines;
  short file: a 73-line shell script;
- gaps are between consecutive moving frames, excluding idle time at a file end.

| Setup | Moving frames | Gap p50 | Gap p95 | Gap max | Gaps over 100 ms | Redraw p50 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Wayland, long, before | 19 and 22 | 17.7–18.0 ms | 583.9–584.8 ms | 585.0–585.8 ms | 9 and 10 | 2.3–2.6 ms |
| Wayland, long, after | 301 and 302 | 16.6 ms | 17.6 ms | 22.1–22.2 ms | 0 | 1.3–1.4 ms |
| Wayland, short, before | 4, aborted | – | – | 124.2 ms | 1 | 2.2 ms |
| Wayland, short, after | 36 | 16.6 ms | 17.4 ms | 20.8 ms | 0 | 1.0 ms |
| XWayland, long, before | 324 | 16.5 ms | 17.4 ms | 22.1 ms | 0 | 15.9 ms |
| XWayland, long, after | 318 | 16.5 ms | 17.3 ms | 21.2 ms | 0 | 16.0 ms |

The probe aborts a run whose sustained stage produces too few moving frames.

## Tests

All of these ran in a Linux build container.

- **`midnight-ui-input` library: 75 passed, 1 ignored.** It includes the two new
  present-mode tests: `native_wayland_presents_in_mailbox_mode_when_offered`, and
  `other_surfaces_keep_slint_fifo` (Wayland without Mailbox, and X11 with and without it).
- **`midnight-viewer-ui` library:** 253 passed.
- **`midnight-explorer` binary tests:** 141 passed, 17 ignored.
- **Workspace Clippy:** CI's
  `cargo clippy --workspace --all-targets --locked --features midnight-acceptance/acceptance -- -D warnings`
  reported no warnings. The vendored adapter, a path dependency outside the workspace,
  compiled without warnings.
- **rustfmt:** `cargo fmt --all -- --check` is clean.

## Not verified

- Windows, where the vendored adapter is built but no selector is installed. A release
  (fat-LTO) build and the installed application.
- Swapchain memory in Mailbox mode, other compositors and other drivers.
- The generated Cargo inventory under `docs/dependencies/` was not regenerated, which
  needs captures of every workspace on both targets. Its `i-slint-renderer-femtovg` row
  should become a local path dependency, declared directly by `midnight-ui-input`.
