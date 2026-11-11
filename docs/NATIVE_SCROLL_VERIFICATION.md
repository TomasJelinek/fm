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

### Follow-up: idle viewer redraw allocation stalls

The post-commit investigation starts from `bc5c112`. It distinguishes the quiet
final viewer measurements from the subsequently instrumented diagnostic runs.
`performance_probe --wheel --wheel-trace --d3d` initially used a bounded
8,192-record buffer with one monotonic clock (the extended sustained probe raises
that cap to 65,536 records). It records native enqueue/receive, every redraw
(including unchanged content), UI frame publication, and long timer gaps. It
formats/writes the trace only after the event loop ends. A UTC anchor allows
external owned-process stack samples to be aligned to that clock.

The clean diagnostic `viewer-idle-timeline1` used frozen SHA-256
`3B9CF2A55FDE87AFDCA5301AFD10EB584D4979879BBD448F6829F42C2FDA8FB2`
and observed no compiler overlap. For a source-text idle notch, enqueue occurred
at 7,857.497 ms and native receipt at 7,857.576 ms. An unchanged redraw then ran
from 7,874.235 to 8,372.690 ms (498.452 ms); the UI published the first changed
frame at 8,387.514 ms. An image idle notch reached the native handler in 0.052 ms,
but its first redraw lasted 299.336 ms. Initial CPU stripe sampling took about
0.7–1.3 ms for text and did not account for the stall. Thus the missing time was
inside rendering, not native input dispatch or just backend frame publication.
The stalled notch varied between repetitions.

A second frozen diagnostic with an exact UTC anchor (SHA-256
`57138E7CC6146EA3FD531C8AC51A0E2526CD50E87112F77A81018FFE209B427F`)
was sampled only in its owned main thread. `viewer-idle-stacks2` captured 420
bounded stacks using its matching PDB. Total suspension was 1,521 ms, with a
16 ms maximum per sample; these timings are diagnostic and are excluded from
performance comparisons. The exported CSV timestamps use the host decimal comma;
correlation normalizes that separator before numeric comparison.

Across eight redraw intervals longer than 100 ms, 87 stack samples fell at least
20 ms inside the measured interval. Fifty-two showed
`WGPURenderer::render → Queue::write_buffer → StagingBuffer::new →
CreateCommittedResource → NtGdiDdDDICreateAllocation`. Twenty-six showed
`Slint Texture::new_from_image → FemtoVG alloc_image → CreateCommittedResource`.
The other nine were in allocation destruction during an image-mode transition.
No interior sample showed pipeline compilation. Startup did include shader
compilation, which is a separate phase. The complete interval summary is
`viewer-idle-long-redraw-summary.json`; source traces, stack CSVs, symbols, and
sampling metadata share the `viewer-idle-stacks2` prefix.

This evidence led to a bounded vertex/uniform upload pool in the vendored
renderer; its ownership and fallback rules are documented in
[the vendor patch notes](../vendor/README.md). Four lifecycle regressions first
failed against the no-reuse stub. The implemented pool then passed those four
and all three existing binding/pixel GPU regressions on Intel Iris Xe/DX12
(`viewer-upload-green1.log`, 7 passed). Two further cases verify deferred
commands sharing the same destination across cached/fallback uploads and correct
oversized uncached storage. The complete nine-case run passed in 12.00 seconds
with GPU validation enabled (`viewer-upload-green2.log`). The native measurements below qualify the combined retained-raster/upload
implementation; these functional checks alone do not establish a timing improvement. Texture reuse is deferred because safe retirement must account for
unsubmitted commands, imported resources, reallocation outside `delete_image`,
and the initial-zero semantics of a fresh texture.

The extended diagnostic can select all five fixture types plus the real JSON and
Markdown cases with `--extended-wheel`, or only the real files with `--real-wheel`.
`--sustained` posts 48 down then 48 up wheel notches at 100 ms intervals; `--tail`
seeks to the end first and reverses those directions. Every traced redraw records
the displayed camera and raster/tile upload counters. Continuous reports separate
unchanged-upload intervals from upload transitions, while retaining the frontier
gaps. Deliberate idle gaps in isolated-notch runs are not continuous-FPS samples.

`--gpu-proof` performs a separate post-gesture GPU readback at 5.5 logical-pixel
increments. It compares rendered crop displacement, checks unchanged upload
counts, and verifies that source line numbers stay fixed horizontally while the
body translates. This is explicitly a rendering-arrangement check; native input
and camera behavior are measured by the timed gestures. Any invocation that uses
GPU snapshots is excluded from published timing comparisons, including its later
stages. The candidate sections below record native validation of these modes.

### Retained candidate 1: common paths, still incomplete

The frozen viewer SHA-256 is
`12276F50F9907CEACAAF71D70DBF8D460582C0A4340776C1403B58344AC494C5`;
the panel probe is
`AC58DD5B3C933DACAE5CAD8FCA5CB85820628DDF3BB31D4E729E6E1EE6B8F967`.
Both EXE/PDB pairs are under `retained-candidate1/` in the same QA directory.
This combines the retained-raster frontend/backend with reusable vertex/uniform
uploads. It predates the separately identified wrapped-tab continuity and tiny
fallback reversal fixes. The ordinary text/source fixtures and both real files
contain no tabs. Text/source wrapping was disabled; Markdown used its document
layout (its separate wrap control is unavailable).

A fresh quiet run of the frozen baseline reproduced first-content delays of
380.763 ms for plain text, 393.068 ms for source, and 320.259 ms for images.
Baseline moving-position gaps were respectively 34.359/38.057/38.289,
20.450/27.956/40.365, and 16.144/18.813/19.584 ms (p50/p95/max).
All quiet runs recorded zero compiler/linker overlap. The viewer geometry was
2250×1575 physical pixels at scale 2.25.

The following values use the exact monotonic `redraw_end` clock. Isolated
cadence excludes the deliberately idle intervals between notches. Sustained
cadence includes cache-frontier gaps and excludes only pauses with exact 0/1
file endpoints. The initial probe's loose edge epsilon was incorrect for large
files near the top; `summarize-retained.py` recomputes these results from raw
records without that epsilon, and the source check has been corrected.

| Case | Isolated moving gap p50/p95/max (ms) | Worst isolated input→first movement (ms) | Sustained moving gap p50/p95/max (ms) |
| --- | --- | --- | --- |
| Plain | 16.496 / 17.486 / 17.683 | 332.831 | 16.377 / 32.082 / 552.587 |
| Source | 16.266 / 17.605 / 17.802 | 321.780 | 16.485 / 36.275 / 55.274 |
| Image | 16.549 / 17.313 / 20.950 | 438.827 | 16.613 / 19.200 / 20.312 |
| Markdown fixture | 16.374 / 17.309 / 17.536 | 474.800 | 16.402 / 32.979 / 542.955 |
| Binary | 16.436 / 17.050 / 17.115 | 352.175 | 16.343 / 37.844 / 40.512 |
| Real manifest.json | 16.414 / 17.259 / 18.815 | 467.247 | 16.358 / 45.650 / 53.185 |
| Real Markdown | 16.455 / 17.729 / 36.570 | 276.015 | 16.384 / 35.352 / 47.191 |

The sustained run sent 48 down and 48 up notches at 100 ms intervals per case.
Plain/source/Markdown/binary/realJSON/realMarkdown crossed 74/152/62/111/13/62
upload transitions. Same-upload gaps had p95 around 17.1–17.4 ms, while the refill
p95 values were 38.606/45.190/37.684/39.664/53.185/39.414ms. Real JSON reached
both endpoints. Real Markdown reached position 0.945 then returned to the top;
the separate tail run explicitly sought the bottom and scrolled upward. Every
real-file tail notch produced 13 moving positions, but first-content delay still
reached 477.629 ms for JSON and 384.198 ms for Markdown. The sustained real-Markdown
first notch also waited 518.068 ms before moving; an initial wait is not a gap
between already completed positions and must not be hidden by the cadence table.

The separate GPU proof found 12–13 physical pixels of crop movement for each
5.5 logical-pixel translation. Raster/tile upload counters stayed unchanged.
Source and real-JSON gutters remained pixel-identical horizontally while their
bodies moved; both gutter and body moved vertically. The first proof invocation
exited 1 because the diagnostic wrongly applied the source gutter check to a tiled
image using an inactive leftover gutter-width property. All vertical checks and
the actual source gutter checks passed. The probe now gates gutter checks on
`retained-raster`; the corrected full proof passed on candidate 2 below. These snapshots
are renderer-arrangement evidence, not native input timing or physical display
presentation measurements. All snapshot-run timings are excluded above.

The seeded-icon System32 panel recheck passed at 3840×2241/scale 2.25 on the
60 Hz monitor: 245 moving gaps, p50/p95/max 16.531/20.024/21.947ms (mean 60.15 Hz).
Submission p50/p95 was 14.486/18.253ms; 251 frames used 1078.125 ms of UI-thread CPU.
The fractional-pixel and invalidation checks passed. This remains a direct panel
fixture and does not include concurrent Engine/native-shell thumbnail work.

Artifacts are `viewer-retained-baseline-repeat1`, `viewer-retained-isolated1`,
`viewer-retained-sustained1`, `viewer-retained-real-tail1`,
`viewer-retained-gpu-proof1`, and `system32-retained-upload1`, with their logs,
metadata, summary JSON and pixel directories. The result supports much steadier
movement within cached rasters. It does not qualify uniform 60 Hz scrolling or
elimination of idle and refill stalls.

### Exact candidate attribution of the remaining stalls

`viewer-retained-stacks1` sampled only the frozen candidate's main thread and
resolved its matching PDB. It collected 346 stacks, with 846 ms total suspension
(max 17 ms), so its timings are diagnostic only. Five redraws over 100 ms contained
48 samples at least 20 ms inside the interval:

- 12 samples in a 471.510 ms source redraw reached
  `RenderPassBuilder::new → set_render_target_screen → create_buffer_init →
  CreateCommittedResource`. The viewport uniform/bind-group is recreated by
  every new render-pass builder.
- 13 samples in a 460.844 ms mode-transition redraw reached `Queue::write_texture`
  staging allocation through `update_image` and glyph rendering.
- 7 samples in a 247.129 ms image redraw reached `alloc_image` texture allocation.
- 16 samples in 310.587/333.888 ms plain/image redraws reached
  `UploadPool::begin → device.poll(Poll) → LifetimeTracker →
  FlushedStagingBuffer::drop → destroy_buffer`. Although polling does not wait
  for GPU completion, it can perform expensive synchronous driver cleanup.

The former vertex/uniform `Queue::write_buffer` path was absent from these
interior samples. The remaining ownership paths are concrete and separate; a
viewport cache alone cannot be claimed to solve the texture-upload/allocation
and cleanup paths. The interval evidence is
`viewer-retained-long-redraw-summary.json`, with representative stack numbers
136, 91, 259, 48 and 289. A small immutable viewport-binding cache is the next
approved experiment; texture lifecycle pooling remains deferred.

The viewport cache's two focused regressions first failed on unchanged
per-render allocation (same viewport produced distinct bindings). The immutable
32-entry implementation then passed all 11 GPU regressions in 12.02 seconds on
Intel Iris Xe/DX12 with validation enabled (`viewer-viewport-green.log`). The new
cases check repeated-key reuse, exact float-bit separation, bounded eviction,
and actual GPU reads of original values after eviction and cache drop before
deferred submission. The existing image/binding and upload tests also pass.
Probe edge classification and the inactive image gutter check are corrected in
the following candidate; the earlier frozen evidence is preserved unchanged.


### Retained candidate 2: viewport reuse and adapter comparisons

The frozen viewer SHA-256 is
`DB0C4F38D13B39DB94AF92658CC094E31D9459E411CC1D8D0EC896222E15BD1B`.
Matching EXE/PDBs and the app/panel probes are in `retained-candidate2/`.
This includes viewport reuse and the wrapped-tab/partial-prefix/tiny-fallback
backend fixes, but predates the subsequent RasterRebase handshake. The measured
ordinary text/source fixtures still have wrapping disabled and contain no tabs.
Geometry remains 2250×1575 physical pixels, scale 2.25. All three timing variants
recorded zero compiler/linker overlap and used this exact executable.

| Case | DX12 isolated gap p50/p95/max (ms) | Worst input→movement (ms) | DX12 sustained gap p50/p95/max (ms) |
| --- | --- | --- | --- |
| Plain | 16.592 / 17.494 / 18.862 | 430.857 | 16.312 / 34.578 / 41.358 |
| Source | 16.621 / 17.286 / 17.609 | 423.909 | 16.432 / 38.133 / 539.781 |
| Image | 16.260 / 18.336 / 19.686 | 323.135 | 16.486 / 19.892 / 20.464 |
| Markdown fixture | 16.210 / 16.992 / 17.297 | 488.337 | 16.315 / 31.275 / 40.598 |
| Binary | 16.680 / 17.430 / 19.929 | 526.998 | 16.252 / 37.087 / 41.396 |
| Real manifest.json | 16.456 / 17.597 / 68.138 | 350.152 | 16.370 / 49.078 / 553.790 |
| Real Markdown | 16.637 / 44.286 / 58.908 | 427.724 | 16.324 / 33.667 / 46.961 |

Same-upload sustained gaps have p95 near 17.1–17.4 ms for text-based cases,
but source and real JSON refill transitions still paused for 539.781 and
553.790 ms. The sustained image first movement waited 515.811 ms. Viewport
reuse therefore did not remove the remaining idle/allocation stalls.

The corrected separate GPU proof passed all seven cases. Each 5.5 logical-pixel
translation produced 12–13 physical pixels of rendered crop movement, with
unchanged raster/tile upload counters. Source and JSON gutters stayed identical
horizontally while their bodies moved, and both regions moved vertically.
`viewer-viewport-gpu-proof2.pixels` holds the captures; its entire invocation is
excluded from timing comparisons. Upload counters identify frontend texture
publications, not every internal glyph/renderer allocation.

Explicit Vulkan retained approximately 31 ms moving-position gaps in every
case, compared with approximately 16 ms under DX12. Image and Markdown first
movement still waited 314.033 and 334.171 ms. It was not extended to a sustained
run and does not support a backend-policy change.

Read-only adapter discovery found Intel Iris Xe (driver 32.0.101.7088) and NVIDIA
RTX 3500 Ada Generation Laptop GPU (driver 621.14). Pinned WGPU 30.0.1 disables
DX12 suballocation for adapter names containing `Iris(R) Xe`; its documented
workaround selects committed allocations regardless of memory hints. The
workaround and all driver/security policies remain unchanged.

A child-only `WGPU_POWER_PREF=high`, `WGPU_BACKEND=dx12` isolated run used the
same executable, with `WGPU_ADAPTER_NAME` explicitly absent. A separate,
timing-excluded run under identical settings confirmed its exact owned PID as
an NVIDIA C+G process (`viewer-high-adapter-evidence2-adapter.json`). The adapter
has 12,282 MiB framebuffer memory; WDDM reported its per-process GPU usage as
unavailable. This separate adapter observation supports selection under those
settings; the clean timing process itself was not queried during measurement.

| Case | High preference isolated gap p50/p95/max (ms) | Worst input→movement (ms) |
| --- | --- | --- |
| Plain | 16.462 / 17.172 / 526.336 | 287.021 |
| Source | 16.444 / 17.140 / 17.180 | 193.085 |
| Image | 16.588 / 17.265 / 18.242 | 509.592 |
| Markdown fixture | 16.176 / 17.793 / 508.747 | 21.410 |
| Binary | 16.610 / 522.106 / 1071.669 | 19.725 |
| Real manifest.json | 16.145 / 18.397 / 38.840 | 332.415 |
| Real Markdown | 16.300 / 17.490 / 520.305 | 17.945 |

Several half-second stalls, including the 1,071.669 ms binary gap, occurred
without a changed frontend upload count. High preference is therefore not an
accepted fix and was not extended to sustained/proof runs. Its sampled process
peak was 505.0 MiB private bytes and 327.0 MiB working set; these are CPU process
memory figures, not GPU usage. The earlier default-adapter runs lack matching
memory samples, so no comparative memory claim is made. First-stage readiness
was 2,442.657 ms versus 3,563.999 ms in the preceding default run; single launches
include initialization/cache variation and do not establish a startup benefit.
No shipped GPU preference was changed.

Artifacts are `viewer-viewport-isolated2`, `viewer-viewport-sustained2`,
`viewer-viewport-gpu-proof2`, `viewer-viewport-vulkan-isolated2`, and
`viewer-viewport-high-isolated2`, including raw traces and exact-clock summary
JSON. The data qualifies fractional movement and much steadier cached-frame
cadence, not uniform 60 Hz scrolling or absence of idle/frontier pauses.


### Bounded follow-up feasibility (not implemented)

Slint 1.18 can import `wgpu30::Texture` through `Image::try_from`, avoiding its
CPU-image texture allocation/upload path. The supported public context route
without a rendering notifier is `WGPUConfiguration::Manual`; there is no public
`with_wgpu`/automatic-context getter in the pinned API. Manual initialization
would change current per-window automatic context ownership and must preserve
surface compatibility, device limits, explicit environment overrides and error
fallback. A second device on the same adapter is insufficient.

A proposed experiment could allocate immutable RGBA8 textures and upload them on
a bounded worker using that same device/queue, then publish completed handles
and camera metadata together while old pixels remain available. Imported
textures require `TEXTURE_BINDING | RENDER_ATTACHMENT`, plus `COPY_DST` for
uploads. Device, queue and texture handles are Send+Sync; Slint Image creation
and property changes remain on the UI thread. Cancellation must reject stale
surface/pixel generations, and capacity accounting must include pending CPU
jobs, in-flight uploads, displayed images and retirement. Closing a window or
changing modes must not publish a late image. No displayed or unsubmitted
texture may be rewritten or explicitly destroyed. The RasterRebase
acknowledgement must follow accepted ready pixels rather than CPU-frame arrival.

This is not a proven fix. The imported-image path lacks the premultiplied flag
used by the existing raster/tile buffers; passing them unchanged would multiply
alpha twice. Transparent images need an explicit, pixel-verified solution.
The software path cannot render an imported WGPU texture back to CPU pixels,
so its existing CPU representation remains necessary. Worker-held ownership can
move final texture drops, but Slint still owns views/bindgroups and WGPU's
`Queue::submit` itself invokes `device.maintain(Poll)`. Driver cleanup and shared
queue contention may therefore still reach the UI thread. A small measured
raster experiment and lifecycle/alpha/fallback tests are prerequisites to any
broader image or manual-context implementation; none has been implemented here.

The example-only `--wrapped-tabs` mode selects two dedicated text/source fixtures,
enables wrapping at launch, and sends 48 forward then 48 reverse wheel notches.
Two short prefix lines lead into a tabbed logical line over 50 KiB, exceeding the
plugins' 8192-byte page request bound. Its gate requires observed moving retained
and legacy viewport frames plus reverse movement. All first-frame, wheel-frame
and redraw timeline records identify `retained-raster`, `tiled-image`, or
`legacy-viewport`; per-stage counts make fallback explicit. Its native result is recorded below.


### Candidate 3: wrapped handoff and focus qualification

The frozen viewer SHA-256 is
`4386459CBCE5DA6BF4D92C452A98477B66F3BB1EBA2C0ECB94BBF4E2B425E066`;
matching app/probe EXE/PDBs are in `retained-candidate3/`. This includes the
RasterRebase handoff. `viewer-retained-wrapped3` passed both wrapped-tab cases:
plain/source produced 41/59 moving retained frames and 242/255 legacy viewport
frames while scrolling forward and reversing. Wrapping was enabled and both
legacy and retained movement were required by the diagnostic gate. Raw
continuous within-upload p50/p95 was 31.980/68.165 ms for plain and
30.116/59.819 ms for source, so the fallback-heavy case is not qualified at 60 Hz.

`viewer-retained-proof3` passed all seven GPU pixel predicates, including source
and JSON pinned gutters and unchanged raster/tile upload counts. Its overall
process nevertheless exited 1: plain first notch produced only one content
position. Later six mode gestures passed. This is a successful pixel proof with
a failed cadence gate, not an all-gates pass; all its timings remain excluded.

New read-only observations exposed a material limitation in earlier timing
interpretation. The wrapped and proof runs were visible, unminimized and not
DWM-cloaked, but had zero foreground samples (0/45 and 0/41). The proof's
owned-window `SetForegroundWindow` request failed. Power-throttling control/state
masks were 0/0: the process did not explicitly override them, which does not prove
that Windows never applied a background policy. The older candidate 1/2 traces
lack Focused events and OS foreground samples, so they cannot establish actual
foreground behavior. Stack allocation/destruction attribution remains valid for
those diagnostic runs; its relevance to foreground user interaction is unproven.

Two clean three-mode candidate-3 repetitions then ran in verified foreground:
`viewer-retained-background3` acquired focus naturally (17/18 samples) despite
its historical artifact name; `viewer-retained-foreground3` had 16/17 foreground
samples after an explicit owned-window request. Both also logged Focused(true).
The request returned true in the second run, although its immediate focus check
was false and the UI Automation fallback reported that the target could not
receive focus. Subsequent OS samples establish the actual observed state.
Neither run should be described as a controlled background/foreground pair.

All nine isolated-notch gates passed in each clean foreground run. First-content
latency ranged 17.292–23.505 ms in the first run and 18.031–28.012 ms in the second.
Moving-gap medians were about 16.3–16.8 ms. Raw per-gesture traces still include
late gaps of 62.493/66.556 ms in plain/source for the first run and
61.606/68.504 ms for the second; image maximum gaps were 19.858/22.207 ms.
No 300–500 ms idle stall occurred in either foreground repetition. This narrows
the remaining investigation, but does not prove background state alone caused
all earlier stalls or qualify arbitrary files/settings. Every run above recorded
zero compiler/linker overlap. Metadata stores the owned PID/HWND and bounded
foreground/visibility/cloaking/power samples; no priority or power policy changed.

The next diagnostic supports `--file=<exact-path>` for both wheel and scrollbar
input while preserving actual file extension/plugin routing, and keeps the
existing `--software` option. The scrollbar path currently dispatches Slint
pointer events directly; it proves rendered thumb/content behavior but does not
exercise the native Windows pointer-input hook. Exact user JSON/XML and optional
software behavior require the subsequent targeted qualification.


### Exact user JSON/XML baseline before covered-target seeks

The exact-file diagnostic was frozen with SHA-256
`CDA65C95798E46C9CDBFB7A2F992DFA6BA432A553938937656AEF3D7F8E7F72A`
(`retained-exact-baseline4/`, matching PDB). Inputs were the untouched
`D:/temp/ovclash/results/2026-09-07-mfg/manifest.json` (2659 bytes) and
`C:/Windows/System32/WwanFeatureTests.xml` (3865 bytes). The trusted diagnostic
plugin set selected source mode for JSON and plain text for XML; both began in
retained-raster presentation, with wrapping disabled. This is the reported actual
routing, not an assumption that XML used source highlighting.

All four isolated-wheel runs passed their multiple-position gate. All four
scrollbar runs failed: after 60 pointer moves and release at requested 0.7, the
thumb, position and displayed position ended at 1.0. Both renderers reproduced
this. Context remained stable; final frames used legacy viewport presentation,
with JSON bytes 1860–2659 and XML bytes 2704–3865. Their final-start offsets are
approximately 0.7 of file length, so the discrepancy concerns byte-seek versus
visual-range positioning as well as unnecessary raster replacement. It is not
explained by GPU allocation alone, nor solely by the diagnostic's old requirement
for a newly published backend frame.

| Input / renderer | Worst wheel first content (ms) | Per-notch moving-gap medians (ms) | Longest reported moving gap (ms) | Foreground samples: wheel / drag |
| --- | --- | --- | --- | --- |
| JSON / gpu | 19.703 | 16.324–16.834 | 64.821 | 4/5 / 24/25 |
| JSON / software | 34.965 | 16.015–16.363 | 54.915 | 4/5 / 24/24 |
| XML / gpu | 17.560 | 16.484–16.715 | 17.282 | 4/5 / 24/25 |
| XML / software | 33.450 | 16.095–16.452 | 54.676 | 5/5 / 24/24 |

All runs recorded zero compiler/linker overlap. The first two GPU wheel runs
still attempted UI Automation after a foreground request; that fallback could
block its caller and was removed before the other six runs. No input samples
were silently discarded, and the metadata retains these startup/focus attempts.
Software was explicitly selected and reported as the actual renderer; no GPU
was required for those four cases. Their drag failures occurred with 24/24
foreground samples each. The wheel comparison therefore establishes optional
CPU behavior and the shared drag failure, not a precise GPU/CPU startup ratio.

The subsequent drag diagnostic observes completed displayed-camera positions
without requiring a new backend frame. It retains a specific final target,
requires no pending position and stable context, and checks bounded visible CPU
crops before/after the gesture for changed, nonblank content. Those crop checks
respect retained offsets and are explicitly labeled as CPU evidence, not GPU
readback. The frozen failed baseline remains unchanged for comparison.


### Exact user JSON/XML candidate: eight cases pass

The frozen candidate probe SHA-256 is
`4B00C4B5B9A98B5FA623C48C120D5D3289A1111E806B3166E912DFA9322F7B92`
(`retained-exact-candidate5/`, matching PDB); the combined app SHA-256 is
`80E7D310F755218BDB652B5FE2602608F320372AAE305A155C8F1F4BA1D4D7E6`.
All eight exact JSON/XML wheel and scrollbar cases passed under both GPU and
software rendering. Every case recorded zero compiler/linker overlap. Each drag
now reached 0.70000005, rather than the baseline's incorrect 1.0, and kept retained
presentation. Each final visible CPU crop was changed and nonblank.

| Input / renderer | Drag input→rendered thumb p50/p95/max (ms) | Content updates during 60 moves | Final content after release (ms) | Worst wheel first content (ms) |
| --- | --- | --- | --- | --- |
| JSON / gpu | 6.212 / 12.927 / 14.323 | 47 | 4.655 | 21.781 |
| JSON / software | 15.548 / 18.505 / 19.027 | 45 | 15.792 | 33.710 |
| XML / gpu | 4.851 / 11.386 / 12.676 | 41 | 9.500 | 19.437 |
| XML / software | 16.002 / 38.145 / 66.519 | 45 | 17.522 | 34.041 |

Wheel moving-gap medians remain approximately 16–17 ms. Raw late gaps still
reach 64.315 ms for JSON/GPU, 41.043 ms for XML/GPU, 55.093 ms for JSON/software,
and 54.646 ms for XML/software. XML/software drag also has a 66.519 ms maximum.
The functional and pixel-selection regressions are resolved for these cases;
these measurements do not establish a strict 60 Hz bound for every frame.

Both software wheel runs had 5/5 foreground samples and both software drag runs
3/3. GPU wheel JSON/XML had 5/7 and 5/6 foreground samples; GPU drags had 3/4
each, becoming foreground after their initial samples. All samples and the
owned-window focus-request outcomes are retained; no early gesture or delayed
frame was silently excluded. UI Automation was absent from these eight runs.
The raw Windows wheel messages exercise native wheel delivery. Drag still uses
Slint pointer-event dispatch and the changed/nonblank checks are CPU visible-crop
evidence, not GPU readback. The earlier seven-case GPU proof separately establishes
retained body/gutter arrangement. This distinction is part of the qualification.

Artifacts use `viewer-exact5-{json,xml}-{gpu,software}-{wheel,scrollbar}` with raw
traces, stdout/stderr and foreground/process metadata. `viewer-exact5-summary.json`
collects the small comparison table. The baseline4 artifacts remain intact.
