# Image opening and catalog startup, 11 November 2026

The supplied `Screenshot 2023-04-18 143311.png` (607×204, 209,543 bytes) was read
without modifying it. Larger existing PNGs were also read without conversion or
copying. This pass changes startup scheduling only: decoder plugins, pixel
sampling, source validation, package trust, renderer selection, tile ownership,
and request-generation checks remain unchanged.

## Attribution

The reusable `midnight-image-viewer-host` example `image_startup` accepts a trusted
application executable and an existing image path. It runs three separate decoder
sessions and reports catalog discovery, captured-source opening, the first source
validation, source-read timing and counts, decode completion, and bounded tile
requests. The source is the normal `CapturedFile` boundary; these host-only
measurements exclude the native filesystem plugin and UI presentation.

With the full private application package set, the supplied small PNG measured:

- Image catalog discovery: 89–92 ms.
- Sandboxed session open: 60–82 ms, including 29–32 ms of fresh package validation.
- First broker read: 57–79 ms from session-open entry.
- Decode tail after the last broker read: 1.5–1.7 ms.
- A 256×256 source-resolution tile: 0.69–0.86 ms on repeated passes.

The decode tail does not represent complete decoder CPU time: formats can decode
incrementally between broker reads. The full open duration includes all decoding.

A 3287×1820 PNG (10,093,010 bytes) required 2.69 s for its first captured-file open
on its OneDrive-backed path, before decoder startup. Resident opens took 209–215 ms for the decoder
session, with 40 bounded reads. A 7280×2160 PNG (25,679,406 bytes) opened in 233–275
ms with 99 bounded reads. Its first attempt was correctly rejected because the
captured revision changed; the subsequent fresh capture succeeded. No validation
was removed to hide that condition.

The existing `startup_phases` example now accepts an explicit application path.
Its serial attribution showed approximately 1.4 ms for font discovery, 32 ms for
native viewer discovery, 85 ms for image/PDF discovery and 160 ms for HTML discovery.
All catalogs were previously loaded consecutively before a source could open.

## Change

`Service::catalogs` retains the existing caches and validators, and starts the
independent image and web discovery operations on two scoped backend threads while
the initiating worker discovers native viewers. Existing cache locks serialize
cold initialization across viewer windows, so one service creates at most two
temporary discovery threads. Hot opens return the same cached catalog objects.
The UI requests one-shot preparation before constructing the native viewer, so
catalog validation can overlap expensive native/GPU window initialization. One
shared atomic admission latch permits only one speculative preparation task per
service, including concurrent/repeated requests and failed openings. That task
contains no source capture or pixel work. Content still starts after `show()`
provides the correct native viewport and monitor scale.

All discovery work joins before returning, including failure. Routing still sees
all three catalogs together, preserving detection of duplicate identities across
transports. There is no new cross-operation cache of security decisions.

A proposed optimization inside package trust was rejected by automatic approval
review and was not applied. The final production change retains those validators
unchanged. A regression confirms that another package validation still rejects
changed file or ancestor permissions.

## Native comparison

The before and after `performance_probe` executables use the same complete private
runtime, including image, PDF and HTML packages. The test runs from a realized
native parent, matching the application's multiwindow path, with a 2250×1575
physical viewer at 225% DPI. Reports measure viewer-open request through the first
completed native redraw containing the image. No screenshots or GPU readback are
performed during timing runs; saved preferences are neither read nor written.

For the small PNG, two quiet software-rendered comparison passes measured
415.301/416.468 ms before and 320.168/325.747 ms after: approximately 22% less
opening latency. Both versions read exactly two bounded chunks (419,086 bytes)
and perform 18 source validations before first presentation. The final zoom run
retains three texture uploads and has a 16.245 ms median over 12 steps.

Initial D3D measurements varied from 641 to 1178 ms. They do not establish a D3D
opening improvement. The final attribution run measured 825.340 ms for synchronous D3D viewer
construction, first prepared-frame publication at 1190.833 ms and completed native
draw at 1205.466 ms. The comparable software run measured 4.689 ms, 324.626 ms and
344.921 ms respectively. Thus the D3D native window construction precedes decoder
startup and dominates that run; this is not a 1.2-second PNG decode or texture
upload. The pinned Slint WGPU Automatic path constructs GPU resources for each
window, so sharing those resources would require a separate backend/device
lifecycle change.

The production parallel discovery helper measured 198–219 ms for the complete
three-catalog phase, compared with approximately 277–286 ms when run serially.

## Verification and artifacts

The concurrency regression failed against serial execution, then passed with two
scoped workers. It checks that all independent catalogs can start before the
native discovery completes and that failure waits for the other tasks to finish and release
their ownership. Sixteen concurrent preparation requests schedule exactly one task.
Focused checks passed: three integrated startup/admission tests, seven viewer opening and
retirement tests, 28 engine image tests, one
shared font-catalog test, and one package permission-revalidation test. The
installed decoder integration test remains opt-in and is not included in that
count. Broad suites are deferred according to the repository's iteration policy.

Raw timings, before/after binaries, native reports, test logs and the native-run
script are under `_build/qa/image-loading-20261111/`. The profile commands are:

```text
image_startup <trusted-application-executable> <existing-image>
startup_phases [--parallel] <trusted-application-executable>
performance_probe <qa-directory> <report> --count --native-parent --software --file=<image>
performance_probe <qa-directory> <report> --count --native-parent --d3d --file=<image>
```

## Remaining GPU startup work

Read-only review of the pinned Slint 1.18.1 sources found a supported reuse hook:
`BackendSelector::require_wgpu_30(WGPUConfiguration::Manual { instance, adapter,
device, queue })`. The application's configuration point is
`crates/midnight-explorer/src/window_backend_windows.rs::configure`. Winit clones
that configuration for each renderer. In `i-slint-core/graphics/wgpu_30.rs`,
`async_init_instance_adapter_device_queue_surface` uses the supplied four objects
for Manual and creates only a new surface at that layer; Automatic creates a new
instance, chooses a surface-compatible adapter, and requests a device and queue.
`i-slint-backend-winit/renderer/femtovg.rs::WGPUFemtoVGRenderer::new_suspended` also
performs the adapter preflight for each window. There is no public setter to
replace the configuration after backend selection with handles from the main
window's rendering notifier.

Manual reuse is not a complete guarantee of eliminating the measured native
construction cost. `i-slint-renderer-femtovg/wgpu.rs::configure_surface_from_init_result`
still creates a new FemtoVG renderer/canvas for each surface. The current vendored
`femtovg::renderer::WGPURenderer::new` creates that renderer's shader module,
layouts, buffers and pipeline cache. The timing above measures the combined
construction path, rather than attributing every millisecond to device creation.

The next bounded experiment is a diagnostic-only Automatic-versus-Manual probe
that constructs several viewer windows using identical graphics settings and
records device setup, each native construction and first redraw separately.
Before adopting Manual in the application, preserve the existing backend/environment
choices, GPU flags and limits; validate surface compatibility on hybrid/multiple
monitors and concurrent windows; and qualify shared device-loss behavior plus the
current software recovery that only restarts before first reveal. Initial adapter
selection currently happens with an actual native surface. Selecting a shared
adapter without that surface is a real lifecycle change, not just a cache flag.
No renderer foundation, vendor renderer or GPU configuration was changed in this
image-loading pass.

## Final diagnostic reruns and remaining qualification

The final native pair is variable and does not establish a stable additional gain
from catalog prewarming. With staged probe BA1C17F2A58661BF, software first redraws
were 538.423 ms and 372.729 ms; the contemporaneous original software probe measured
499.155 ms. D3D measured 800.430 ms (247.948 ms construction, 775.860 ms publication).
These runs are separate from the earlier quiet 415/416 to 320/326 ms pair above.

One software invocation (`full-after-software-5`) returned exit code zero without
recording a first frame. It retained one source read and four validations. Its old
probe did not retain opening status, so its cause remains unknown. The probe now
reports opening completion/error, last observed status and an explicit error when
the event loop returns before all first-frame records. It also records parent close
requests and a disappearing viewer; an incomplete run can no longer silently pass.

The refreshed probe's software and D3D pass 8 both reported `opening_result=Ok(())`
and completed all twelve zoom measurements. Software construction took 8.414 ms,
but publication took 10450.750 ms and first redraw 10473.675 ms. The following D3D
run took 230.826 ms construction, 566.619 ms publication and 632.096 ms redraw.
Both had two source reads, 419086 bytes and eighteen source validations at first
frame. Zoom medians were 17.097 ms and 16.123 ms. The ten-second startup outlier is
not explained by this count-only instrumentation. Per-source-operation duration
logging was added for the next probe build to distinguish source I/O/validation
from catalog, worker or preparation delays. Neither the old missing-frame run nor
the long startup outlier is considered resolved.


Pass 9 added source-operation durations. Software first redraw was 1304.139 ms
(construction 9.702 ms, publication 1281.364 ms); D3D was 1244.082 ms (construction 713.526 ms,
publication 1131.295 ms). Both opened successfully. Initial source capture took
0.35–0.41 ms, reads 0.25–0.94 ms, and individual validations 0.12–0.52 ms. Source I/O
therefore does not explain these runs' ~1 s pre-publication delay. The probe now
also timestamps source events relative to viewer opening to locate whether the
remaining delay precedes capture or follows it. That additional instrumentation
is built as the separately staged `image-loading-timeline.exe`, SHA-256
`e134087ac0247bb23ff1df7b554da033c4c610ce241df2a8d3d161ebea0ac94c`.
The coordinated quiet samples are recorded below. The earlier 10.5 s and
missing-frame observations are not counted as successful timings.

## Fresh source timeline and explicit no-frame rejection

The first quiet software invocation (`full-timeline-software-1.stderr`) captured
the original read-only OneDrive screenshot at 224.488 ms. Its first 209543-byte
read at 1137.474 ms failed with
`opening_result=Err("Source changed since capture (LiveValidated revision)")`.
The diagnostic remained open without a frame and the owned-process runner timed
out and closed it at 45 seconds. This is a failed sample, not a startup timing.
No source, ACL or validation policy was changed. The exact changed revision field
was not captured; the path being in OneDrive does not prove the cause of the change.

A fresh capture then succeeded in software and D3D, each with two reads, eighteen
validations before the first frame, and all twelve zooms. At 2250×1575 physical
parent size and 225% scale:

| Stage, ms from viewer open | Software retry | D3D |
| --- | ---: | ---: |
| Native construction returned | 157.811 | 641.852 |
| Source capture began | 227.696 | 642.288 |
| Validation before decoder process startup | 270.568 | 676.608 |
| First source read completed | 1356.891 | 711.557 |
| Last validation before first publication | 1377.474 | 733.234 |
| First UI publication | 1384.482 | 1154.680 |
| First completed redraw | 1405.279 | 1170.538 |
| Median zoom redraw, twelve samples | 16.502 | 16.810 |

The software gap occurs after `ImageSession::open` validates the source and before
its first brokered read; it spans restricted-process startup, Ready/Open transport
and the first request, before the decoder has those image bytes. In D3D, a separate
421 ms interval remains between the last source validation and UI publication.
These observations locate remaining work; they do not isolate every millisecond
within startup/presentation or justify changing the renderer or security boundary.
The original quiet ~22% software discovery improvement remains a separate result.

The explained rejection has the same one-read/four-validation fingerprint as the
historical no-status pass 5. This makes that guarded rejection path a plausible
explanation for the older missing frame, but its historical cause is not proven.
The earlier ten-second outlier likewise remains unexplained. The final sample logs
are `full-timeline-software-2` and `full-timeline-d3d-1`; the bounded runner checks
opening success, a first-frame record and all twelve zoom results, and reaps only
its own probe on failure. Qualified production packages were unchanged by these runs.

## 2026-11-12 restricted-worker startup attribution

The opt-in `startup-diagnostics` feature on `midnight-image-viewer-host` enables
fixed numeric stage timings in the host and `midnight-worker-platform`. It is off
by default. Each operation collects at most sixteen marks and writes one line
when it finishes; it records no paths, environment values, source bytes or wire
messages. The platform-specific startup paths and all containment/validation
ordering remain unchanged.

A quiet host-only run used the existing small PNG and the image provider beside
`MidnightExplorer-20261112-progress-final-0cad3979/me.exe`. No source or plugin
package was modified. Six opens across two fresh host processes completed with
the same 607x204 metadata, two reads and 419,086 brokered bytes per open.

| Stage duration | First cold open | Next five opens |
| --- | ---: | ---: |
| Fresh provider validation | 28.114 ms | 26.891–28.374 ms |
| AppContainer profile setup (including SID derivation) | 12.923 ms | 8.820–13.596 ms |
| `CreateProcessW` | 2,394.882 ms | 11.090–22.318 ms |
| Ready wait after broker starts | 5.847 ms | 3.846–4.615 ms |
| Open sent to first Read request | 0.081 ms | 0.072–0.136 ms |
| First Read request to I/O permit | 0.010 ms | 0.003–0.010 ms |
| Complete host open | 2,445.347 ms | 54.906–72.210 ms |

This directly locates the reproduced multi-second cold delay inside native
process creation. It is not a source-read, broker-permit or Ready-handshake
stall. The native call can include OS/loader/security-product work; this
measurement does not identify Defender or any other specific component as the
cause. The prior D3D publication delay is a separate boundary and is not measured
by this host-only example.

Starting a source-free decoder earlier could overlap native window construction,
but would require a bounded per-opening ownership/geometry handoff. It would hide
little behind the usual approximately 5 ms software-window construction. No
speculative decoder pool, cached package authorization, sandbox change or
unsupported derive-only profile shortcut was introduced for this measurement.
Microsoft documents profile creation as a per-user data-store operation and SID
derivation as obtaining the named profile's SID; deriving a SID alone is not
proof that the required profile was created. See
[CreateAppContainerProfile](https://learn.microsoft.com/en-us/windows/win32/api/userenv/nf-userenv-createappcontainerprofile)
and
[DeriveAppContainerSidFromAppContainerName](https://learn.microsoft.com/en-us/windows/win32/api/userenv/nf-userenv-deriveappcontainersidfromappcontainername).

Build: `cargo build -p midnight-image-viewer-host --example image_startup
--features startup-diagnostics --locked --offline`, with the existing pinned SDK
config and `_build` target directory. Invocation remains
`image_startup <trusted-app> <existing-image>`. Evidence is under
`_build/qa/image-startup-20261112/`: `diagnostic-build-final.log`,
`host-before.stdout/.stderr` and `host-repeat.stdout/.stderr`. This is attribution
evidence, not a claimed startup speedup.

Focused qualification passed with both configurations: Windows ordinary host and
worker-platform libraries 16/16; enabled containment/resource-policy tests 4/4;
enabled host library tests 6/6 and actual image-session tests 3/3, including
retained pixels, source changes, corruption, cancellation and deadline while
waiting for an I/O permit. Strict all-target Clippy passed for both crates with
diagnostics enabled. Command durations were 6.221, 11.219, 7.138 and 2.251 seconds,
respectively. Native Linux library checks passed with the feature off and on
(34 and 16 seconds, one build job); no Linux runtime-containment test was claimed
for this diagnostic-only change. Exact commands and logs are retained in
`qualify.ps1`, `qualification-summary.txt`, `qualify-linux.sh` and
`linux-summary.txt` in the same QA directory. Independent review found no change
to process/resource ownership, security ordering or feature-off runtime work.
