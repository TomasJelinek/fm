# Native wheel scrolling investigation

The Windows probe uses the real MainWindow, presenter and Winit/FemtoVG-WGPU
renderer with 10,000 entries in each panel. It posts WM_MOUSEWHEEL to its own
window and records the resulting Winit LineDelta events and rendered list offsets.
It suppresses only the synthetic cursor-exit event caused by posting WM_MOUSEMOVE
without moving the desktop pointer. It does not move the user's pointer.

## Two independent causes

The 120 ms wheel tween previously advanced from wall-clock time in a timer.
When scheduling or rendering delayed that timer, the first displayed offset could
jump farther than a row. Exact 16 ms software tests did not reproduce this.
With a 32 ms first tick, the production clock regression observed a 27.73 px jump
from one 60 px notch. File rows in the desktop density are 22 px high.

The timer now requests a redraw. The native RedrawRequested observer advances
the motion immediately before rendering, spending at most 16 ms of animation
time per frame. Missed frames extend the tween instead of skipping invisible
positions. Same-direction impulses accumulate only undelivered pixels; reversal
discards the previous direction's residual. The existing 2,400 px pending bound
prevents an unbounded backlog. Pixel and phased touchpad input remains native.
Row snapping waits until the motion finishes and the list becomes idle.

Each discrete vertical notch travels at least one layout band. Detailed and
Brief use the original 60 px distance; Thumbnails uses its taller band height.
The native thumbnail regression showed that a 60 px notch could interpolate
correctly and then snap back to its starting band. The actual band-height
distance prevents that lost movement. The real-pane test exercises the same
thumbnail notch through interpolation and idle settling.

Cold fractional frames also exposed shader translation cost in development
builds. Local CPU sampling identified Naga WGSL/SPIR-V preparation. A targeted
`profile.dev.package.naga` opt-level 2 experiment reduced the first fractional
redraw span from approximately 61–73 ms to 20–31 ms on this machine. It did not
fix the clock bug: the native check still failed with a 27.82 px first jump.
The measured dependency optimization and the redraw clock fix address separate
causes; neither is a substitute for the other.

## Measurement correction

Slint's rendering notifier changes this renderer's work: it adds a clear-only
FemtoVG flush. In the pinned renderer, each flush evicts unused pipelines, so the
extra flush caused repeated shader compilation. Initial notifier-based scroll
and resize numbers therefore measured a perturbed rendering path. Row caching
was tried under that instrumentation and removed; it is not part of the fix.

Both native probes now observe Winit RedrawRequested boundaries and the following
event/about-to-wait callback without installing a rendering notifier. This span
measures dispatch and submission, not GPU completion or display scanout. The
corrected resize run had 32 samples: request-to-submission median 117.946 ms,
p95 217.187 ms; redraw dispatch median 41.972 ms, p95 57.590 ms. Those values must
not be compared directly with the old notifier-based benchmark.

## Reproduction and artifacts

Build with `scripts/cargo.ps1 build -p midnight-ui --example native_scroll --offline
--locked -j1`, then run the built example with `--mode=0 --check`, `--mode=1 --check`
and `--mode=2 --check` for Detailed, Brief and Thumbnails. The Windows native code
lives in `examples/native_scroll/windows.rs`; the launcher is platform-neutral.
The check requires at least four distinct moving positions per notch, checks
the first and every later active step is smaller than a row, and checks exact
idle row alignment and progress past the starting band. Pixel snapshots also
verify ordinary, selected and changed theme row backgrounds after performance
capture has stopped. `--capture-views` additionally writes `panel-views.bmp` with
Brief and Thumbnails side by side; the probe supplies existing icon artwork only
after timing capture, because its fixture has no desktop backend.

Local evidence is under `_build/qa/native-scroll/`:

- `normal-renderer.csv`: corrected baseline without the rendering notifier.
- `naga-optimized.csv`: targeted optimization with the old clock, still failing.
- `redraw-driven.csv` and `redraw-driven-error.log`: final clock fix passed the
  native check. The four notches yielded 7, 8, 7 and 7 distinct moving frames;
  the first step was 14.93 px, all active steps stayed below the 22 px row height,
  and idle alignment plus all three pixel-background checks passed.
- `corrected-resize.log`: corrected resize benchmark described above.
- `cpu-samples.txt`: local symbolized samples used to identify shader preparation.
- `wheel_frame_red.rs`: delayed-frame regression that failed against the old clock.

The final workspace-built probe also passed all three layouts with four real
wheel notches in each run:

| Mode | Band height | Distinct moving frames per notch | Largest active step | Final aligned offset |
| --- | ---: | --- | ---: | ---: |
| Detailed | 22 px | 7, 8, 7, 7 | 14.93 px | -264 px |
| Brief | 22 px | 8, 7, 7, 7 | 14.93 px | -264 px |
| Thumbnails | 135 px | 7, 7, 7, 7 | 33.60 px | -540 px |

These captures are `detailed-final.csv`, `brief-final.csv` and
`thumbnails-final.csv`, with matching `.log` files containing the native input
events and successful assertions. Every run passed the first/later step bound,
idle alignment and forward-progress checks. After timing, each run returned to
Detailed and passed all three row-background checks. `panel-views.bmp` was
visually inspected with seeded icons visible in the Brief columns and thumbnail
grid. The mode integration test separately verifies thumbnail selection pixels,
visible-cell metadata updates, independent mode switching and bounded reads
from 10,000-entry models. The wrapped thumbnail prefix test also checks Unicode
shaping and multiline highlighting across themes, densities and font sizes.

The seven source clock tests cover exact delivery, delayed presentation,
accumulation, reversal, cancellation and bounded burst input. They also verify
that sustained input arriving immediately before redraw cannot starve an active
animation by repeatedly resetting its presentation timestamp. The real-pane
software test also checks that a timer wakeup alone does not move the viewport
and that the rendered row boundary passes through an intermediate pixel
position during model refresh.

The final Windows workspace test run passed 633 tests with 8 explicitly ignored
fixtures/benchmarks (`windows-workspace-green.log`). The additional native
case-collision regression passed separately (`windows-case-collision.log`).
Strict workspace/all-target Clippy passed (`windows-clippy-final.log`), followed
by a final library/operation test run after its style-only fixes: 469 passed,
6 ignored (`windows-final-lib-operations.log`).
