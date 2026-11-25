# FIFO presentation on Wayland

Linux, native Wayland (GNOME Shell 50.1, 59.94 Hz display), a discrete workstation GPU with
its driver 615.71.09, Slint 1.18.1 with FemtoVG-WGPU on WGPU 30 (Vulkan). Main was
`7454289b`, which includes the
[pipeline retention](2026-11-25-wayland-pipeline-retention.md). The probes are debug
builds.

## Symptom

On native Wayland with the GPU renderer, animation and wheel scrolling stalled for up to
about a second at a time. XWayland and the software renderer were smooth on the same
desktop, with the same window placement.

- **About sky.** The About box's animated sky ran at 0–40 frames per second.
  - The About probe, `crates/midnight-ui/examples/native_about.rs`, shows MainWindow with
    10,000 rows per pane, opens the About box and measures its redraws for 5.5 s.
  - In one run not a single frame was drawn while the box was open.
- **Wheel scrolling.** `performance_probe --observer-wheel --sustained --wheel-trace`
  scrolled the long and short files of the pipeline record.
  - In builds of `ad9557bd` with and without the pipeline change, every run showed stalls
    of 0.9–1.2 s.
  - On `7454289b` all three runs were aborted after 2–4 moving frames.

## Cause

A `WAYLAND_DEBUG=client` trace shows how the driver presents in FIFO mode. Each frame
carries:
- the application's frame callback request (`pre_present_notify`);
- a newly created color-management image description;
- a `wp_commit_timer_v1` target time;
- explicit-sync points;
- `wp_fifo_v1` `set_barrier` and `wait_barrier`, then the buffer commit;
- a second, empty commit that waits on the barrier.

Two problems follow.

1. **A skipped refresh per frame.** Each target is the last reported presentation plus two
   refresh periods, less 0.5 ms. A client paced by frame callbacks, as Slint is here, already
   knows its previous frame's presentation, so its frames skip every other refresh. The
   frame gap was about 33 ms, or 17 and 33 ms alternately.
2. **Commits presented far past their target.** The compositor presented many timed commits
   long after their target time. Meanwhile the application sat idle, waiting for the frame
   callback.
   - In a wheel trace, six frames were 167–968 ms late.
   - In an About trace, none of 18 timed frames was presented within one refresh of its
     target: 7 were 100–500 ms late and 10 over 500 ms late.

Explicit sync is not the cause. With the driver's switch that disables it, the timed FIFO
commits and the stalls remained.

## Mailbox experiment

The experiment was not committed. A private copy of Slint's FemtoVG-WGPU adapter took the
present mode from an environment variable; the adapter otherwise hard-codes `AutoVsync`
(FIFO) in `resize()`.
- The driver offers Mailbox and FIFO on the Wayland surface, and FIFO and Immediate under
  XWayland.
- In Mailbox mode the driver sent neither commit-timing targets nor FIFO barriers: plain
  commits with frame callbacks.
- Rendering stayed paced by the frame callbacks, so no extra frames were drawn.

About probe:

| Setup | Runs | Frames per second | Gap median | Gap max |
| --- | ---: | --- | ---: | ---: |
| Wayland, GPU, FIFO (shipped) | 2 | 40.0 and 0 | 32.8 ms | 33.5 ms (first run) |
| Wayland, GPU, Mailbox | 2 | 59.9 and 60.0 | 16.7 ms | 17.0–18.1 ms |
| XWayland, GPU, FIFO | 1 | 60.4 | 16.6 ms | 19.0 ms |
| Wayland, software | 1 | 60.0 | 16.7 ms | 17.1 ms |

Wheel probe on `7454289b`:

| Setup | Runs | Moving frames | Gap p50 | Gap p95 | Gap max | Redraw p50 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Wayland, GPU, FIFO (shipped) | 3 | 2–4, all aborted | – | – | up to 133 ms | 2.0–2.2 ms |
| Wayland, GPU, Mailbox, long | 2 | 301, 302 | 16.6 ms | 17.5–17.6 ms | 21.1–25.9 ms | 1.2 ms |
| Wayland, GPU, Mailbox, short | 1 | 36 | 16.6 ms | 17.4 ms | 20.7 ms | 1.0 ms |

## Options

- **Select Mailbox on native Wayland when the surface offers it**, and keep FIFO elsewhere.
  This is now implemented through a patched copy of Slint's FemtoVG-WGPU adapter; see
  [Mailbox presentation on Wayland](2026-11-25-wayland-mailbox.md).
- **Workarounds for earlier builds.** XWayland (`env -u WAYLAND_DISPLAY`) or the software
  renderer (`--software`) avoid the problem.

## Reproduction

On a native Wayland desktop with the discrete GPU, run the About probe from the
repository root, preferably with throwaway XDG folders:

```sh
cargo run -p midnight-ui --example native_about
```

For the controls, add `MIDNIGHT_PROBE_RENDERER=software` for the software renderer, or
prefix `env -u WAYLAND_DISPLAY` for XWayland. To see the timed commits, add
`WAYLAND_DEBUG=client` and pair each `wp_commit_timer_v1.set_timestamp` with its
`wp_presentation_feedback.presented` event. The Mailbox runs need the uncommitted adapter
patch described above.

## Limits

- One desktop, compositor version and driver.
- The compositor was observed only through its protocol events, so which side should change
  is not established.
- Debug-profile probes; a 1000×700 viewer window and a 1340×820 main window.
