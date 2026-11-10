# GitHub document presentation

The GitHub preset uses semantic blocks on the existing host renderer. It does not
execute HTML, load remote content, or introduce a browser dependency. Compact and
Comfortable keep their existing presentation. Selecting GitHub restores its body
font and size; subsequent explicit font edits remain available. Plain text and
source viewers keep their own font preferences and layout path.

The mapping is independently implemented from published Primer design metrics:
[body and quotes](https://github.com/primer/css/blob/main/src/markdown/markdown-body.scss),
[headings](https://github.com/primer/css/blob/main/src/markdown/headings.scss),
[code](https://github.com/primer/css/blob/main/src/markdown/code.scss),
[tables](https://github.com/primer/css/blob/main/src/markdown/tables.scss), and
[lists](https://github.com/primer/css/blob/main/src/markdown/lists.scss), with
[system typography](https://github.com/primer/css/blob/main/src/support/variables/typography.scss).
No external stylesheet, artwork, font, or executable code is copied or bundled.

- Body: platform `system-ui`, 16 logical pixels, line height 1.5. The existing
  font database resolves this to Segoe UI on Windows and the system-ui Fontconfig
  alias on Linux; no fonts are bundled. Selecting GitHub again resets an earlier
  custom Markdown family/size. A later explicit font edit overrides that reset.
- Reading area: maximum 980 logical pixels, centered in wider windows; 32-pixel
  gutters on windows at least 768 logical pixels wide and 16-pixel gutters below.
- Headings: 2 / 1.5 / 1.25 / 1 / 0.875 / 0.85 times body size,
  semibold, line height 1.25; h1/h2 have a subtle bottom separator.
- Paragraphs: one body-size gap; headings have a larger preceding gap.
- Fenced code: monospace at 85%, line height 1.45, one body-size inset and a
  subtle background. Long code lines remain horizontally scrollable.
- Quotes: muted readable text, a quarter-em border, and one-em left inset.
- Lists: two-em nesting inset, compact adjacent items, normal paragraph gaps
  around the list. The semantic protocol currently exposes bullet items and depth.
- Tables: shared column geometry with 13/6-pixel horizontal/vertical padding,
  collapsed one-pixel borders, semibold headers, tabular digits and vertically
  centered cell content. The second body row starts the alternating backgrounds;
  the header and first body row retain the canvas background. Columns use shaped
  minimum/preferred content widths: words can wrap, while numeric tokens stay
  intact and a table wider than the reading area remains horizontally scrollable.
- Backgrounds and separators use the selected application colors; muted text
  retains at least normal-text contrast when the supplied foreground permits it.

Semantic ranges refer to the existing decoded UTF-8 text. Copying, source anchors,
selection and hit testing use that same text; decoration never inserts artificial
source bytes. Pagination and viewport request ownership remain engine concerns.

## Implementation and verification

1. Add document block metadata and a renderer entry point; demonstrate failing
   geometry/raster tests before replacing the temporary legacy delegation.
2. Build composite block layouts using existing shaping, source maps and table
   layout. Paint block backgrounds before selection and glyphs.
3. Preserve block metadata in the engine page and select the new renderer only
   for GitHub. Retain font scaling and the other presets.
4. Run renderer geometry, Unicode, selection, table and raster regressions;
   inspect light/dark rendered fixtures. Run engine page tests and strict Clippy.

`cargo run -p midnight-viewer-render --example markdown_probe --
_build/qa/markdown-style` produces light/dark bitmap fixtures and reports cold
layout/raster time separately from warmed fractional scrolling. It uses the
production renderer with headings, inline styles, quotes, code, nested lists and
tables; the timing is a local diagnostic rather than a CI performance threshold.

`cargo run -p midnight-viewer-render --example table_probe -- INPUT.md OUTPUT`
renders the first pipe table of a controlled Markdown fixture at normal and
narrow reading widths, with light/dark output. This small diagnostic reads the
fixture without editing it and reports the actual system-ui font resolution.
It is not used to parse viewer documents. Put its output beneath `_build/`.
