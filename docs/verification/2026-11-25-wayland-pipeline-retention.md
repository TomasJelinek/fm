# Pipeline retention for Wayland frames

Linux, native Wayland (GNOME Shell 50.1), Slint 1.18.1 with FemtoVG-WGPU on WGPU 30,
vendored FemtoVG 0.27.0. The report was mouse-wheel stalls on a discrete workstation GPU
that did not occur on Windows.

## Cause

On native Wayland the application installs a Slint rendering notifier to request frame
callbacks ([source-wheel verification](2026-11-24-source-wheel-wayland.md)). Any rendering
notifier makes Slint flush a window's clear separately before its content. Upstream
FemtoVG kept only the render pipelines used by the latest flush, so the clear-only flush
dropped all but one of them. The next content flush therefore rebuilt every pipeline it
drew with, 15 per frame in the viewer.

A temporary counter in the probe below showed the pattern exactly: 15 pipelines created
in each content flush and 1 in each clear flush. With the discrete GPU's driver, stencil and
concave fills spent about 5.3 ms each in pipeline creation, so a redraw took about 28 ms
and wheel scrolling settled at 30 frames per second.

Windows and X11 install no notifier and flush once per frame. There, pipelines were only
rebuilt when a frame stopped using them, for example 586 creations in 321 XWayland frames
on the discrete GPU.

## Change

`vendor/femtovg/src/renderer/wgpu/pipelines.rs` keeps every pipeline until the cache holds
more than 64. It then drops those least recently used by earlier flushes, never one used by
the current flush. The policy and its reasons are in the [vendor notes](../../vendor/README.md).

## Regression tests

Build and list the test executable:

```sh
cargo test --manifest-path vendor/femtovg/Cargo.toml --lib --no-default-features \
  --features wgpu --locked --no-run
```

Run it from `vendor/femtovg`, with the explicit GPU cases:

```sh
WGPU_BACKEND=vulkan <test executable> renderer::wgpu:: --ignored --test-threads=1
```

- `pipelines_survive_a_flush_that_did_not_use_them` draws frames as Slint does with a
  notifier (a clear-only flush, then a concave fill).
  - On the unchanged code (`ad9557bd`, the test attached to the old cache) it failed:
    `frame 0: the clear-only flush left 1 of 3 pipelines`. The other 14 GPU regressions
    passed.
  - With the change, all 15 GPU regressions passed. The test also checks that later frames
    create no pipeline and that the output pixels are right.
- Four new cache tests (bound, eviction order, reuse, current-flush protection) need no
  GPU. The library's non-ignored tests: 74 passed, 0 failed, 15 ignored.
- Adapter: Mesa lavapipe, Vulkan on the CPU, selected with
  `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json`. The discrete GPU could not be used
  for these runs (see Not verified).

## Probe: pipelines created

The probe was `performance_probe --observer-wheel --sustained --wheel-trace` on the
unchanged and the changed code:
- 96 notches 50 ms apart, reversing after 48, in a 1000×700 window;
- a temporary per-flush counter that was not committed;
- FemtoVG-WGPU forced on lavapipe with `--d3d`, `WGPU_BACKEND=vulkan` and
  `SLINT_WGPU_CPU=1`;
- long file: `crates/midnight-viewer-engine/src/worker.rs` at `ad9557bd`, 2,277 lines;
  short file: a 73-line shell script.

| Trial | Pipelines created, before | after | Redraw p50, before | after |
| --- | ---: | ---: | ---: | ---: |
| Wayland, long | 1,596 in 204 flushes | 22 in 88 | 136 ms | 5.8 ms |
| Wayland, short | 1,108 in 200 flushes | 22 in 184 | 82 ms | 4.8 ms |
| XWayland, long | 328 in 255 flushes | 22 in 339 | 9.2 ms | 14.6 ms |

The changed code created its 22 pipelines while opening and none while scrolling.

Lavapipe timings stand for neither the discrete GPU's driver nor its presentation:
- On Wayland after the change, 300–575 ms gaps remained between moving frames. The UI
  thread was idle in them, waiting for the compositor's frame callbacks, and redraws took
  5–10 ms.
- Under XWayland the frame-gap p95 fell from 85.5 ms to 20.4 ms, with no gap over 34 ms
  after the change. Median redraw rose from 9.2 ms to 14.6 ms while 35% more frames moved,
  on a CPU rasterizer.

## Earlier discrete-GPU experiment

Earlier the same day, before this change, the driver was 595.99.02 and main was `4e3e64c6`.
An experimental switch then kept every pipeline, which matches this cache below its cap:
that workload drew with 22 states. Long file, native Wayland:

| Variant | Runs | Redraw p50 | Gap p50 | Gap p95 | Gap max |
| --- | ---: | ---: | ---: | ---: | ---: |
| Upstream retention | 5 | 27.8–29.1 ms | 33.3–33.6 ms | 34.5–51.4 ms | 45.3–245.6 ms |
| All pipelines kept | 3 | 1.6–1.8 ms | 18.4–22.2 ms | 36.5–40.1 ms | 50.8–55.7 ms |

The remaining gaps came from FIFO presentation. Mailbox presentation, which the driver
offers but Slint's FemtoVG-WGPU adapter does not select, brought the gap p95 to 17.8–18.0 ms
in two runs. That needs a Slint change and is not part of this one.

## Not verified

- **Discrete-GPU timings for this change.** At 17:44 the GPU driver's libraries were upgraded to
  615.71.09 while the 595.99.02 kernel module stayed loaded. Until a reboot, new processes
  could not use the GPU, and Slint fell back to its software renderer. Before/after timings
  on the discrete GPU, native Wayland and XWayland, are pending on driver 615.
- **Builds and platforms.** Not checked: a release (fat-LTO) build, the installed
  application, and Windows/DX12, where the single flush per frame now also keeps pipelines
  a frame did not use.
