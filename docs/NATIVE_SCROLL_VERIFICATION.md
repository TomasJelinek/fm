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

## Maximized panel measurements — 2026-11-11

The historical `--check` remains an interpolation and idle-alignment check. It
does not establish smooth frame pacing: four fractional positions can still be
separated by long frame stalls. The extended probe writes each rendered offset,
redraw dispatch/submission time, and UI-thread CPU time to CSV, and emits
`frame_metrics` JSON for each wheel impulse. Its timestamps do not measure GPU
completion, compositor presentation, scanout, or physical mouse-to-photon time.

Build the example using the normal project environment, then run:

```powershell
scripts/cargo.ps1 build -p midnight-ui --example native_scroll --offline --locked
_build/debug/examples/native_scroll.exe --app-renderer --path=C:\Windows\System32 --maximized --zoomed --check
```

`--path=` loads the actual directory's names, kinds, lengths, and modification
times on a worker before opening the window. The fixture uses the production
Presenter, MainWindow, FilePane, and renderer. It does not instantiate Engine,
the installed filesystem provider, native icon loading, or preview services;
its numbers must not be described as full-application measurements. No file
contents are opened and no preferences are saved. By default, timed rows have no
seeded file/folder icons; nonempty fixture icons are used only in post-timing
visual checks. `--timed-fixture-icons` instead loads the existing file/folder
fixture artwork before warmup and logs how many of the first 64 model rows have
nonempty icons. This exercises image rendering during scrolling, but remains
separate from Engine and native shell-icon loading.

`--app-renderer` launches a fresh child with the application's effective default
WGPU policy: DX12, DEBUG/VALIDATION disabled, and indirect-call validation
retained. Explicit WGPU backend/debug/validation environment settings remain
available, matching the app's diagnostic overrides. The probe logs these
effective environment settings, GPU transformation support, physical window
size, scale, maximized state, panel size, display name, and display refresh rate.
The original generic renderer remains available by omitting the flag.

`--zoomed` invokes the same panel action as Ctrl+F11. `--maximized` requests a
maximized native window. `--sustained` injects 21 wheel notches at successive
200 ms timer callbacks, allowing repeated-input and sampling measurements;
omit `--check` because that legacy check assumes isolated, fully settled notches.
The CSV retains actual input/frame times rather than assuming timer deadlines.
`--scrollbar-wheel` moves the injected pointer over the vertical Fluent scrollbar
while retaining real native wheel messages; the default target is list content.

`--capture-pixels=<directory>` performs three GPU readbacks only after timing
has stopped. It saves cropped actual file-pane pixels at offsets 0, -5.5, and
-11 logical pixels and reports physical row-background boundaries. These are
pixel-rendering checks, not performance samples. Ordinary row, changed
selection, and changed-theme background checks also run after capture ends.

The Windows CPU accounting API has coarse granularity on this machine
(15.625 ms); aggregate CPU totals are more useful than one frame's CPU value.
A long wall-clock submission span with little corresponding UI CPU establishes
waiting or preemption, but does not by itself identify a GPU wait or its cause.

Evidence is under `_build/qa/smooth-scroll-20261111/`. The preserved
`native-scroll-control.exe` permits repeated controls after candidate builds.
`run-native-case.ps1` records executable SHA-256, geometry, raw metrics, and the
maximum number of Cargo/rustc/link processes observed every 250 ms. Runs that
overlap compilation are retained but excluded from comparisons.

The runner's optional `-MaxP95FrameMs 16.667` makes the requested p95 cadence
budget a manual acceptance gate after saving the measurements. It is deliberately
not a machine-independent CI assertion. The legacy example `--check` can pass
while this performance gate fails.

### Row-cache experiment

The previous row-cache experiment above was removed because rendering-notifier
instrumentation perturbed pipeline behavior. The 2026-11-11 experiment revisits
the hint on each virtualized FileEntry using the corrected observer. The control
and candidate executables were frozen before the repeated runs, and no Cargo,
rustc, or linker processes were observed during these four comparisons.

Matched geometry was 3,840×2,241 physical pixels, 225% scaling, a maximized
single panel, and a 60 Hz monitor. The fixture enumerated 5,104 filesystem entries
plus the parent row. Sustained motion crosses new rows; it is not confined to
moving already cached artwork inside one row.

| Workload | Cache | Active gap samples | Gap median / p95 | Submission median / p95 |
| --- | --- | ---: | ---: | ---: |
| Four isolated notches, repeated control | Off | 45 | 24.577 / 36.157 ms | 24.584 / 36.072 ms |
| Four isolated notches | On | 44 | 20.372 / 25.983 ms | 20.733 / 25.927 ms |
| 21 sustained notches | Off | 164 | 26.430 / 35.707 ms | 26.052 / 34.657 ms |
| 21 sustained notches | On | 200 | 20.979 / 33.751 ms | 20.640 / 32.687 ms |

These are `system32-control-repeat`, `system32-cache-isolated`,
`system32-control-sustained`, and `system32-cache-sustained` CSV/log/JSON files.
The candidate produced more intermediate frames during the comparable sustained
motion, and reduced median submission time. Its p95 still exceeds two 60 Hz
refresh intervals; these results do not establish smooth 60 Hz presentation.

First-response latency is separate from gaps between active frames. The isolated
candidate had one 424.813 ms first-moving-frame submission. The initial control
also had a 540.565 ms submission stall; the subsequent quiet repeated control
did not reproduce it. Neither outlier is hidden by the gap table, and neither
is assigned a cause without further evidence.

All four runs passed ordinary/selected/changed-theme row-background assertions.
The candidate's post-timing readbacks also retain fractional pixels: at logical
offsets 0, -5.5, and -11, the first row boundary moves from physical rows 49–50
to 36–37 to 24–25 at 225% scaling. Those images demonstrate rendering continuity,
not compositor cadence or full-application performance.

### Remaining resource churn

The candidate's matching local PDB permitted owned-process instruction and
call-stack sampling without rebuilding. The sparse-IP run is
`system32-cache-profile-*`; the bounded 64-stack run is
`system32-cache-stacks2-*`. Their maximum observed compiler counts were two and
three respectively. These are diagnostic call-site evidence, excluded from
the quiet timing comparison. Sampling only addresses in `win32u.dll` deliberately
biases the stack set toward the allocation/wait sites; stack counts are not
wall-time percentages.

The sampler used the UI process directly, resumed its thread in a `finally`
block after each sample, and used only local symbols. Timestamped native input
observations bounded the offset between sampler and probe clocks. Only samples
whose entire timing uncertainty lay inside a CSV active-motion interval were
retained. The sparse-IP uncertainty was 20.157 ms; the stack-run uncertainty was
57.037 ms. Startup, idle settling, and post-timing readbacks were excluded.

Of 64 retained stacks, 28 include bind-group creation, 26 include bind-group
destruction, and two include buffer initialization. The observed paths include:

- `CreateCommittedResource` / graphics synchronization → DX12
  `DeviceAllocationContext::create_buffer` → `create_bind_group` → WGPU/FemtoVG
  command rendering.
- Graphics allocation destruction → `free_resource` → `destroy_bind_group` →
  WGPU `BindGroup::drop` / submission lifetime retirement → `Queue::submit`.
- Resource mapping → WGPU `StagingBuffer::new` / `create_buffer_init` → FemtoVG
  render-target setup.

The pinned WGPU DX12 implementation allocates an `Internal Sampler Index Buffer`
when a new bind group contains samplers. This directly explains why repeated
texture/sampler bind-group construction reaches GPU allocation and destruction;
it does not imply that every allocation is a viewport uniform buffer.

A separate `SLINT_DEBUG_PERFORMANCE=refresh_lazy,console` run reported last-frame
layer counts of 0, 0, 1, 0, and 0 during sustained input. These once-per-second
samples count newly allocated GPU layer textures. Slint can repaint an invalidated
layer into its existing same-size texture without incrementing that counter, so
these samples do not establish how often row contents are repainted. They rule
out wholesale layer-texture allocation in the sampled frames. Its instrumentation
is not used for the quiet timing table. Files: `system32-cache-layers-diagnostic.*`.

An explicit Vulkan comparison also retained GPU rendering and identical
geometry, with no compiler overlap. It had 152 active gaps: median 31.868 ms,
p95 33.491 ms, maximum 37.609 ms. Submission median/p95 were 31.186/33.054 ms,
while aggregate UI CPU was only 468.75 ms over 153 active frames. The lower CPU
cost did not yield 60 Hz cadence, so a backend switch alone is not an established
solution. Files: `system32-cache-vulkan-sustained.*`. The collector exposes the
FemtoVG/WGPU renderer name, not adapter identity; no exact Vulkan adapter claim
is made from that output.

### Persistent binding-cache candidate

The native allocation stacks motivated a bounded renderer-local cache of 512
texture/sampler bind groups. It validates the actual texture handle and sampler
flags, invalidates entries on image deletion/reallocation and uniform-buffer
replacement, and bypasses unmanaged temporary filter textures. This experiment
does not alter pipeline retention.

The frozen executable is `bindings-candidate/native_scroll.exe` (SHA-256
`DC572C0EAC16A2844189EE9370F1E16DA0EFDB8B2EA6A1F2380E0CB352BD86E5`).
It also includes the then-current `zeno` development-profile optimization and
integrated input changes. The preserved row-cache-only control predates those
changes, so this is a combined branch comparison, not an isolated estimate of
the binding cache's contribution. The separate viewer profile experiment found
no useful high-DPI `zeno` improvement.

All four runs below used the same System32 data, maximized/zoomed geometry,
DX12 policy, and 60 Hz monitor described above. Every run exited successfully,
and no Cargo/rustc/link process was observed during capture.

| Workload | Frozen variant | Active gaps | Mean event cadence | Gap median / p95 | Submission median / p95 |
| --- | --- | ---: | ---: | ---: | ---: |
| Isolated | Row cache only, fresh control | 45 | 51.22 Hz | 19.074 / 23.636 ms | 19.097 / 25.936 ms |
| Isolated | Binding cache candidate | 45 | 62.64 Hz | 16.285 / 19.535 ms | 4.711 / 10.091 ms |
| Sustained | Row cache only, fresh control | 215 | 49.76 Hz | 19.570 / 24.005 ms | 19.297 / 23.344 ms |
| Sustained | Binding cache candidate | 247 | 60.20 Hz | 16.533 / 18.703 ms | 15.495 / 18.117 ms |

Mean event cadence is active-gap count divided by summed active-gap time. It
can exceed display refresh when redraw dispatches occur close together; it is
not presented FPS. The candidate's sustained maximum gap was 19.987 ms, and its
maximum moving-frame submission was 19.851 ms. Aggregate UI CPU fell from
2,906.25 ms over 216 active control frames to 937.5 ms over 252 candidate frames.
The remaining sustained submission spans often approach one refresh interval;
these data alone cannot attribute that waiting to a particular synchronization
site. The p95 remains above a strict 16.667 ms budget, so this is a measured
improvement toward 60 Hz cadence, not a guarantee of uniformly paced scanout.

Isolated input-to-first-submission values were 14.001–40.380 ms for the candidate
and 22.609–51.436 ms for its fresh control. Motion completed in 219.916–227.723 ms
versus 276.486–288.238 ms. The previous first-moving-frame outlier did not recur
in this candidate pass. Measurements start at injected native input, not at a
physical wheel sensor.

Both candidate runs passed the existing background/selection/theme assertions
and produced the same fractional row boundaries at offsets 0, -5.5, and -11 as
the earlier pixel proof. A new post-timing check also updates the same live
`VecModel` row without replacing its model or changing its view mode: changing
the name altered 3,622 pixels; replacing an 8×8 red icon with a 13×13 blue icon
painted 1,936 blue pixels and left zero old red pixels. The updated name remained
unchanged during the icon update. These checks exercise cached row invalidation,
including different backing image sizes.

Files are `system32-bindings-control-isolated`,
`system32-bindings-control-sustained`, `system32-bindings-isolated`, and
`system32-bindings-sustained` CSV/log/JSON artifacts, with candidate BMP crops in
the corresponding `-pixels` directories.

The three explicitly enabled renderer GPU regressions also passed on DX12:
resource reuse with live pixel updates and same-ID texture reallocation/deletion;
uniform-arena growth with correct rebinding; and the 512-entry eviction bound
plus pruning of undrawn reallocated textures. Their actual adapter was Intel(R)
Iris(R) Xe Graphics (integrated), driver 32.0.101.7088. This adapter report belongs
to the standalone tests; the native window probe itself does not expose adapter
identity. The successful log is `vendor-bindings-green2.log` (3 passed, 0 failed).

```powershell
$env:WGPU_BACKEND='dx12'
cargo test --manifest-path vendor/femtovg/Cargo.toml --lib --no-default-features --features wgpu --locked --offline --target-dir _build renderer::wgpu::bindings::tests -- --ignored --nocapture --test-threads=1
```

The standalone vendor lock initially selected incompatible Windows bindings for
`gpu-allocator`; its narrow update to the already present `windows` 0.62.2 fixed
test compilation. The application lock was left unchanged by that correction.

### Linux compile verification

The final affected UI and application graph passed a one-job Linux check with
`--locked --offline`, using `_build/qa/interaction/linux-verification-env.sh` and
`_build/linux`. The command selected `midnight-ui-input`, `midnight-ui`,
`midnight-viewer-ui`, `midnight-compare-ui`, and `midnight-explorer`. It completed
in 9 minutes 37 seconds. After the final viewer input-routing fixes, the same
incremental graph check passed again in 59.15 seconds. These are compile
portability checks, not Linux GUI frame-pacing measurements. Both the root and
standalone renderer lock files had
identical SHA-256 hashes before and after. The log and hash records are
`linux-affected-check.log` and `linux-check-locks-{before,after}.sha256` in the
same evidence directory. The final incremental run is recorded in
`linux-final-affected-check.log` and `linux-final-check-locks-{before,after}.sha256`.

After the final Options scroll-extent and native notice refresh fixes, the same
one-job affected/application check passed once more in 4 minutes 3 seconds.
The root and renderer lock hashes again matched before and after. This last
source check is recorded in `linux-final-notice-affected-check.log` and
`linux-final-notice-check-locks-{before,after}.sha256`.

### Final application-graph native verification

After the final Windows format, all-target Clippy, and combined application/probe
build, three quiet native runs used the frozen `final-candidate/native_scroll.exe`
and matching PDB. Its SHA-256 is
`7B2B70F5B956E430BCFFF4873CF39DCC2A98BDCD91C5D0BCD21AC8F6EF322C22`.
This build includes the final input-routing changes and renderer caches, with
the unsuccessful `zeno` profile experiment removed. All three processes exited
successfully, and each recorded zero concurrent Cargo/rustc/link processes.

The geometry remained 3840×2241 physical pixels, scale 2.25, maximized with the
left panel zoomed to 1692.667×755 logical pixels, on the 60 Hz display. All runs
reported GPU transformations enabled and the application's DX12/debug-off/
validation-off policy. The source contained 5,104 System32 entries plus the
parent row.

| Final workload | Active gaps | Mean event cadence | Gap median / p95 / maximum | Submission median / p95 |
| --- | ---: | ---: | ---: | ---: |
| Sustained wheel over content | 248 | 60.00 Hz | 16.585 / 19.011 / 21.914 ms | 15.185 / 18.163 ms |
| Four isolated notches over scrollbar | 45 | 61.26 Hz | 16.549 / 19.021 / 24.424 ms | 8.947 / 16.631 ms |
| Sustained wheel with fixture icons | 246 | 60.24 Hz | 16.503 / 18.912 / 21.470 ms | 15.205 / 18.300 ms |

These remain redraw-dispatch/submission measurements, not compositor-presented
FPS. The p95 values exceed a strict 16.667 ms budget. The final data demonstrate
roughly 60 Hz average event cadence and fractional content motion, while retaining
occasional longer gaps. They do not establish uniformly paced scanout or physical
trackpad performance. The scrollbar case exercises native wheel routing while
the pointer is over the vertical scrollbar; it is not a scrollbar-drag benchmark.

The isolated scrollbar notches each produced 12–13 active frames, with 48 total
fractional-row frames. Input-to-first-submission was 6.738–34.894 ms; completed
motion took 221.214–231.018 ms. The first sustained response was 36.551 ms without
icons and 34.463 ms with icons; later sustained responses were 13.957–19.614 ms
and 14.529–19.286 ms respectively. The earlier several-hundred-millisecond
first-moving-frame outlier did not recur. Aggregate UI-thread CPU was
1,093.75 ms over 253 active frames for default sustained input, 281.25 ms over
49 isolated frames, and 1,062.5 ms over 252 icon-case frames. These coarse
GetThreadTimes samples should not be interpreted as precise per-frame CPU cost.

The icon workload passed `--timed-fixture-icons`, preloading ordinary fixture
file/folder artwork before warmup. It asserted 63 nonempty icon rows among the
first 64 model rows. It still bypasses the Engine, native shell-icon provider,
and thumbnail workers, so it does not measure their concurrent production cost.
The original baseline/control comparisons above retain their original icon-free
timed setup.

Every run passed ordinary-row, selected-row, theme-change, and same-model
name/icon-replacement pixel checks. GPU crops again placed the first row boundary
at physical y=49–50, 36–37, and 24–25 for logical offsets 0, -5.5, and -11.
The final icon crop was visually reviewed: artwork and text move together, and
the partially clipped top row remains correct. Name replacement changed 3,622
pixels; the subsequent blue icon replacement left zero old red pixels and kept
the updated name. These assertions occur after timing.

Artifacts are `system32-final-sustained`, `system32-final-scrollbar-wheel`, and
`system32-final-icons-sustained`, each with CSV/log/JSON and `-pixels` BMP files
in the same evidence directory. The latter two are reproduced by adding
`-ScrollbarWheel` or `-Sustained -TimedFixtureIcons` to `run-native-case.ps1`;
all three used `-CapturePixels` and the frozen final executable.

### Final regression gates

The broad eight-package Windows run initially finished with 1,531 passed,
14 failed, and 21 ignored tests. Targeted follow-up checks closed all 14
failures. The final focused batch passed 40 cases (`final-fixture-green.log`),
and the shared bitmap-helper follow-up passed 33 cases
(`bitmap-helper-green.log`). Two failures exposed real UI defects: the Touch
General options layout understated its scroll extent, and an existing native
notice did not refresh its completed body within the same generation. Both
production fixes passed their corresponding interaction regressions. The other
closures updated stale test fixtures and harness expectations.

Final strict all-target Clippy passed in 35.56 seconds; formatting passed in
45.45 seconds; the combined application/native-probe build passed in 8.70
seconds. The source-size check covered 1,807 owned source files, all at or below
100 KB, and `git diff --check` was clean. The final one-job Linux affected/UI/app
check passed in 4 minutes 3 seconds with unchanged lock files as recorded above.
Windows gate logs are `final3-clippy.log`, `final3-format.log`, and
`final3-debug-build.log` under `_build/qa/smooth-scroll-20261111/`. This is a composite
verification gate: the full broad suite was not rerun after the late fixes.

### Final debug deployment and startup

The final `me.exe` SHA-256 is
`C83FF72F063DA969923CF2DDFE386B805169F0A9906A1B4302B280CBC46036E5`.
It is available at `_build/debug/me.exe`; a matching private installation with
viewer, filesystem and thumbnailer plugins was deployed to
`%USERPROFILE%/MidnightExplorer-20261111-pixel-scroll`. Its hash-based runtime
registration permits the same verified build executable to find the protected
plugins without weakening plugin trust checks.

The backend smoke opened `C:/Program Files/Git` and its `bin` directory and
completed successfully. Separate software-renderer and native-renderer UI
smokes opened `C:/Windows/System32` and the project directory, saved screenshots,
and exited successfully. Both screenshots were visually reviewed: the panels,
icons, command strip and bottom buttons are present, with no startup error.
These startup checks do not replace the separate cadence measurements above.
Evidence is `final-deployment.log`, `final-{backend,ui-software,ui-native}.*.log`
and `ui-{software,native}-startup.png` in the same ignored QA directory.

The high-DPI text/source viewer and idle-restart limits remain open as detailed
in [the viewer verification report](verification/2026-11-11-viewer-pixel-scroll.md).
The panel result must not be generalized into a claim that every viewer now
maintains 60 Hz or that all first-input delays have been removed.
