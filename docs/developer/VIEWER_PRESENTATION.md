# Source and Markdown presentation

The viewer window exposes source line numbers (on by default), mode-specific font
family and logical-pixel size controls, and Markdown presentation presets. Source
and Markdown keep the controls visible above the content. Plain text exposes its
font controls through **Aa**. Image/PDF and binary views do not offer text fonts.
The typography controls occupy one row, including Markdown's presentation preset.
The font chooser lists installed family names alongside the `sans-serif`, `serif`
and `monospace` fallback choices. The viewer backend discovers and caches the
catalog through the renderer's font database; UI callbacks only present it.
The chooser, size and Markdown preset share one row down to 320×200 windows.
Font sizes are 8–48 logical pixels. Keyboard and pointer selections reach the
existing ViewerPreferences listener immediately, including a selection followed
by Escape before another frame arrives. Each mode restores its own family and
size. An unavailable saved family remains selected until explicitly replaced;
the renderer uses its usual glyph fallback. Programmatic restoration does not
save a new edit.

Markdown always wraps to its reading width. GitHub is the default with 1.5 font
size line spacing, 16-pixel side padding and a maximum 980-pixel reading width.
Compact uses 1.25 spacing and 8-pixel padding. Comfortable uses 1.8 spacing,
28-pixel padding and a 760-pixel maximum reading width. Dimensions are logical
pixels and scale with display DPI. Preset changes and font changes reflow the
bounded semantic page on the viewer worker; they do not reread or reparse the
file. Heading and emphasis sizes scale with the selected font size. The source
anchors, decoded text, search and clipboard content remain independent of these
presentation choices.

GitHub-style pipe tables use shared column positions, bold header cells, borders,
and left/center/right alignment. Each cell has its own bounded text layout, so
proportional fonts and wrapped Unicode text do not break the columns. Narrow
windows keep a readable minimum column width and expose horizontal scrolling.
The composite layout retains the ordinary viewer source-anchor, selection,
clipboard and hit-testing contracts. Code spans/blocks use a monospace family
independently of the selected prose font. Read-only tasks use portable `[x]` / `[ ]`
markers instead of font-dependent emoji or ballot glyphs;
strikethrough is painted as a decoration rather than leaving `~~` markers visible.

The independently built Markdown plugin supplies optional semantic table and
strike metadata through the public API. Older hosts can display the same row
text without those decorations. Limits and bounded-context/plain-text fallbacks
are documented in the [plugin README](../../plugins/midnight-viewer-markdown/README.md).
Remote images, HTML execution, mathematics and diagrams are not implemented.

Source gutters are separate prepared annotations, outside the text selection and
clipboard content. Their width remains stable while scrolling because the source
byte length bounds its possible line count. Soft wrapping does not create new
logical line numbers. The current status uses the active selection endpoint;
without a selection it reports the first visible logical row. Columns start at
one and count decoded Unicode scalar values: a tab, combining mark, supplementary
character and malformed-sequence replacement each count as one. CRLF and CR
normalize to LF; LF, U+2028 and U+2029 start logical lines. This matches the source
decoder, including its visible substitutions for other controls.

Exact positions outside the first fragment use the optional public
[source-coordinate request](../../api/SOURCE_POSITION.md). Content and scrollbar
acknowledgements never wait for a position lookup. While a bounded lookup is
pending the status says **Line/column indexing…** and the gutter has no invented
numbers. The source plugin reads at most 256 KiB per coordinate request, including
lookahead and decoder overlap. A distant or over-budget lookup can complete
without coordinates; the host reports **Line/column unavailable** for that page
and can recover exact positions when returning near a checkpoint. An older viewer
plugin can decline the optional feature for the session. Obsolete
requests are cancelled, and replies must match the active request and page range.
No complete-file line table is built or retained by the host.

All three text plugins prepare bounded windows rather than loading complete files.
The basic plugin gathers at most 8 KiB per viewport; source and Markdown gather at
most 64 KiB of source context, with bounded output and checkpoint/cache state.
See [large-file evidence and scope](../verification/2026-11-10-large-text-viewers.md)
for byte/heap measurements, native versus archive preparation, and timing limits.

The settings codec and persistence contract are described in
[Viewer preferences](VIEWER_PREFERENCES.md). UI callbacks perform only small input
and presentation updates. Source reads/indexing remain in the independent plugin;
text shaping, rasterization and annotation preparation remain on backend workers.
