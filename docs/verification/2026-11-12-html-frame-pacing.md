# HTML frame pacing — 12 November 2026

The pinned CEF window settings limited `OnPaint` to 30 frames per second. The
candidate raises that ceiling to 60 while retaining latest-frame coalescing,
viewport memory limits, document generations, process containment and software
rendering. This is a ceiling, not a claim that every document renders at 60 fps.
No dependency or protocol changed.

## Native comparison

Six quiet runs used the same Windows native probe, owned script-free `wheel.html`,
3015 × 1845 physical parent at 225% scale and 2705 × 1550 content viewport. Each
opened successfully and completed 12 isolated wheel gestures. The probe waits for
300 ms of changed-frame quiescence between gestures; these measurements describe
wheel-to-first-published-redraw latency, not sustained animation frame cadence.
No Cargo or other native benchmark ran concurrently.

| Renderer / sample | 30 fps ceiling median | 60 fps ceiling median | Change |
| --- | ---: | ---: | ---: |
| D3D, first | 113.397 ms | 106.006 ms | 6.5% lower |
| D3D, second | 119.660 ms | 100.564 ms | 16.0% lower |
| Software | 128.208 ms | 116.488 ms | 9.1% lower |

The largest D3D observation decreased from 154.470 ms to 122.051 ms. The gain is
useful, but residual latency around 100–116 ms remains visible and is not described
as instant or equivalent to mobile scrolling. The software sample exercises the Slint software renderer; Chromium retains its
own rendering selection, so this is not a test with GPU hardware disabled. This
setting adds no GPU requirement. Linux and continuous frame cadence need separate
qualification for this change.

The qualified 30 fps baseline DLL SHA-256 is
`C00EB47E6352DB612D61E2DB0D875E58B75F9890A12A1F884E4589F8012C5603`.
The 60 fps default-build candidate DLL SHA-256 is
`5AEE763A9E02A9A80149FF91869A100D1E0E861089B3B12F3F8F73386CD22DF4`.
Neither contains the optional diagnostic-timing feature. The isolated measurement
installation was restored to the exact baseline after every sample, including its
package inventory. The user's qualified installation was not modified by this test.

Local evidence is under `_build/qa/html-performance-20261111/`:
`run-frame60-ab.ps1`, `frame60-native-summary.json`, `frame60-*-run.log`,
`native-frame60-*.stdout`, `native-frame60-*.stderr`,
`frame60-build.log`, and the retained `provider-frame60-candidate` package files.
The independent package build, all 13 provider tests (8 unit and 5 policy/surface
integration tests), strict all-target Clippy and formatting checks passed. The first
Clippy attempt lacked Ninja on PATH; its failed environment log is retained as
`frame60-clippy-missing-ninja.log`. Adding the same installed Ninja path used by the
normal package builder resolved that toolchain setup failure; no source change or
lint suppression was needed. Cargo still reports the existing CEF semver-metadata
manifest warning.

See the [preceding output and pixel-conversion measurements](2026-11-11-html-performance.md)
for the other measured bottlenecks and historical first-open outliers.
