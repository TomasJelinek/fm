# Viewer startup — 26 November 2026

The owner asked for the startup of every viewer to be measured and made as fast as
possible. This record has four parts: the time from the open request to the first
accepted content frame, by viewer kind and phase; the causes found; the three changes made;
and the costs that remain.

## Method

- **Code measured.** All builds are of main at `04bb387d` plus this work. The branch was
  then rebased onto main at `57eb2602`, which also changes where viewers open
  (`e9767480`); the timings were not repeated on the rebased tree.
- **Machine.** One Windows 11 desktop with 20 logical processors and a discrete GPU, used
  through DX12 with the application's shared WGPU device. The display is 2250 × 1575
  physical pixels at 225%.
- **Shared load.** The machine was shared with other builds. Each summary row lists the
  average CPU load sampled before its runs, mostly 5–20%.
- **Probe.** All numbers come from release builds of
  `crates/midnight-viewer-ui/examples/open_latency.rs`. It opens files through
  `ViewerWindows::open_with_ready`, the application's path, and stops at the first
  accepted content frame. Window, pixel preparation and delivery are included.
- **Placement.** A native Slint parent stays visible (`MIDNIGHT_PROBE_PARENT=1`), and each
  viewer takes the parent's captured placement (bounds, scale, position), as viewers take
  the main window's in the application.
- **Plugins.** Release builds of this tree's basic, source, Markdown, image, PDF and HTML
  viewers and of the video helpers sit in a `viewers` and `thumbnailers` folder beside the
  probe. The private profile folder is under `_build`.
- **Not covered.** The application's own path before `open_with_ready` is not timed: the
  presenter building the launch, and the native filesystem plugin's capture. The probe
  uses the engine's built-in capture.
- **Probe changes in this work:**
  - `MIDNIGHT_PROBE_REPEAT`, `MIDNIGHT_PROBE_SETTLE_MS` and `MIDNIGHT_PROBE_PAUSE_MS`.
  - The parent's placement.
  - An `open-trace` feature. It compiles the engine's phase marks (`open_trace.rs`) and
    the window's opening trace into an optimized build; `MIDNIGHT_VIEWER_OPEN_TRACE=1`
    prints them. The application builds without the feature.
- **Cold and warm.** Each process waits 2 s after start (`SETTLE`), then opens one file
  four times, 2 s apart after each first frame (`PAUSE`). The first opening is the cold
  one, the first viewer after start. The next three are warm.
- **Rounds.** Three rounds alternate the builds in ABBA order, giving 3 cold and 9 warm
  openings per cell; the tables give their medians.
- **Fixtures:**
  - Text: 900 bytes, and a 100 MB log.
  - Source: the 92 KB engine `worker.rs`. Markdown: a 60 KB record.
  - Binary: a 64 KiB file shown in hex.
  - Images: PNG and JPEG at 1920 × 1080 and at 7072 × 4736 (33.5 MP).
  - Documents: a three-page PDF and a 25 KB HTML page.
  - Video: a 640 × 360 uncompressed AVI.

  Images of 50 MP were refused ("image dimensions exceed budget"): the decoder's
  128 MiB decoded limit allows about 33.5 MP. No audio fixture was measured: the
  installed playback package lists no audio-only container that this machine could
  produce offline. Office preview runs in the HTML viewer and was not measured
  separately.

In the cold column, the first launch of a freshly built helper executable is excluded.
In one discarded warm-up run, it took 1.8–3.0 s more, because the system scans new
executables.

## Causes

The phases show that the engine's own work before the window is ready is 2–15 ms for
every kind. The time went to window creation on the UI thread and to the image helper's
package checks. Traced warm openings before the changes (`B0p`, median ms from the
request):

| Kind (fixture) | Component built | Shown | Engine route chosen | Canvas acknowledged | Helper or plugin ready | Engine frame | Accepted |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Text (900 B) | 178 | 178 | 7 | 242 | 244 (plugin session) | 252 | 306 |
| Text (100 MB log) | 177 | 177 | 6 | 242 | 244 (plugin session) | 274 | 305 |
| Source (92 KB) | 170 | 170 | 5 | 246 | 248 (plugin session) | 263 | 317 |
| Markdown (60 KB) | 173 | 174 | 3 | 252 | 254 (plugin session) | 283 | 331 |
| Hex (64 KiB) | 197 | 197 | 3 | 266 | 267 (plugin session) | 292 | 333 |
| PNG 1080p | 187 | 187 | 7 | 260 | 266 (spare claimed 129) | 322 | 326 |
| JPEG 1080p | 182 | 182 | 8 | 248 | 275 (spare claimed 134) | 374 | 378 |
| PNG 33.5 MP | 191 | 191 | 7 | 264 | 367 (spare claimed 126) | 548 | 551 |
| JPEG 33.5 MP | 188 | 189 | 6 | 267 | 365 (spare claimed 125) | 603 | 607 |
| PDF (3 pages) | 178 | 178 | 6 | 246 | 231 (cold helper) | 265 | 311 |
| HTML (25 KB) | 171 | 172 | 4 | 237 | 463 (first browser event) | 640 | 642 |
| Video (AVI) | 188 | 188 | 6 | 266 | started with the layout | 600 | 601 |

Median of three warm openings of one traced run. "Component built" is
`ViewerWindow::new` returning; "Shown" is `show` returning (native creation and the
first frames follow on the event loop); the engine phases run on the viewer's backend
worker in parallel with them. Image and PDF helpers start when the layout is selected.

1. **Adapter enumeration for every window, about 150–180 ms (all kinds).**
   - Constructing a viewer component makes Slint's winit backend create a FemtoVG-WGPU
     renderer. Its `WGPUFemtoVGRenderer::new_suspended` calls
     `any_wgpu30_adapters_with_gpu`.
   - With the application's `Manual` configuration, that call enumerates every adapter of
     every backend of the shared instance (`Backends::all()`), and it does so for each
     window.
   - In a temporary build, setting `SLINT_WGPU_CPU`, which skips that check, brought
     `ViewerWindow::new` from about 170 ms to 0.2 ms. It also brought warm text opening
     from 300 ms to 136 ms.
2. **Two unchanged resizes per new window, about 50–75 ms (all kinds).**
   - Windows delivers two `Resized` events with the window's unchanged size after
     creation. Slint passes each to the renderer.
   - The FemtoVG adapter reconfigured the DX12 swap chain each time: 31–43 ms per event on
     the UI thread, timed with temporary event-loop instrumentation.
   - The window only acknowledges its canvas after them, so text and other content waited.
3. **The image helper's package checks, 120–210 ms each, two or three per opening
   (image and PDF).**
   - `midnight-image-viewer-host` validated its package by checking every one of its
     objects together with every folder above it. The image package has about 390
     licence-notice files.
   - Each opening validates the package when the spare helper is claimed and again
     immediately before Open. A cold PDF helper does so before the spawn as well.
   - The host's startup diagnostics measured 120–210 ms per validation. A ready image
     spare still took about 140 ms from claim to Open, which matches the 139 ms recorded
     earlier for a 2 × 1 BMP.
   - The plugin host already had a single walk that checks the package folder and its
     ancestors once, then each object on its own. The HTML and video packages use it.
4. Not a cause: the layout acknowledgement. The window already acknowledges a selected
   layout in the same delivery that applies it.

## Changes

| Change | Kinds | Before → after (warm median) |
| --- | --- | --- |
| Image host: shared package walk | Image, PDF | PNG 1080p 291 → 122 ms, JPEG 339 → 167 ms, PDF 248 → 113 ms (final code without and with it) |
| FemtoVG adapter: no reconfiguration for an unchanged resize | All | Text 301 → 251 ms, Markdown 316 → 283 ms, PNG 311 → 290 ms, PDF 308 → 275 ms |
| Viewer component built ahead while idle | All | Text 251 → 69 ms, Markdown 283 → 87 ms, PNG 290 → 124 ms, PDF 275 → 112 ms |

**Shared package walk** (`midnight-image-viewer-host/src/providers.rs`).
- The image host keeps its own entry and depth budgets (512 entries, depth 8) and its
  refusal of links and special files.
- It then runs `midnight_plugin_host::validate_plugin_package`: the package folder with
  every folder above it once, then each object on its own (owner, ACL or mode, canonical
  path, regular file or folder).
- The same objects pass the same checks. An object's folder was accepted earlier in the
  same walk.
- Discovery, claim and the final guard before Open all use it.

**Unchanged resizes** (`vendor/i-slint-renderer-femtovg/wgpu.rs`, the already vendored
renderer).
- `WGPUBackend::resize` returns without reconfiguring when the size and present mode
  equal the configured ones.
- A lost or outdated surface is still reconfigured when its next frame begins.
- With temporary event-loop instrumentation, each of the two `Resized` events took
  5–41 µs instead of 31–43 ms.

**Spare viewer component** (`midnight-viewer-ui`, `SpareWindow`).
- What it is: `ViewerWindows` keeps at most one viewer component that has not been
  shown yet.
- When it is built: on the UI thread, 1.5 s after `prepare_spare_window` (the
  application calls it when it installs its viewers) and 1.5 s after each opening takes
  it.
- What it holds: a component only, with no native window, surface, engine session or
  helper process until an opening shows it.
- Release: `release_spare_window` drops it and stops refilling. The application calls
  it on exit, before closing its viewers.
- Effect: the adapter enumeration (cause 1) moves off the opening into an idle moment.
  It still occupies the UI thread for about 150–180 ms once per opening, while nothing
  is being opened.
- No Slint source is changed for it. The owner decided against patching Slint's winit
  backend.

Overall, before → after (cold median / warm median, ms; 3 cold and 9 warm openings each):

| Kind (fixture) | Before (`B0p`) | After (`B3f`) |
| --- | ---: | ---: |
| Text (900 B) | 264 / 311 | 153 / 69 |
| Text (100 MB log) | 296 / 300 | 135 / 89 |
| Source (92 KB) | 317 / 323 | 136 / 78 |
| Markdown (60 KB) | 318 / 297 | 154 / 87 |
| Hex (64 KiB) | 294 / 296 | 150 / 84 |
| PNG 1080p | 307 / 307 | 166 / 124 |
| JPEG 1080p | 484 / 353 | 373 / 164 |
| PNG 33.5 MP | 531 / 521 | 345 / 342 |
| JPEG 33.5 MP | 682 / 576 | 566 / 390 |
| PDF (3 pages) | 464 / 318 | 299 / 112 |
| HTML (25 KB) | 634 / 626 | 505 / 428 |
| Video (640 × 360 AVI) | 676 / 593 | 699 / 584 |

The steps between them for the first four rows (cold / warm medians; `B1p` adds the
package walk, `B2p` the resize change, `B3f` the spare component):

| Fixture | `B0p` | `B1p` | `B2p` | `B3f` |
| --- | ---: | ---: | ---: | ---: |
| Text (900 B) | 264 / 311 | 307 / 301 | 244 / 251 | 153 / 69 |
| Markdown | 318 / 297 | 315 / 316 | 272 / 283 | 154 / 87 |
| PNG 1080p | 307 / 307 | 304 / 311 | 272 / 290 | 166 / 124 |
| PDF | 464 / 318 | 478 / 308 | 409 / 275 | 299 / 112 |

The package walk shows no gain on its own (`B1p`): while window creation took about
250 ms, the image and PDF helpers finished within it. With the final code, removing only
the walk (`B3nw`) made PNG 166 / 122 → 304 / 291 ms, JPEG 371 / 167 → 473 / 339 ms and
PDF 289 / 113 → 387 / 248 ms (three rounds each).

Traced warm openings after the changes (`B3f`):

| Kind (fixture) | Component built | Shown | Engine route chosen | Canvas acknowledged | Helper or plugin ready | Engine frame | Accepted |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Text (900 B) | 4 | 61 | 10 | 64 | 65 (plugin session) | 73 | 79 |
| Text (100 MB log) | 3 | 56 | 8 | 57 | 58 (plugin session) | 87 | 92 |
| Source (92 KB) | 2 | 64 | 8 | 67 | 68 (plugin session) | 83 | 89 |
| Markdown (60 KB) | 3 | 54 | 8 | 55 | 56 (plugin session) | 80 | 85 |
| Hex (64 KiB) | 2 | 50 | 5 | 52 | 54 (plugin session) | 76 | 84 |
| PNG 1080p | 2 | 50 | 9 | 51 | 96 (spare claimed 47) | 147 | 150 |
| JPEG 1080p | 1 | 54 | 6 | 55 | 94 (spare claimed 44) | 189 | 194 |
| PNG 33.5 MP | 3 | 58 | 9 | 60 | 191 (spare claimed 43) | 372 | 376 |
| JPEG 33.5 MP | 2 | 52 | 5 | 54 | 186 (spare claimed 43) | 415 | 419 |
| PDF (3 pages) | 2 | 44 | 7 | 46 | 126 (cold helper) | 148 | 151 |
| HTML (25 KB) | 1 | 52 | 5 | 54 | 271 (first browser event) | 433 | 435 |
| Video (AVI) | 1 | 53 | 6 | 55 | started with the layout | 579 | 581 |

With the spare component, `show` creates the native window and surface itself (about
50 ms) before it returns. Images and PDF are now bound by the helper (decode, then about
40 tiles of 256 px rendered one request at a time), HTML by Chromium, video by the
playback helper.

## Idle cost

- **Spare viewer component:** one at a time.
  - Memory: in the probe, the process's private bytes were 36–39 MB higher with the
    spare built than without it (164–166 MB against 127–128 MB before any opening,
    three runs each).
  - Repeated openings: while eight openings of the same file ran 2 s apart, both builds
    held 212–274 MB of private bytes, and the spare did not accumulate.
  - UI thread: building the spare takes it for one adapter enumeration (cause 1) per
    opening, 1.5 s after that opening.
- **Image spare:** unchanged. One source-free helper while fewer than four helpers are
  live, retired after 60 s unused (see the [image viewer API](../developer/IMAGE_VIEWER_API.md)).
- **No other prewarming was added.**

## Remaining costs

- **Window creation itself, about 60–70 ms warm.** On the UI thread, this is native
  window and DX12 surface creation (about 18–25 ms), the first frame (about 15–25 ms, of
  which about 11 ms is 18 render pipelines rebuilt by each window's own FemtoVG renderer),
  and DWM and message work.
  - Reusing a hidden viewer window would remove most of it, but needs a design for
    resetting every viewer property.
  - Sharing FemtoVG's pipelines across renderers of one device would save about 10 ms per
    window.
- **First viewer after start.** The probe's first viewer also pays Slint's one-time
  component and font setup and the first compilation of FemtoVG's render pipelines
  (about 300 ms in one traced cold run).
  - In the application, the main window has already compiled most of these, so the
    probe's cold column is an upper bound.
- **Adapter enumeration in Slint (cause 1).** It is still paid once per opening, off the
  critical path. Removing it needs an upstream Slint change:
  `WGPUFemtoVGRenderer::new_suspended` (i-slint-backend-winit,
  `renderer/femtovg.rs`) should not call `any_wgpu30_adapters_with_gpu` for a
  `WGPUConfiguration::Manual` request, or should cache its answer. The `SLINT_WGPU_CPU`
  experiment above shows its size.
  - This needs an upstream issue and pull request against Slint. None has been filed.
- **Image tiles.** After the canvas is acknowledged, the first fitted view of a 1080p
  image is rendered as about 40 tiles of 256 px, one request at a time: about 1.2–1.5 ms
  each, about 55 ms in all.
  - Rendering a band of tiles per request, as documents do, would remove most of the
    per-tile engine round trips. It needs no API change, because the host already
    batches documents. It was not attempted here.
- **HTML, about 430 ms warm.** Every page starts its own Chromium with a private profile.
  - A browser kept ready for the next session would remove most of this, but needs a
    design (profile isolation, idle shutdown, revalidation) in the web viewer host.
  - The page session also starts only after the canvas acknowledgement. It could start
    while the window is created, as images and video already do. It would then need its
    first viewport size from the provisional geometry.
- **Video, about 585 ms warm.** The playback helper starts while the window is created.
  The rest is the helper's own opening and first decode, which this work did not change.
- **Text plugins.** A cold Markdown library load and session open takes about 55 ms after
  the canvas acknowledgement. Opening the plugin session while the window is created
  would hide it. It was not done: the window is now shorter than before, and the gain
  falls to 1–5 ms for warm openings.

## Tests

These ran on the rebased tree, in debug test builds unless stated.

- `cargo test -p midnight-image-viewer-host -p midnight-viewer-engine -p midnight-viewer-ui
  --locked --offline` passed:
  - image viewer host: 42 passed, 4 ignored;
  - viewer engine: 671 passed, 20 ignored;
  - viewer UI: 259 library tests and 36 integration tests passed.
- New tests:
  - The image host admits a package with nested licence notices and revalidates it.
  - The image host refuses a junction (Linux: a symbolic link) deep in a package, both at
    discovery and in an installed provider's validation.
  - The image host refuses a package deeper than its budget.
  - An opening takes the prepared spare component and leaves none behind.
  - A released spare is dropped and not refilled.
  - The engine's opening trace is off unless requested.
- `cargo test -p midnight-explorer --bin midnight-explorer viewers::`: 6 passed, 4 ignored.
- `cargo clippy --workspace --all-targets --locked --features midnight-acceptance/acceptance
  -- -D warnings` and `cargo fmt --all --check` passed.
- Not run: the acceptance suite, and the Linux build and timings (Linux timings are
  measured on real hardware).
