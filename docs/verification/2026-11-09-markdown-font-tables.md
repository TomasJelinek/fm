# GitHub Markdown font and table corrections

The focused Windows renderer and font-policy checks pass. Whole-document plugin
paging and host scrolling are tracked separately in the
[Markdown continuity report](2026-11-09-markdown-continuity.md).

## Causes and changes

Selecting the GitHub preset previously changed only its spacing enum. A custom
monospace family or large size survived selection, and the generic `sans-serif`
default did not select Windows' native UI family. GitHub now defaults to, and
explicit selection restores, `system-ui` at 16 logical pixels. Later explicit
Markdown font edits remain supported. Text/source font choices remain independent;
restoring saved settings does not reset a saved custom font.

The policy lives in `ViewerPreferences::apply`, shared by immediate viewer
acceptance and application persistence. Early application preferences now retain
the concrete family/size effects of the preset reset while coalescing patches in
causal order. This handles both GitHub → custom font → GitHub and GitHub → Compact
before initial preference loading, without discarding unrelated loaded fields.

GitHub tables previously compressed columns toward an arbitrary two-em minimum.
Long numeric tokens could split across lines. Columns now use the actual shaped
minimum/preferred content widths, with the same header styles and tabular-number
features used for final painting. Ordinary words wrap; a table whose unbreakable
content exceeds the reading width remains horizontally scrollable.

Table cells use 13/6-pixel padding, semibold headers, vertically centered content,
collapsed one-pixel border spacing and alternating body backgrounds beginning at
the second body row. The previous header-inclusive index striped the first body
row. Fractional compressed-column boundaries also caused a nominal one-pixel
stroke to paint two pixels under conservative raster bounds; table vertical
strokes now snap to physical pixels. Layout and scrolling retain fractional
coordinates.

The actual report's upward page transition then exposed a separate cancellation
error: a later source caret restored through a fractional H2 prefix produced a
local origin of `6.000011444` instead of `6.0`. Conservative fill bounds added a
second horizontal border pixel; the opposite error sign could also move glyphs.
The shared composite viewport translation now normalizes only errors within
1/1024 of a physical pixel from an integer. Backgrounds, borders and glyphs use
the same translation. A subsequent 17-pixel wheel phase exposed the same error
in a root H2 separator, outside child viewport translation: exactly one 936-pixel
border row changed. The same normalization now applies to final rectangular and
rounded fill bounds before pixel coverage is calculated. Meaningful fractional
scroll positions remain intact; the regressions explicitly preserve quarter- and
half-pixel changes.

## Reference and platform fonts

The metrics were checked against GitHub's primary
[Primer table rules](https://github.com/primer/css/blob/main/src/markdown/tables.scss),
[Markdown body rules](https://github.com/primer/css/blob/main/src/markdown/markdown-body.scss)
and [typography variables](https://github.com/primer/css/blob/main/src/support/variables/typography.scss).
The implementation uses the existing native renderer. No stylesheet, font,
third-party artwork, dependency or licensed asset was copied or added.

The renderer's installed font database resolves `system-ui` to **Segoe UI** on the
verified Windows machine. Ubuntu 24.04's `fc-match system-ui` reports **DejaVu Sans
Book**; Linux follows the configured Fontconfig system alias rather than a bundled
or hard-coded Windows font. That Linux lookup is not a Linux raster qualification.

## Evidence

| Check | Result | Evidence beneath `_build/qa/` |
| --- | --- | --- |
| Font reset, numeric-width and striping regressions before fixes | Four expected failures | `markdown-continuity/style-red.log` |
| Core font preference cases | 3 passed | `markdown-github/style-green.log` |
| Immediate engine preference acceptance cases | 3 passed | `markdown-github/style-green.log` |
| Complete main UI library suite, including early preset replay | 667 passed | `tree-filters/ui/lib-green.log` |
| Compressed-column pixel regression before stroke correction | Expected `[1, 2, 1, 1]` instead of four one-pixel strokes | `markdown-github/pixel-red.log` |
| Complete renderer suite after vertical-stroke correction | 37 passed | `markdown-github/pixel-green.log` |
| Caret restoration through fractional document origins before correction | Expected changed raster | `markdown-github/continuation-red.log` |
| Complete renderer suite after continuation correction | 39 passed | `markdown-github/continuation-green.log` |
| Root H2 separator before final fill-bound correction | Expected changed raster at -0.0001 px | `markdown-continuity/heading-border-red.log` |
| Complete renderer suite after final fill-bound correction | 40 passed | `markdown-continuity/heading-border-green.log` |
| Renderer strict Clippy after final correction, all targets | Passed | `markdown-continuity/render-clippy-final.log` |
| Actual report table rendering/font resolution | Segoe UI; normal content 980 × 380, narrow content 967 × 380 | `markdown-github/render.log` |
| Final report image border scan | All eleven vertical boundaries are one pixel wide | `markdown-github/border-pixels.txt` |

The renderer suite includes exact raster equality for the same table anchored in
the viewport after changing preceding paragraph/H1/H2 context, numeric token
integrity, vertical centering, selection/source preservation and existing
Unicode, wrapping, table pagination and raster tests. The continuation regression
restores a later source caret across 48 different leading H2 contexts. Another
raster check covers both error signs and verifies that quarter/half-pixel scroll
positions remain distinct from integral scrolling. The root heading separator
regression exercises both error signs and the same meaningful fractions without
depending on a nested table viewport.

The local user report was read without modification:
`D:\temp\ovclash\results\2026-09-07-mfg\post-merge-backend-timings.md`.
`table_probe` renders its first table with the production renderer. Final inspected
images are `markdown-github/rendered/table-light.png`, `table-narrow.png` and
`table-dark.png`. The narrow image exposes the complete overflowing table for
visual inspection; host viewport clipping/scrolling is a separate boundary.

Windows commands use the existing repository-local toolchain, offline locked SDK
configuration and coordinated shared `_build` output. Representative commands:

```powershell
. ./_build/qa/navigation-ordering/windows-env.ps1
cargo test -p midnight-viewer-render @uiArgs
cargo test -p midnight-core --lib viewer_preferences @uiArgs
cargo test -p midnight-viewer-engine --lib preferences::tests @uiArgs
cargo clippy -p midnight-viewer-render --all-targets @uiArgs -- -D warnings
cargo run -p midnight-viewer-render --example table_probe @uiArgs -- INPUT.md _build/qa/markdown-github/rendered
```

The host early-load replay regression passed in the coordinated main UI library
suite. Scoped `rustfmt --check` also passed. No whole-workspace or full-file
scrolling success is inferred from these focused results.
