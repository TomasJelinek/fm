# Viewer controls — 2026-11-08

Scope: VIEW-02 source numbering/current logical position and Markdown presets;
VIEW-03 presentation of the already persisted viewer font/preset/number choices.
No external dependencies or settings schema changes were needed.

## Implementation

- Source mode shows a default-on line-number checkbox, separate prepared gutter
  annotations, and current logical line/column in the status. Soft wrap does not
  change numbering; the selection endpoint determines the current position.
- Exact positions use a backwards-compatible optional public request to the
  independent source plugin. Bounded, cancellable indexing does not participate
  in viewport completion or scrollbar acknowledgement. Older plugins remain
  usable and explicitly report unavailable positions after distant navigation.
- Markdown has GitHub (default), Compact and Comfortable presets with distinct
  line spacing and reading widths. Body fonts and semantic heading sizes reflow
  together. Source text, source anchors, selection and clipboard text stay intact.
- Source/Markdown expose font family, generic family shortcuts and 8–48 logical
  pixel size controls; plain text opens the same typography controls with Aa.
  Preference patches reach the existing listener immediately. No query, content
  or reading position is persisted.

Contracts: [presentation](../developer/VIEWER_PRESENTATION.md),
[preferences](../developer/VIEWER_PREFERENCES.md),
[optional position API](../../api/SOURCE_POSITION.md).

## Automated evidence

Commands use the existing Windows cache, `--locked --offline`, dev/test debug
information disabled and incremental compilation disabled. Logs are under
`_build/goal/viewer-controls-completion/`.

- Red: presets all produced the same 91.99-pixel layout; line-number changes
  produced identical rasters; the typography widget was absent.
- Green: 67 engine unit tests, 26 viewer UI unit tests, 16 UI integration tests,
  and 18 renderer integration tests (plus one renderer unit test).
- UI integration includes actual pointer controls at 320 pixels, source and
  Markdown changes followed by immediate Escape, manager recreation/restoration,
  independent font settings, invalid size rejection, search input and scrollbar
  regressions. Existing image/PDF viewport and input tests remain green.
- A native popup regression emitted `[1, 0, 1]` for two intended preset advances.
  The selected callback incorrectly returned content focus before the popup
  closed. Keeping native popup focus fixed it; the real keyboard test now emits
  `[1, 2]`.
- A native mode-switch check also exposed a stale SpinBox after source size
  editing: the widget retained 15 while Markdown's stored size was 16. A real
  widget-value regression failed before the fix. Editable controls now use
  two-way bindings so restoration updates their displayed values without
  invoking save callbacks. The full eight-widget suite passes.
- The separately built source DLL passes 27 tests. Public API/SDK tests (56),
  host tests (49), strict Clippy and fresh C/Rust ABI layout checks pass. Details
  are in [the API verification report](2026-11-08-source-position-api.md).
- Viewer renderer/engine/UI strict all-target Clippy passes. Source size check
  reports every handwritten file below 100,000 bytes.
- A real plugin/engine integration test searches a 20,000-line Unicode/CRLF
  source fixture and receives **Ln 15001, Col 13**. Removing the gutter changes
  pixels while retaining text and exact copied match `row 15001`.

## Native evidence

The updated source package is staged in the authorized protected QA runtime
`C:\Users\tjelinek\MidnightExplorer-20261108-041855-1aaca13e`.
Its DLL SHA-256 is
`09ECFA256E1C509F8B4B08820456A50C31E7761315EED814DF98422CD33BEE68`.
Uniquely named acceptance/performance executables use owned `_build` fixtures,
never application preference files or normal app launches with test paths.

Native source toggle, typed family and size changes pass observable raster checks.
Native preset changes reach Compact and Comfortable, and Markdown family/size
changes update content pixels. The captured source and Markdown windows were
visually inspected at 225% display scale. The final combined `winit-software`
acceptance run passes all 13 stages in 16.7 seconds, including text search/wrap,
binary/text switching, the new controls, image zoom/1:1, PDF next/previous pages,
placement and Escape. Evidence: `native-software-complete.log` and snapshots in
`native-software-complete/` under the evidence directory. The image-stage harness
now waits for the post-toolbar viewport geometry before sending initial zoom;
the production stale-geometry guard remains unchanged.

The final unloaded scrollbar probe requests **winit / FemtoVG-WGPU / Direct3D**,
matching the Windows application's then-default renderer/API policy. Subsequent
CPU sampling and an actual-renderer assertion established that Slint silently
fell back to **software rendering** for that request; these measurements are
software results, not GPU results. The follow-up is recorded under
`_build/goal/viewer-latency/`. The probe
passes all three stages with 60 pointer moves each, continuing content updates
during dragging and reaching the final target. Both build queues and other native
QA windows were idle. The debug executable used a 2250×1575 physical window at
225% scale. Evidence: `scrollbar-app-renderer.txt` and `.log`.

| View | Median (ms) | p95 (ms) | Final content (ms) | Content updates during drag |
| --- | ---: | ---: | ---: | ---: |
| Plain text | 135.888 | 184.709 | 185.073 | 29 |
| Source | 4.987 | 203.402 | 181.540 | 26 |
| Image | 135.998 | 141.925 | 136.387 | 54 |

Timing is input to the event-loop boundary after redraw submission, using the
same nonintrusive observer approach as the existing native resize probe. It is
not compositor presentation latency. The earlier notifier-based probe produced
incomplete image evidence, and the first explicit D3D notifier attempt returned
`Unsupported`; those results are not directly comparable with this observer.
The run accepted its initial ActualPixels command on the acknowledged
image surface without a retry. Preparation now has a 12-second diagnostic
timeout instead of waiting for the overall run deadline. No production image
zoom, PDF geometry or image scrolling changes were needed.

These results establish working controls, continuous content progress and final
scroll completion. They do not establish a release latency improvement or a
200-ms latency guarantee. Source reads were deliberately uninstrumented, so the
zero read counters in this latency report do not claim absence of I/O.

## Combined application deployment

The Windows debug application rebuilt successfully after the final production
control changes. The source DLL above and existing compatible viewer/archive/
thumbnail packages were deployed to a new protected runtime:
`C:\Users\tjelinek\MidnightExplorer-20261108-053905-dd9620fe`.
The hash-bound registration also permits launching `_build/debug/me.exe`.
Executable SHA-256:
`951653FD8802C9A91BE2F42743A7475296C0F9883A4A631870A0B1470C4FFF7C`.

The actual debug build's isolated native startup probe passed (exit 0, empty
stderr). Real saved preferences were unchanged, SHA-256 before and after:
`BCF7388CC3C3E2742FD86FEBF6F91A3EDAC29FB312FDFC6BC054492BCBBAB053`.
Evidence: `app-build.log`, `deploy.log`, `app-acceptance.txt` and
`app-native.{out,err}.log`. A fresh real-PDF navigation test also passed against
this installation, preserving exact manual zoom and adaptive Fit on pages of
different sizes (`pdf-page-zoom-installed.log`). Its initial sandbox-account run
was correctly rejected by the package-owner check before loading any plugin;
the successful native-account run did not relax that check.

Final workspace formatting, whitespace and all 1199 handwritten source-file size
checks pass. Live application preference/restart qualification remains separate:
an isolated startup with unchanged preferences, manager recreation and strict
codec tests must not be described as saving settings across a native app restart.
