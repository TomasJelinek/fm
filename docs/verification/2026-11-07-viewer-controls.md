# Text, binary and image viewer verification — 2026-11-07

## Implementation files

- `crates/midnight-core/src/viewer.rs`,
  `crates/midnight-core/src/shortcuts/catalog.rs`,
  `crates/midnight-core/tests/viewer_shortcuts.rs`.
- `crates/midnight-viewer-render/src/lib.rs`, `src/options.rs`, `src/selection.rs`,
  and `tests/wrapping.rs` within that crate.
- `crates/midnight-viewer-engine/Cargo.toml` and its `src/lib.rs`, `src/tests.rs`,
  `src/worker.rs`, `src/worker_tests.rs`, `src/image_geometry.rs`,
  `src/image_input.rs`, `src/image_integration_tests.rs`, `src/image_pixels.rs`,
  `src/image_view.rs`, `src/image_worker.rs`, `src/worker_image.rs`.
- `crates/midnight-viewer-ui/src/lib.rs`, `ui/viewer.slint`, `tests/keyboard.rs`,
  and `tests/wrap_control.rs` within that crate.
- `plugins/midnight-viewer-basic/tests/protocol.rs`; the plugin implementation
  and its public SDK dependency boundary remain unchanged.
- The workspace `Cargo.lock` records the host's new local path dependency;
  independent process-plugin/API/host sources are verified separately.

## Behavior

- Text and Source have a Wrap checkbox and configurable `viewer.wrap` (`Ctrl+W`).
  Reflow preserves original text, selection offsets and the visible source anchor.
  Resize reflows wrapped content. Binary keeps fixed offset/16-byte/printable columns.
- Existing bounded binary detection remains in the content broker. F3 detects
  binary content; Alt+F3 forces basic Text and permits manual Binary. UTF-8 and
  BOM-marked UTF-16 LE/BE remain text. Malformed text is not automatically assumed
  to be binary solely because it is invalid UTF-8.
- Installed process image providers participate in F3 routing. Missing/disabled
  image providers retain native fallback. Ambiguous native/process identifiers
  disable both candidates before executable code is loaded.
- Image controls offer Fit, actual pixels, zoom, scrollbars, Ctrl+wheel zoom at the
  pointer and drag panning. Search/Wrap/mode controls are hidden for images.
  Escape closes and Space cycles the captured file order. Configurable actions:
  `viewer.zoom-in` (`Ctrl+Plus`, `Ctrl+Equal`), `viewer.zoom-out` (`Ctrl+Minus`),
  `viewer.fit` (`Ctrl+0`), `viewer.actual-pixels` (`Ctrl+1`).
- One image actor per viewer owns the restricted decoder and retained source.
  The host worker remains responsive while decoding. Cycling cancels and reaps
  the previous actor before opening another; closing retains its window permit
  until backend cleanup finishes. Viewport revisions reject obsolete results.
  Image identities reject prior-file actions while preserving drag/zoom bursts;
  resize additionally invalidates old pointer-anchor coordinates.
- Decoder output is converted to premultiplied RGBA on the backend. Navigation
  and zoom use physical canvas pixels independently of bounded raster sampling.

## Verification

Meaningful RED→GREEN checks covered ignored wrap width, unbroken Unicode words,
invalid wrap geometry, host reflow/selection preservation, configurable wrap and
image shortcuts, alpha conversion, physical image sizing with a 4K canvas, and
two rapid zoom steps from one displayed frame.

Windows targeted suites use `scripts/cargo.ps1`, `--locked --offline`, profile
debug information disabled and incremental compilation disabled. Final registry:
`_build/image-viewer/releases/worker-sdk-1794037042914624500/.cargo/registry.toml`.
Linux uses `_build/qa/interaction/linux-verification-env.sh` in Ubuntu-24.04 WSL.

Evidence logs under `_build/`:

- `viewer-wrap-tests.log`: core, renderer and engine baseline/wrap suites.
- `viewer-wrap-plugin-tests.log`: independent basic plugin, 12 unit and 15 protocol
  tests, including binary search across read boundaries in both directions and
  contiguous bounded pages for a huge unbroken Unicode line.
- `viewer-wrap-linux-render.log`, `viewer-wrap-linux-plugin.log`: Linux renderer
  and basic plugin suites; the latter also contains strict plugin Clippy.
- `viewer-image-ui-controls.log`: actual Slint button, checkbox, drag and Ctrl+wheel
  dispatch, hidden text tools, and focus restoration.
- `viewer-final-tests.log`: final Windows renderer/engine/UI suites.
- `viewer-image-burst-green.log`: Windows engine suite after the rapid-input fix,
  23 passed and 2 native tests deferred to the explicit native run.
- `viewer-final-shortcuts.log`: five viewer shortcut tests.
- `viewer-image-linux.log`: Linux engine/UI suites.
- `viewer-final-linux.log`: Linux engine suite after the rapid-input fix and final
  strict viewer Clippy.
- `viewer-final-native.log`: trusted Windows native/process integration, including
  F3/Alt+F3 routing, Unicode, search/cancel, source replacement, image fit/zoom/pan,
  obsolete inputs, cycling, cleanup, disabled providers and duplicate identifiers.
- `viewer-final-linux-native.log`: corresponding Linux native/process integration
  run, using the trusted deployment
  `/home/tjelinek/_build/MidnightExplorerViewerQA/viewer-integration-KqpzCX`.
  Basic and image packages reuse existing Linux builds; missing Source and
  Markdown packages were built in debug mode against the released native SDK,
  using their independent locks and offline cache.
- `viewer-final-clippy.log`: strict Windows viewer Clippy.

Final results: the engine's 23 regular tests and both explicitly enabled native
integration tests passed on Windows and Linux. Both platforms also passed the
19 renderer tests, 6 viewer UI tests, 27 independent basic plugin tests, and
strict viewer/plugin Clippy. Five focused shortcut tests passed on Windows.
The final Linux native run completed in 19.20 seconds; its first text viewport
took 52.7 ms, the 20 MiB search took 17.51 seconds, cancellation plus cycling
took 40.8 ms, and worker cleanup took 5.1 ms. These are local acceptance-run
observations during concurrent UI/build activity, not a controlled throughput
benchmark or release performance baseline.

Windows native fixture diagnosis observed a freshly generated BMP's ChangeTime
advance about 150 ms after creation while its file ID, length and LastWriteTime
stayed unchanged (`viewer-native-revision-evidence.log`). The external cause is
unknown. Production LiveValidated correctly rejected this metadata revision.
The test now waits for 500 ms of unchanged stable version, bounded by 5 seconds,
before capturing the generated fixture. No application retry or revision guard
changed. Both final native tests passed serially (`viewer-final-native.log`),
including rapid-input and duplicate-provider checks.

The independent basic plugin still imports only the released public SDK. No
parser dependency, public native plugin ABI, application license, sandbox policy
or live application installation was changed by this viewer integration.

## Current limits

Text copy/selection remains limited to the loaded page; file scrollbars retain
the existing approximate byte-based extent. Wrap is remembered within the viewer
window and is not persisted as an application preference. Image output is limited
to 4096 pixels per edge and 4 MP; larger physical canvases scale a bounded surface,
preserving image size and navigation but reducing fine pixel detail. Image formats,
first-frame animation behavior, decoder budgets and source guarantees are owned
by the independent image viewer package and its host supervision protocol.
