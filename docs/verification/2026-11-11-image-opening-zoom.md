# Image opening and zoom, 11 November 2026

This targeted pass addresses the reported opening-size jump and slow image zoom.
The supplied `Screenshot 2023-04-18 143311.png` is an opaque 607×204 PNG,
209,543 bytes. The original was opened read-only and left unchanged. No plugin
protocol or decoder dependency changed.

## Causes and correction

The first backend request used logical window dimensions with an approximate
48-pixel chrome deduction. Later requests used the physical native canvas.
Image tiles were normalized to their original viewport but displayed across the
new canvas, briefly changing their pixel size. The viewer now obtains initial
geometry after native realization and renders each accepted image frame using
its explicit physical dimensions. Compact chrome and late DPI changes cannot
stretch a 100% image. Rejected frames cannot change the retained viewport.

Zoom also submitted transparent letterbox tiles to the restricted decoder, with
two captured-source validations per tile. The engine now prepares fully
out-of-image tiles locally, retaining unique tile IDs, full camera coverage and
the existing allocation budget. Guaranteed-transparent tiles retain camera metadata
but do not allocate presentation textures or create image draw rows. This removes
blank-space uploads and rendering on both GPU and software. Boundary/intersecting
tiles still use the worker; cached-only or transparent-only frames still need
current-revision validation.

Magnification no longer asks the worker to create enlarged pixel copies. Raster
sampling is capped at exactly source resolution while camera surface identity
still changes with the display zoom. Cached pixels and their texture IDs survive
100%→125%→156%→381% and higher zooms; their new presentation coordinates apply the
magnification. Downsampling and PDF rasterization retain their previous paths.

## Backend measurements

Debug source-module probe, the application's installed native filesystem
provider, the installed restricted image worker, and the supplied PNG. Seven common
scales were exercised at two viewport sizes. Zoom did not reread or decode the
source again in either version. The final run additionally checks 1164.14% zoom.

| 2250×1400 physical viewport | Before | After |
| --- | ---: | ---: |
| Decoder tiles across common scales | 77–88 | 0–3 |
| Source validations | 154–176 | 1–6 |
| First complete visible scene | 42–61 ms | 0.23–9.2 ms |
| 100%: decoder tiles | 88 | 3 |
| Initial 100%: visible scene ready | 57.3 ms | 9.2 ms |

Warmed magnification uses zero decoder requests and one source validation: 0.23–0.35 ms
at 2250×1400. Source-buffer and texture reuse, not merely a faster decoder, removes
the repeated work. Returning after a different downsampling level may require
source tiles again after bounded cache eviction.

These isolate backend work; they are not complete desktop input latency.
Logs and the disposable probe source are under
`_build/qa/image-zoom-20261111/backend-{before,retained}.log` and `backend-probe.rs`.

## Native method

The reusable `midnight-viewer-ui` example `performance_probe` accepts the PNG via
`--file=<path>`. An isolated, protected QA installation contains the same viewer
packages as the application. It never reads or writes application preferences.

```text
performance_probe <qa-directory> <report.txt> --count --d3d --file=<image>
performance_probe <qa-directory> <report.txt> --count --software --file=<image>
performance_probe <qa-directory> <proof.txt> --d3d --image-proof --file=<image>
```

The timing run invokes the real toolbar zoom callback six times inward and six
times outward, measuring command submission through completed native redraw.
It does not include OS mouse-message delivery. Revisited scales are not assumed
to remain cached. Snapshot/readback runs are separate and excluded from timing
comparisons. Both versions use debug, the same image, 225% display scaling and a
2250×1575 physical window. GPU mode is the application's FemtoVG/WGPU D3D12 path;
software is explicitly selected, rather than inferred from an unavailable GPU.

## Native results

The final same-machine run, with builds/deployment stopped and no synthetic load.
The probe reports the upper middle sample as the median for its twelve observations:

| Renderer, 12 zoom steps | Before median | After median | Before maximum | After maximum |
| --- | ---: | ---: | ---: | ---: |
| FemtoVG/WGPU D3D12 | 179.669 ms | 16.018 ms | 214.985 ms | 21.136 ms |
| Software | 130.009 ms | 17.041 ms | 167.198 ms | 21.956 ms |

This is approximately 11.2× / 7.6× lower median command-to-redraw latency for the
reported image. The final run retains exactly three uploaded image textures
through all twelve steps, rather than recreating zoomed pixels. Large-image
first decode and filtered downsampling are separate workloads; these numbers do
not claim that every image or operation completes in one frame.

Separate native pixel captures verify opening geometry. Previously the image
first occupied 1329×445 physical pixels, then shrank to 607×205. The final software
run occupies 607×204 from first paint through settling and returning from zoom.
The final GPU run occupies 606×204 on first paint and 607×204 afterwards (a
one-pixel edge rounding difference at 225% DPI), without the enlargement jump.
The initial and settled capture positions agree.

Evidence: `retained-{before,after}-{d3d,software}.txt` and
`proof-retained-{d3d,software}.txt` plus their `.images` directories under the QA
folder above. Snapshot timings are not included in the table.
## Regression checks

The software framebuffer regression failed on the old code: a 128-pixel image
initially occupied 249 pixels before settling to 128. It now checks both the image
and a distinct internal marker at 2× DPI, with normal and compact controls.
Focused coverage also checks rejected opening cleanup, stale-frame geometry,
panning/scrollbars, PDF zoom/page boundaries, source validation, tile memory
ownership, fractional image edges and transparent scenes. Full workspace suites
are deferred under the project's targeted-test iteration policy.

The final targeted Cargo run passed 28 engine image tests and 27 UI image tests
(0.17 s and 1.06 s test execution, excluding compilation/process startup). The
installed-runtime test remains opt-in. Three pixel-ownership/allocation tests and
five opening/lifecycle tests passed in the preceding focused checks.

The magnification regression checks texture reuse, exact physical destination
geometry and solid marker pixels. Its software screenshot expectations explicitly
account for the pinned Slint renderer's 8-bit fixed-point reciprocal: at 3×, a
128×96 source block covers 386×290 sampled pixels, while its mathematical display
extent remains 384×288. This renderer rounding does not affect the 100% opening
regression. No vendor renderer change or broad pixel-tolerance increase is used.
## Build and startup

The final debug `me.exe` and native probe build succeeded with the pinned offline
SDK configuration. The application executable is `_build/debug/me.exe`, SHA-256
`fb063c1ab89e7920897d1673523bbcbf0b4f22a2e5dd09ae3964934263a229c4`.
Its matching private plugin runtime was deployed and registered; filesystem and
thumbnailer packages are included. The application startup smoke exited 0,
produced its native snapshot, and emitted no stderr.

`cargo fmt --all --check`, `git diff --check`, and the source-size check passed
(1,888 source files, all at or below 100,000 bytes). Logs remain under `_build`;
no test PNG, generated binaries, snapshots or private installations belong in Git.