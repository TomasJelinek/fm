# Markdown scrolling and GitHub presentation — 9 November 2026

Three independent-plugin regressions reproduced the reported instability:

- An eight-line viewport returned 120 headings because only literal LF characters
  were counted, although a semantic heading already occupies a displayed line.
- More than 256 blocks later in the parsing lookahead downgraded the visible
  heading to literal text.
- Six repeated up/down cycles reread/reparsed source: 22 reads instead of the
  four needed to prepare the three initial pages.

Semantic cropping now counts block lines. Lookahead complexity no longer changes
the visible page's mode, and parsing/checkpoint discovery share one parser pass.
A revision-local LRU reuses up to 16 pages/1 MiB. Cache hits do not read the source;
new sessions never reuse the previous revision's entries. Cancellation and eviction
have regressions. Uncached distant seeks without a trusted block checkpoint still
use the existing literal fallback; this is not a whole-file Markdown index.

GitHub presentation now uses semantic block geometry instead of only styled flat
text. H1/H2 rules, neutral hierarchy, inline/fenced code backgrounds, quote borders,
list indentation and striped content-sized tables support light and dark colors.
User font selection and source anchors are preserved. The host does not execute
HTML or fetch remote content. No dependency, copied CSS or plugin ABI was added.

## Evidence

Artifacts: `_build/qa/markdown-scroll/`, `_build/qa/markdown-style/`.

- Independent Markdown plugin: **46 tests passed**, strict Clippy passed. Three
  causal failures are recorded in `red.log`; full results in `second-green.log`.
- Windows host routing/readiness baseline: **86 viewer-engine tests passed**, four
  existing installation-dependent tests ignored; viewer UI **34 passed**. Later
  combined host verification is recorded in `host-final.log`.
- Linux GitHub page integration: **5 tests passed** after refreshing a stale API
  build artifact. Renderer tests and strict Clippy passed before the independent
  table-pagination review correction; its dedicated RED/GREEN logs record that fix.
- Light/dark 800×844 fixture PNGs were visually inspected. Warm raster benchmark:
  about **2 ms/frame** across 20 fractional scroll origins with reused layout/glyphs.
  This measures backend raster work on this machine, not end-to-end display latency.

The native acceptance example now includes a long Markdown document, seeks away
and returns to the start, and requires identical semantic text **and pixels** before
exercising all three presets and font controls. Its final native result is recorded
separately from the unit-test evidence; compiling a probe does not count as running it.

## Final Windows native result

The rebuilt probe ran successfully from a new private installation with the new
Markdown DLL (SHA-256
`166fd88774727d37b40bb77c280567654a4bee5101ad29b78d327080430675bc`).
All 13 native stages passed in about seven seconds, including identical text and
pixels after leaving and returning to the first Markdown page, presentation/font
controls, text/search/binary/source, image zoom, PDF page changes, placement and Escape.
Logs: `native-result.json`, `native-acceptance-full.err`; window snapshots are under
`native-acceptance/`. The software renderer used real Windows windows and installed
plugins, not a mocked parser. The first sandboxed attempt was refused by Windows
ACL inspection; the successful run used normal user permissions with checks intact.

Final Windows host suite: **87 engine tests passed, 4 existing external-fixture tests
ignored; 33 renderer tests passed; 34 viewer UI library and 17 UI integration tests
passed**. Main UI: **653 passed**, with **3** viewer-association callback tests passed.
Both the semantic-cache and table-boundary fixes were independently reviewed.

The updated Markdown DLL is also deployed with the final main debug application.
Native startup tests passed both from its new private installation and from
`_build/debug/me.exe`, using exact-build runtime registration. Both exited 0 with
empty stderr and rendered panel snapshots; preferences were unchanged. Deployment
and startup records are in `_build/qa/make-file-list/`. Strict all-targets Clippy
also passed for the modified host UI/core/common and viewer packages.
