# D3D viewer pacing attribution — 2026-11-12

SCROLL-02 remains open. The current locked-desktop delays are attributable to waiting for a
DX12 swapchain image before Slint draws the frame. They do not justify a renderer optimization
or establish normal foreground interaction performance. No production or probe source changed.

## Reproduction and probe overhead

Starting repository: `5388a97`. Existing probe SHA-256:
`abdaa33884cc14a657ebb5024446e67beb85927fafc5c7ae3ff496eaceb2f85e`.

The existing binary was staged as `scroll-pacing-baseline-abdaa338.exe` beside the qualified
`C:/Users/tjelinek/MidnightExplorer-20261112-lifetime-dfbafca6/me.exe`. Plugin packages, receipts,
registration and application settings were unchanged. Reports and QA scripts are under
`_build/qa/d3d-scroll-20261112/`.

The BAT reproduction uses the supplied `D:/dev/ov-physics-trunk/ovclash/build.bat`, Text mode,
acknowledged normalized start 0.7, Segoe UI 13, native parent, and 1000×700 logical pixels
(2250×1575 physical at scale 2.25). It posts three native wheel notches to the owned viewer.
Spatial proof and GPU readback are disabled. The original performance gates remain unchanged.

~~~powershell
& _build/qa/d3d-scroll-20261112/run-case.ps1 -Label baseline-trace -Trace
& _build/qa/d3d-scroll-20261112/run-case.ps1 -Label baseline-no-trace
& _build/qa/d3d-scroll-20261112/run-case.ps1 -Label baseline-foreground-trace -Trace -Foreground
~~~

All three original guard runs fail. The trace-disabled run still has moving redraws of
229–564 ms and only three content positions in its first two gesture intervals. Thus numeric
trace collection is not the primary cause of these delays. These are background observations,
not a valid foreground timing comparison.

Independent source review confirmed that `render_observer::finished` stops the measured redraw
interval before its timeline recording and after-frame callback. The next redraw timer starts
after those callbacks. Normal retained-raster wheel observations do not sample source pixels;
the bounded bilinear sampler is explicitly opt-in and marks its results timing-excluded.
Timeline storage is bounded to 65,536 rows and text formatting occurs after the event loop.
These facts do not make the observer a GPU or monitor-scanout timer: it measures native redraw
dispatch through the next event/about-to-wait boundary.

## Foreground precondition was unavailable

The external runner enumerates only windows owned by the launched process. It samples native
foreground identity, visibility, minimization and DWM cloaking, distinguishing the file-caption
viewer from its native parent. Every recorded owned window in the first three runs had zero
foreground observations. The explicit `SetForegroundWindow` request returned false.

The subsequent real-click option checks PID ownership, the acknowledged `build.bat` caption,
and `WindowFromPoint` ownership before sending any mouse input; it restores the cursor after
any attempted movement. The guarded click did **not** execute. Its point resolved to Explorer,
not to the viewer. A separate read-only native inspection found the foreground process was
Windows `LockApp`, session 1. Both calling and input desktops were named `Default`, so desktop
name, IsWindowVisible and a prior Winit Focused event were insufficient foreground evidence.

The two `foreground-click-*` reports therefore remain invalid as foreground comparisons.
One passes the original sample guard and one fails; neither is a smoothness qualification.
`click-setup-diagnostic-excluded.*` records the failed ownership precondition. No input was
sent to Explorer, LockApp or another application, and no attempt was made to dismiss the lock UI.

The actual installed `me.exe` foreground interaction path was not qualified in this state.
That and a matched foreground probe run remain required after the desktop is unlocked.

## Fresh call-stack attribution

The old November 11 traces found DX12 allocation/destruction costs. Those historical findings
were not assumed to explain the current delay.

The new sampler uses matching local PDB symbols, samples only the owned probe main thread,
resumes it in a `finally` block, and caps the capture at 1,024 stacks / 60 seconds. Captures are
diagnostic only; thread suspension invalidates performance comparisons. All artifacts retain
the original guard outcomes.

- `locked-stacks.*`: 169 stacks, total suspension 385 ms, maximum 10 ms. The BAT mode/seek
  readiness gate fails during this capture, so it establishes opening behavior only.
- `locked-md-stacks.*`: exact supplied `D:/dev/ov-physics-trunk/ovclash/AGENTS.md`, initial
  position zero without a mode change or explicit seek; 190 stacks, total suspension 409 ms,
  maximum 10 ms. Native wheel gestures run and the original jump guard fails. This is valid
  active-motion attribution, not a performance pass.

The common-clock correlation keeps only samples whose whole suspension interval is at least
20 ms inside a redraw longer than 100 ms after the first native wheel enqueue. Nine Markdown
redraws span **213.987–254.528 ms**. All **55 interior samples** follow:

~~~text
WaitForSingleObjectEx
  wgpu_hal::dx12::Surface::acquire_texture
  wgpu_core::Surface::get_current_texture
  wgpu::Surface::get_current_texture
  Slint WGPUBackend::begin_surface_rendering
  FemtoVGRenderer::internal_render_with_post_callback
~~~

No retained interior sample reaches raster layout, image upload, buffer allocation or queue
submission. This local sample set locates the observed wait; it is not a wall-time percentage
or proof that these other costs are always zero. The unchanged-upload moving intervals also
stall. Summary: `locked-md-stacks-interior-summary.json`; raw sample and resolved-symbol CSVs
share that prefix.

Pinned WGPU 30.0.1 defaults to a frame-latency waitable object. Its DX12 `acquire_texture` calls
`SwapChain::wait`, which invokes `WaitForSingleObject`, before selecting the next backbuffer.
Slint 1.18.1 calls that acquisition before item rendering; its surface uses AutoVsync.
Microsoft describes the waitable handle as synchronization with prior-frame presentation.
[Microsoft: GetFrameLatencyWaitableObject](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_3/nf-dxgi1_3-idxgiswapchain2-getframelatencywaitableobject)

This supports **presentation backpressure in the captured environment**, rather than a new
glyph composition, source-map, source-read or trace-formatting bottleneck. It does not prove
why Windows delayed the signal, or establish an adapter/driver fault. No current exact adapter
identity is claimed from loaded driver modules.

## Disposition

No source optimization is supported by the available interactive evidence. Removing the
wait, changing adapter policy, disabling VSync, forcing software or adding allocation caches
would change behavior without a qualified foreground reproduction. None was done.

Next gate: after unlock, retain OS foreground observations throughout exact BAT/Markdown wheel,
refill and reversal runs, then compare the actual installed application path. If large waits
remain, capture the same acquire/submit/present boundaries before choosing a bounded change.
The prior source-coordinate fix remains separately qualified; SCROLL-02 is not marked fixed.

No Cargo build or production tests were necessary for this documentation/QA-only investigation.
