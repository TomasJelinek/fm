# Markdown viewport continuity

The reported file is a 5,692-byte Markdown report with paragraphs and several
tables. The previous cache test returned directly to one exact source anchor;
it did not exercise overlapping wheel requests all the way from the bottom.

## Corrections

The plugin previously clipped paragraphs and table cells at arbitrary source
anchors. A later viewport therefore reshaped a different paragraph suffix and
measured a different subset of table rows. Bounded semantic windows now retain
complete paragraphs and table groups around an anchor. The host restores the
requested source position inside that layout. Text/source/binary paging remains
independent. When complete context cannot fit the existing byte/block budget,
scalar-safe paging remains available and must advance overlapping requests.

Nearby requests also reuse one parsed context (at most 1 MiB), in addition to
the existing 16-page/1 MiB final-page cache. This retains already-read content;
it does not preload the file or remove bounded worker/cancellation behavior.
The independent cache regression reduced 40 reads to the original two while
requesting different anchors in both directions. Large-file traversal verifies
that old context is evicted and can be reconstructed without losing content.

Backward reads reserve room for following parser context within the existing
64 KiB read bound. Otherwise a table near that bound could be presented as a
complete table before its remaining rows were read. BOM handling, zero-length
horizontal rules, exact source anchors and the hard-byte-budget progress cases
have regressions. Markdown soft source line breaks now become spaces; explicit
hard breaks remain visible newlines.

The [font/table report](2026-11-09-markdown-font-tables.md) records GitHub typography,
column sizing, stripe order, border and contrast verification. No dependency,
external stylesheet/font or plugin ABI change was introduced.

## Evidence

Logs and generated artifacts are under `_build/qa/markdown-continuity/`.
`plugin-red.log`, `host-red.log`, `softbreak-red.log`, `context-red.log`,
`window-red.log` and `progress-red.log` record causal failures before their fixes.
The complete independent plugin suite and strict all-target Clippy pass in
`plugin-final.log` and `plugin-clippy.log`.

`markdown_scroll` is a read-only production-worker probe: it opens the supplied
file through installed plugins, scrolls to the bottom and back twice, compares
the overlapping pixels after every movement and requires identical pixels at
the beginning after each round. It writes images and timing records beneath the
chosen `_build/` output directory. Unlike the old test, requests use overlapping
wheel anchors, not just cached page starts. Native probe results and the deployed
application hash are recorded below when the final candidate is qualified.

The first two actual-file runs also caught a separate renderer defect: preserving
a later source caret across different preceding content accumulated about
0.000011 physical pixels of floating-point error. Conservative raster bounds
turned that into an extra row of table-border pixels. The focused renderer
continuation regression reproduces the same cancellation error with a fractional
heading prefix; exact top-page tests alone had missed it.

Restoring the initial document margin exposed another transition at a different
wheel phase: the viewport began six pixels before a heading. The new fragment
started with that heading, so clamping its negative origin removed the gap and
moved the content by 78 pixels for a 72-pixel request. Contextual request anchors
now retain both sides of a gap without changing the preserved reference caret.
Forward/backward, rule/EOF, exact-line and synthetic quote/list-prefix regressions
cover the boundary selection. The
actual-file probe also accepts wheel-step and viewport dimensions to check more
than one alignment of page boundaries.

A 17-pixel wheel step then exposed one remaining H2 separator scanline changing
at a page boundary. Final paint coordinates now normalize only sub-millipixel
near-integer cancellation error, including root borders and rounded fills.
Quarter- and half-pixel scrolling remain distinct. The heading regression fails
before this correction and passes afterward.

## Final Windows verification

The final independent Markdown plugin suite passes all 55 tests. The renderer
passes 40 tests, the viewer engine 95 (four existing installation-dependent tests
remain ignored), and the viewer UI 51. Strict all-target Clippy passes for the
plugin and the three host viewer crates. Logs are `plugin-final.log`,
`heading-border-green.log`, `host-final-green.log`, `plugin-clippy.log` and
`host-final-clippy.log` under the evidence directory.

The final native probe executable SHA-256 is
`146562FE90CB234AFD923E033902A8283F756CDA2A1786CE71558544B34870AE`.
It uses the installed Markdown DLL with SHA-256
`5B487CBBCAE2F09CE766EEE159815FA77B548F80A8E063C341F2C6FA3E373ECE`.
Each configuration traversed the supplied report down and up twice:

| Viewport | Wheel step | Frames | Changed overlap pixels | Frame latency p50 / p95 |
|---|---:|---:|---:|---:|
| 1000 × 600 | 72 px | 172 | 0 | 2.62 / 9.11 ms |
| 1000 × 600 | 17 px | 728 | 0 | 2.37 / 5.00 ms |
| 1000 × 600 | 97 px | 128 | 0 | 2.37 / 7.55 ms |
| 700 × 500 | 43 px | 312 | 0 | 2.18 / 5.34 ms |

All four runs also returned exactly identical initial pixels after each complete
round. Logs/images are `probe6-step72`, `probe6-step17`, `probe6-step97` and
`probe6-narrow`. Timings include the production plugin/worker/renderer and frame
delivery to the probe; they do not measure Slint input dispatch or composition.
These are local Windows results, not Linux/macOS qualification.

The application was rebuilt in the debug profile and installed with the matching
private runtime, including the corrected Markdown and Network plugins. The
launcher `_build/debug/me.exe` has SHA-256
`C3895CBC031D39272FE3683B6650ED18BBE5B5E7B4858CC8426C0B9804AEBA7F`.
The native UI smoke completed with exit code 0, no stderr and a rendered snapshot;
saved preference hashes were unchanged (`app-smoke.json`, `app.png`). The main UI
also passed 668 unit tests and the five final tree bridge/panel integrations;
strict package Clippy passed. Existing warnings in unfinished engine capture
code remain outside the scoped UI lint qualification.
