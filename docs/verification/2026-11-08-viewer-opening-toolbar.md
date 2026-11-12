# PDF opening and Markdown toolbar

New PDF sources open in Fit window, enlarging small pages and shrinking large
pages. Changing pages keeps the current manual zoom exactly. Markdown typography
controls occupy one row.

The existing default preference was already Fit. The failure came from restoring
the saved document zoom during the worker's PDF Opened event and on late settings
synchronization. Relative UI zoom also used that saved value instead of the
displayed scale. The fix preserves accepted wheel bursts, source/frame guards,
and the existing page-change geometry contract. Raster image preferences retain
their previous reopening behavior.

The existing schema remains compatible. Saved document zoom values are still
decoded and explicit edits may still update that field; the value no longer
restores the view of a PDF. New sources start Fit; page navigation retains the
runtime view. No schema rewrite, preference-file access, dependency update, or
plugin API change is part of this work.

The responsive typography row retains the preset, editable family, generic-family
choices and size. Compact controls use a generic-family dropdown, preserving
keyboard access to Sans, Serif and Mono.

## Verification

Evidence is under `_build/goal/viewer-opening-toolbar/`:

- `engine-red.log`: saved PDF zoom overrides Fit; raster preservation already
  passes. `zoom-ui-red.log`: displayed 8% with old saved 500% produced a wrong 625%
  first zoom step. `toolbar-red.log`: row centers were 26/26/57 pixels at 900px.
- `engine-final.log`: 70 passed, four native/package tests explicitly ignored,
  0.17 seconds. New/late-restored PDF policy, raster restoration, exact page
  geometry, document enlargement, 64× maximum and bounded raster remain covered.
- `pdf-plugin-acceptance.log`: real sandboxed PDF package passed, 8.68 seconds,
  using the current engine test executable and protected runtime
  `C:\Users\tjelinek\MidnightExplorer-20261108-064756-80106a72\me.exe`.
  A saved 234.56% preference still opens the actual 400×600 PDF page in Fit at 20%
  in a 160×120 viewport. Actual pixels and exact manual scales survive page turns
  in both directions across differently sized pages. Resizing to 1200×1200
  enlarges the first page to 200%; page turns retain Fit at 120%/200%.
- `fit-upscale-red.log` and `fit-upscale-ui-red.log`: small-page PDF Fit incorrectly
  stopped at 100%, and zoom after pending Fit used the raster-image cap. Document
  geometry now retains its enlargement policy through page changes and resizing;
  the presenter uses the same 64× document/1× raster Fit bounds.
- `zoom-review-red.log`: independent review reproduced a second race. After
  requesting 125% then 100%, an old 100% frame and intermediate 125% frame caused
  the next step to become 156.25% instead of 125%. Numeric scale equality is not a
  command acknowledgment. Local accepted intent now remains tied to source,
  page and viewport until context replacement/cancellation; failed enqueue never
  replaces the prior accepted intent. Raster preference restoration clears it.

- `ui-upscale-final.log`: all 48 UI tests passed (31 unit, 17 integration), covering
  zoom bursts, old/intermediate frames, rejected enqueue, source/page replacement,
  raster restoration, pending Fit parity, scrollbar/keyboard behavior, immediate
  preferences and the actual responsive typography widgets.
- `markdown-toolbar-900.png` and `markdown-toolbar-320.png`: rendered headless
  widgets with the production dark-palette call, inspected by implementer and root.
  Preset, family, generic choices and size occupy one row without overlap. At
  320px the custom-family editor shows only a short portion of its text; its full
  editable value remains accessible, and the adjacent generic-family chooser is
  readable. This is a layout/readability check, not native compositor evidence.
- `strict.log`: `cargo clippy -p midnight-viewer-engine -p midnight-viewer-ui
  --all-targets --locked --offline -- -D warnings` passed, 12.62 seconds.
  Direct rustfmt checks and focused `git diff --check` are clean. Every touched
  handwritten source file remains below the 100,000-byte limit.

Initial UI runs exposed a Slint layout dependency warning and the test's missing
intermediate native ComboBox selection event; the input-size property removes the
layout cycle. Pinned Slint's popup Up/Down handler invokes `selected` on each arrow,
so Mono→Serif→Sans intentionally publishes two font changes before Return closes
the popup. The test asserts the exact immediate callback sequence.

Commands used `scripts/cargo.ps1`, `--locked --offline`, development/test debug
information disabled and incremental compilation disabled. There are no dependency
or public plugin ABI changes. The initial strict invocation lost an unquoted
PowerShell `--`; its preserved log is an invocation error, not a Rust lint result.
The corrected invocation passes its arguments as an explicit array.

The final engine test binary is
`_build/debug/deps/midnight_viewer_engine-7e3da46575a43eb7.exe`, SHA-256
`94C725E054AF6A8EF6B8C60F12E2E96F9E4784247BD31155832E62AA5172F60B`.
All owned build/test sessions are terminal and the Windows Cargo lane was handed
to the shared-toolbar task. No normal application preferences were modified and
no commit or push was made by this task.

## Scope

Production changes are limited to viewer-engine `image_geometry.rs`,
`worker_image.rs` and `worker.rs`; viewer-ui `image_zoom.rs`, `lib.rs` and
`ui/viewer.slint`. Worker and native PDF tests cover source opening and geometry;
UI presentation, widget and acceptance-harness tests cover actual control routing
and rendered layout. The scripted Opened constructor in `image_worker.rs` exists
only under `cfg(test)`. Developer preference/presentation documentation describes
the compatible saved-document-zoom policy and compact generic-family chooser.

The main app build remains a root-coordinated gate. This focused work does not
claim a full application restart, hardware-compositor measurement or native GUI
acceptance from a headless widget test.
