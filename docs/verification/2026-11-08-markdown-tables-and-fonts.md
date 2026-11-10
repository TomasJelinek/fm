# Markdown tables and installed font chooser — 8 November 2026

The Markdown viewer now offers installed font families and renders GitHub-style
pipe tables. The independent plugin previously used the default CommonMark
parser, leaving table syntax as paragraphs; the host also lacked semantic cells.
Both boundaries are implemented without coupling the plugin to application
source. The selected Markdown family and size use the existing per-mode viewer
preferences, independently of Text and Source.

## Behavior and boundaries

- Font discovery runs on the viewer worker, is bounded and cached. The compact
  toolbar uses a keyboard-accessible installed-family ComboBox. Saved generic
  families and unavailable literal family names remain valid selections.
- Tables have shared columns, bold headers, borders, left/center/right alignment,
  wrapped Unicode text and empty cells. Narrow windows retain readable column
  widths with horizontal overflow. Inline code uses a monospace family while
  prose uses the chosen font. Selection, hit testing and copying retain the
  existing exact source-anchor contract.
- Strikethrough and read-only task lists are supported. Tasks use `[x]` / `[ ]`:
  native testing found missing ballot-box glyphs and unwanted emoji in Calibri.
- Public document blocks carry optional validated cell/strike metadata. Old
  payloads remain valid; old hosts can ignore the optional metadata and show row
  text. Limits are 64 columns, 4,096 cells and 4,096 strike ranges per page, within
  existing text/wire limits. Host retained-memory accounting includes metadata
  vector capacities. No new third-party dependency or version was introduced.
- Reads remain bounded to 64 KiB, emitted text to 8,192 bytes, with existing
  checkpoint/parser limits. Very large tables and distant uncached contexts can
  use the documented readable source fallback. HTML stays inert; remote images,
  fonts, mathematics and diagrams are not rendered/fetched.

See [presentation](../developer/VIEWER_PRESENTATION.md),
[wire contract](../developer/VIEWER_PROTOCOL.md) and
[plugin limits](../../plugins/midnight-viewer-markdown/README.md).

## Tests and build evidence

All logs below are local, ignored artifacts in `_build/qa/markdown/`.

| Gate | Result | Log |
| --- | --- | --- |
| Released public API tests | 25 passed | `api-tests.log` |
| Plugin host, renderer, viewer engine and UI libraries/integration tests | 198 passed, 9 explicitly ignored | `host-viewer-tests.log` |
| Independent Markdown plugin, all targets | 40 passed | `plugin-final.log` |
| Independent Basic viewer consumer | 27 passed | `midnight-viewer-basic-tests.log` |
| Independent Source viewer consumer | 27 passed | `midnight-viewer-source-tests.log` |
| Independent conformance package | Builds/tests successfully; no standalone tests | `viewer-conformance-tests.log` |
| Markdown strict all-target Clippy | Passed | `plugin-clippy-final.log` |
| Main debug application and native examples | Built | `build.log` |
| Final independent Markdown release package | Built | `plugin-build-final.log` |

The tests include Unicode/escaped/empty cells, alignment, malformed metadata,
source ranges, cropping in both directions, sequential paging, bounded distant
table fallback, selection geometry, font catalog/persistence/input and memory
accounting. Causal failing tests were observed for the original literal table
rendering (`plugin-red.log`) and portable task markers (`task-marker-red.log`)
before the fixes. The nine ignored host/engine tests require their separate
fixture/runtime qualification; they are not reported as passing here.

Plugins consumed exported API/SDK packages through the generated local registry,
without source-tree patches. Four independent consumer lockfiles were refreshed
for the changed internal draft API/SDK package checksums; third-party lock entries
and versions did not change. Rustfmt, scoped whitespace checks and the repository
100,000-byte source-size gate passed. The app build still reports 39 existing
unused inline-tree foundation warnings; this is not a full-workspace CI claim.

## Native Windows acceptance

The final Windows `winit-software` probe passed all 13 stages in 5.30 seconds and
exited with code 0. It exercised text/binary/source/Markdown/image/PDF routing,
actual font/preset/size input, re-rendering, placement, navigation and Escape.
`native/final/stderr.log` and `result.txt` record the run. The source and Markdown
font controls changed rendered pixels without changing accessible content.

`native/final/06-markdown.png` was visually inspected: Calibri 17, Comfortable
preset, three aligned columns, bold header and Unicode cell (`Café 世界`), right
aligned 42, centered monospace `ready`, empty cell, strikeout and legible task
markers. Compact 320- and 900-pixel toolbar captures were also checked. An earlier
probe stall was a harness focus error: after operating the ComboBox, the harness
now explicitly focuses content before sending the file-cycle Space shortcut.
Failed evidence remains in `native/`; the final evidence is in `native/final/`.

The rebuilt application startup smoke exited 0, produced `startup.png`, wrote no
stderr and left the saved preferences hash unchanged (`smoke-result.json`).
The normal application was reopened successfully with its existing settings;
`reopened.json` records the live main window and executable hash.

## Runtime

Application: `_build/debug/me.exe`, SHA-256
`BD1D34108DC30FFBC1F7F1227AABC6B3AAE9A257A03B62FA7634F1165E17E3A3`.

Final independent Markdown DLL SHA-256:
`92D7A43655962F1C445D38AB720DBB0861EDC5531A0453497A330766D5DD7F87`.

The matching private runtime is registered at
`C:\Users\tjelinek\MidnightExplorer-20261108-212102-75f7a701`.
Plugin/ancestor trust checks remain enabled. This is a debug application for
iteration with a separately optimized plugin, not a newly published release.
