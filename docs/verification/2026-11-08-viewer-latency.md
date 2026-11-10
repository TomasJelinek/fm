# Viewer renderer latency — 8 November 2026

The original debug baseline requested the application's Windows
FemtoVG-WGPU/Direct3D policy, but **actually used Slint's software renderer**.
It must not be described as a GPU measurement. The viewer-controls report has
been corrected. This investigation preserves bounded input, source coordinates,
image tile identity/budgets, PDF zoom and backend-only source/render work.

## Causal evidence

At 2250×1575 physical pixels and 225% scale, the instrumented scrollbar pass
separated pointer dispatch, retained redraw and content redraw. Pointer dispatch
median was 0.04–0.07ms. Plain/source retained redraw medians were 0.35/0.42ms, but
content redraws cost 136/125ms. Image redraws cost 135–139ms even without new tile
uploads; only four of 58 image redraws uploaded tiles. Thus additional source
reads, decoding or copying new tiles did not explain the dominant shared cost.

Windows WPR could not enable system-profile policy (0xc5585011); no recording
started. A local helper then sampled only the freshly launched QA process's main
and busiest worker thread instruction pointers. Every brief suspension was paired
with resume in `finally`. The existing local PDB resolved the samples without
symbol downloads. The main thread consumed 19.1 CPU seconds; dominant functions
were software pixel blending (398 samples), texture fetch/blend (146), winit's
software pixel blend (109) and `Rgba<u8>::clone` (105). This sampled run is not
used for latency claims.

Pinned Slint 1.18.1 source explains the unexpected fallback:
`any_wgpu30_adapters_with_gpu` handles an explicit WGPU30 configuration or no
graphics requirement, but returns false for the generic Direct3D requirement.
Later WGPU initialization supports Direct3D, but the preflight never reaches it.
The focused native regression initially reported
`requested_d3d=true actual_renderer=software` and failed.

## Fix and feature audit

The Windows default now supplies an explicit WGPU30 `Automatic` configuration
with DX12 to both adapter discovery and initialization. The Linux policy,
explicit renderer/environment overrides, OLE window hook and software fallback
remain intact. The probe uses the same configuration and checks the initialized
renderer. With the two enabled renderers, the pinned software implementation
reports no transformation support and FemtoVG reports support; a GPU probe fails
if it receives software instead. No rendering notifier is installed, because it
changes the FemtoVG flush path. Timing uses the existing event-loop boundary after
redraw submission, not compositor presentation.

Development profiles optimize the existing software-renderer, winit and rgb
pixel loops at level 2. Release optimization and dependency versions are unchanged.
This keeps fallback rendering useful on systems without a supported GPU.

The app enables Slint's `unstable-wgpu-30` API feature, and the viewer example
enables it through its dev dependency. WGPU 30.0.1 was already pinned and selected.
All 610 pre-existing package version/source/checksum pins were preserved. Feature
resolution added two packages to the complete lock: `android-build` 0.1.4 (MIT)
and `i-slint-backend-android-activity` 1.18.1 (the existing Slint Royalty-free 2.0
branch). Their actual license files were inspected. Neither package nor Skia is
selected by the Windows or Linux normal/build trees; these are resolved optional
platform edges. Existing WGPU's MIT license and rgb 0.8.53's MIT license were also
inspected. Slint's existing attribution/distribution obligations still apply.
Root maintains the shared complete inventory and distribution notices.

## Unloaded native results

Both final runs pass the real scrollbar gestures with all build queues idle,
unchanged authored fixtures and the same 2250×1575/225% geometry. The GPU run
reports **actual_renderer=femtovg-wgpu**; the explicit fallback run reports
**actual_renderer=software**. Each stage sends 60 pointer moves and two reversals,
requires content progress during the gesture, and verifies the final target.

| View | Before: actual software p95 (ms) | After: actual GPU p95 (ms) | After: optimized software p95 (ms) |
| --- | ---: | ---: | ---: |
| Plain text | 184.052 | 15.939 | 63.010 |
| Source | 197.504 | 30.878 | 69.930 |
| Image | 146.852 | 23.457 | 19.826 |

The before column uses the split-profile repeat (`profile-before.txt`); the
earlier policy-only report measured 184.709/203.402/141.925ms and is consistent
with the same software bottleneck. These are individual debug runs, not a
statistical confidence interval or release benchmark.

The same-software comparison isolates the pixel-loop improvement: median content
redraw fell from 136.370→15.813ms for plain text, 125.164→14.448ms for Source and
139.195→15.773ms for images. The final GPU run had 33/36/54 content updates during
dragging and final content readiness of 58.160/84.267/20.134ms respectively. Image
tile uploads remained 55, so the improvement did not come from skipping requested
content or increasing retained tile capacity. No source-read timing or count
claim is made: the latency runs deliberately omit read instrumentation.

The dominant 125–140ms redraw stall is removed. Distant Source seeks still need
asynchronous page work, and this debug run does not establish uniform 60Hz or
physical display scan-out latency. No production viewer engine, PDF zoom,
coordinate indexing, image cache or input-acknowledgement code changed.

## Verification and integration gate

Artifacts are under `_build/goal/viewer-latency/`: split profiles, owned-process
samples and local symbols, the GPU-selection RED, metadata/tree reports and the
pre-feature lock snapshot. The optimized renderer build and both unloaded native
runs pass. The final locked/offline viewer gate passed 67 engine tests (four
environment-dependent native tests ignored), 19 renderer tests and 42 UI tests.
All-target Clippy with `-D warnings` passed for those three crates against the
final features and profiles. Logs are `final-viewer-tests.log` and
`final-viewer-clippy.log`. The previously completed native controls/PDF acceptance
is recorded in [the controls report](2026-11-08-viewer-controls.md).

The subsequent [application cutover](2026-11-08-native-filesystem-cutover.md)
built and deployed this renderer configuration, passing native/software startup
and all-format viewer backend acceptance. The application and probe use the same
Windows WGPU30/DX12 configuration. The final tested probe's
SHA-256 is
`1119747AA61A9828D946F9C0D59E8FCD259B955262BCFDAE03019C1BD8A897CC`.
