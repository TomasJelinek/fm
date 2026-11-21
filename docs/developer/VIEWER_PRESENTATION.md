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

An open Find bar keeps its place in a short window. When the window cannot hold the
Find bar, this row (or the HTML viewer's Online, JavaScript and zoom row), the status
and footer rows and one control height of text, the row gives way and the Find bar
shows **Close**, which closes Find and brings the row back; in plain text **Aa** does
the same. Find or Ctrl+F opens Find again in its place, and the HTML zoom keys work
throughout. The decision uses the window size that the viewer applies with its
compact layout, so the footer, the Find bar and the status line stay inside the
window at the 320×200 minimum with touch density and in short wider windows with
32-pixel interface fonts. The Lines box and the other source controls fit 480×320 and
320×200 windows; acceptance probes that skip transparent layers do not see its native
check box, an opacity-0 layer over the drawn box, at any window size.

Markdown always wraps to its reading width. GitHub is the default with 1.5 font
size line spacing, 32-pixel side padding (16 pixels below 768 logical pixels of
window width) and a maximum 980-pixel reading width.
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

Selected text keeps its syntax or Markdown hue: a glyph color that would show at
less than 4.5:1 contrast (WCAG) against the selection color, as painted over the
viewport background, is lightened or darkened only as far as needed, decided per
glyph cluster (black or white when no lightness of that hue reaches 4.5:1).
Unselected text and underline/strikethrough lines keep their colors; the text
comparator's find highlight follows the same rule, and the HTML and image viewers
are unaffected.

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

## First display and raster geometry

A viewer opened from the application captures its parent's physical bounds and
exact display scale. `WindowPlacement` applies that scale before sizing the child,
so the first logical layout and backend font metrics do not depend on when the
native child is created by the event loop. Subsequent native DPI events remain
authoritative. A placement supplied only as a logical size has no captured scale
and does not guess one.

Each displayed raster retains the logical viewport against which its effective
pixel dimensions were mapped. Showing the source or Markdown controls can shrink
the live canvas; it clips the existing frame instead of compressing its glyphs.
Body, gutter, camera offsets and selection use that same frame extent. Metadata
updates retain it for the same surface, and initial viewport-only pixels pass it
to retained presentation when the generation and effective viewport match. A new
layout surface uses the replacement viewport. This mapping preserves the backend's
effective raster scale when its pixel budget reduces resolution below native DPI.
At native resolution one texel maps to one physical pixel. The canvas size sent
to the backend is rounded to whole physical pixels, and a canvas that is not a
whole number of physical pixels (fractional DPI, or 22.5 pixels of chrome at
125%) clips the remaining fraction instead of resampling every glyph. A canvas
origin between physical pixels (the 6-pixel padding is 7.5 pixels at 125%) does
not resample either: the window offsets the retained body, gutter and
viewport-only images by the origin's sub-pixel phase, so their texels start on
whole physical pixels. FemtoVG in Slint 1.18 also aligns an image's draw origin
itself; the software renderer only rounds the image rectangle, and floating-point
noise at a half-pixel origin could grow a raster by one row and repeat a row of
texels. Software-rendered tests check one-pixel checkerboards at 110%, 125% and
175% with several guard sizes. The `texel_alignment` example checks a native
window with either renderer, including a control with the former stretched
mapping, for example after `$env:SLINT_SCALE_FACTOR = '1.25'`:
`cargo run -p midnight-viewer-ui --example texel_alignment -- --femtovg`.

The UI crate's bounded pixel-preparation worker converts immutable backend RGBA
of text and legacy surfaces into safe shared Slint pixel buffers off the event
loop. Slint images and window state remain on the UI thread; the engine and
independent plugins do not acquire a Slint dependency. The worker reuses converted
buffers for metadata-only updates. Image and PDF tiles are the exception: the UI
texture cache copies each new 256×256 tile (256 KiB) into a Slint buffer on the
event loop when it first arrives, at most 128 tiles and 32 MiB per frame, and
reuses it by tile identity afterwards. Panning therefore copies only newly exposed
tiles. Delivery bounds, retirement and stale-frame rules are described in
[Viewer scrolling](VIEWER_SCROLLING.md#retained-text-pixels-and-delivery).

Window controls whose widgets assign their own state (mode chooser, Online and
JavaScript, wrap, case, line numbers, font and Markdown choosers, size) are bound
two-way to window properties; a one-way binding would be dropped by the first
click, selection or model reset. Rust re-sets these properties from the state in
force with every presented frame. The mode chooser and the Online/JavaScript boxes
also follow document changes and backend state replies immediately, and restore
the state in force when a request cannot be delivered, so they never show a mode
or web permission that is not applied.

All three text plugins prepare bounded windows rather than loading complete files.
The basic plugin gathers at most 8 KiB per viewport; source and Markdown gather at
most 64 KiB of source context, with bounded output and checkpoint/cache state.
See [large-file evidence and scope](../verification/2026-11-10-large-text-viewers.md)
for byte/heap measurements, native versus archive preparation, and timing limits.

### Refill reads and the retained frontier

The basic and Source plugins answer a Text or Source viewport with at most about 8 KiB.
One anchor per character and the wire item budget limit a reply to about 13,000
characters, so larger plugin pages are not possible within the current API. The engine
joins byte-capped replies up to the request's line budget, in at most 16 reads and one
text layout: 120 KiB laid out, a tab counted at its widest (four bytes), and 4,096 styled
spans (`worker_chain.rs`; see below). A retained refill asks for the rows around the
camera, and the shown page already holds most of them: the page moves a little each time
the camera nears its edge. Such a request reads only the rows past the shown page's edge
(before its start, backwards) and joins its reply to the rows the page holds. The joined
page is the one a whole read gives: both end at the line budget or, with longer rows,
where one layout is full (to within a character, as each read stops before a character
that does not fit). A reply that does not continue those rows makes the engine read the
request whole. Scrollbar positions keep whole reads.

Method: the engine's deterministic co-simulation (`worker_frontier_tests.rs`, ignored
measurement `measure_page_reads_by_row_width`) runs the real worker, a simulated basic or
Source plugin that pages as the plugins do and answers one 8-ms frame later, and a model
of the window's retained camera, in a 1312×720 window at 100 %. After a seek to the middle
it runs four PageDown, four bursts of five wheel notches, twenty Down presses and four
PageUp. Plugin reads and their bytes are deterministic evidence; frames until the content
rests stand for latency, one frame per read round trip. The test prints both columns: it
reads every request whole for "before". Text and Source gave the same counts up to
830-byte rows (Windows, 21 November 2026):

| Rows | Four PageDown, before → after | Four bursts of five notches | Four PageUp |
| --- | --- | --- | --- |
| 60 bytes | 8 reads, 36 KiB, 32 frames → 8 reads, 10 KiB, 32 frames | 19 reads, 86 KiB → 19, 4 KiB | 8, 36 KiB → 8, 9 KiB |
| 120 bytes | 16, 73 KiB, 40 frames → 8, 20 KiB, 32 frames | 36, 164 KiB → 19, 8 KiB | 14, 63 KiB → 8, 19 KiB |
| 250 bytes | 24, 152 KiB, 48 frames → 8, 42 KiB, 32 frames | 48, 304 KiB → 19, 17 KiB | 21, 131 KiB → 8, 39 KiB |
| 500 bytes | 40, 304 KiB, 64 frames → 16, 85 KiB, 40 frames | 60, 457 KiB → 19, 35 KiB | 35, 263 KiB → 14, 82 KiB |
| 830 bytes | 64, 505 KiB, 88 frames → 24, 142 KiB, 48 frames | 64, 505 KiB → 19, 59 KiB | 64, 499 KiB → 21, 136 KiB |

Bursts take 112 frames before and after; twenty Down presses read 5 → 2 times over
500-byte rows and 16 → 2 over 830-byte rows. Opening reads are unchanged (1 read for
60-byte rows, 8 for 830-byte rows), no short file reads more, and every gesture still
moves its whole distance. The reruns after the long-row change, the renderer's bounded
shaping runs and the pages of 120 KiB below gave every count of this table again, in Text
and Source.

**Rows longer than a page holds.** A joined page is one text layout, and the renderer's
layout (parley 0.11) keeps a glyph cluster's offset inside its shaping run in 16 bits
(`ClusterData::text_offset`); uniformly styled text was one shaping run until the renderer
bounded its runs (below), so no byte past 64 KiB of a page had the right caret, hit test or
line range. The chain read on until a
page reached 64 KiB, and Text pages of the basic viewer's 8,176-byte ranges grew to 73,584
bytes: the caret of offset 72,000 of a 73,200-byte page lay on the page's end, a row too
low. A backward refill restores the camera on the last row of its page, so each one
shifted the camera's world by a row, and every PageUp over 1200-byte rows stopped three
rows (49 px) short. Source pages, of 8,192-byte ranges, stopped at exactly 64 KiB and kept
their travel. The earlier attempt with pages of 128 KiB, rerun in the co-simulation,
failed the same way. Backward restores of rows past 64 KiB landed 160 px above their new
page, which the camera therefore never reached, and each retry read the page before it,
back to the file start: PageUp over 1200-byte rows jumped 24,359 px. Refills placed on
rows past 64 KiB anchored at their page's start instead (24 times over 2400-byte rows),
and new surfaces arrived while the window still moved, which drops its remaining gesture:
PageUp over 2400-byte rows stopped after 148 to 247 of 720 px. Reading on until a page
fills the window made pages longer than 64 KiB too (44 rows of 2400 bytes are 105 KB). A
joined page now stays within one layout (`fit`): a chained read asks only for the room
left, and a reply is cut at an anchor to fit. With pages of 64 KiB, 54 rows of 1200 bytes
filled a page, 27 of 2400 and 8 of 8000, while a 720-pixel window shows 44 rows, 76 with
its guards; pages now hold 120 KiB (below).

A page whose rows did not fill the window left the retained camera, and the relative
scrolling that took over clamped each step to the next page's top: PageDown over
2400-byte rows moved 27 rows (443 px) in Source and 3 in Text, an arrow key one row. A
page that barely filled it let its camera move a few pixels, too few for the guard a
refill keeps above the camera: no refill advanced, or the camera stopped between two
pages that each covered one side of it. A page whose rows are too long for it to hold the
window and a guard on each side (its bytes or styled spans over those pixels would exceed
a page's 120 KiB or 4,096) now keeps the retained camera and lets it move at least a guard
(256 logical pixels), as far as its last row's top and past its first row, while the
window shows blank space below the page's rows; a refill keeps the rest of the page
below the camera (`viewport::camera_span`). A page that ends the file keeps its end at the window's bottom
unless it is shorter than the window. A refill that would read no new row is not sent,
and a camera outside its page asks for a page holding it without a guard when a guarded
one cannot advance. Pages that fill the window and a guard below it, and pages of shorter
rows, those up to 830 bytes among them, keep their range: a small fragment of short rows
still takes the viewport path when its camera cannot pass a row.

Verified in the co-simulation (`worker_long_row_tests.rs`, Windows, 21 November 2026):
rows of 1200, 2400 and 8000 bytes, and a file mixing them with rows of 60 to 830 bytes,
in Text and Source, wrapped and unwrapped. After three scrollbar positions, every one of
three PageDown, five notches (one per frame, twice, and in one burst), twelve Down presses
(one per frame, and in one burst), three PageUp, five notches up and a burst of twelve Up
presses moves its whole distance, to within a pixel of the document's rows as they lay
out one by one. With the change undone, every one of these four tests fails: PageUp
over 1200-byte rows moved −670.8 of −720 px in Text, PageDown over 2400-byte rows 49.2
(Text) or 442.6 px (Source), over 8000-byte rows 0 to 147.5 px, and in the mixed file
gestures dropped their travel with a failed handoff or never came to rest. The
measurement gives, before (the code of 21 November) → after:

| Rows | Four PageDown | Four bursts of five notches | Twenty Down | Four PageUp |
| --- | --- | --- | --- | --- |
| 1200 bytes, Text | 108 reads, 862 KiB, 136 frames → 34, 201 KiB, 62 frames | 108, 862 KiB → 32, 86 KiB | 27, 215 KiB → 8, 39 KiB | 108, 862 KiB, all four short → 35, 213 KiB |
| 1200 bytes, Source | 144, 1152 KiB, 178 frames → 34, 201 KiB, 62 frames | 136, 1088 KiB → 32, 86 KiB | 32, 256 KiB → 8, 39 KiB | 136, 1088 KiB → 35, 213 KiB |
| 2400 bytes | 36 (Text) or 32 reads, all short → 55, 385 KiB, 82 frames | 180 or 160, short → 36, 175 KiB | 180 or 160, short → 12, 72 KiB | 36 or 32, short → 58, 409 KiB |
| 8000 bytes | not measured → 169, 1321 KiB, 210 frames | → 74, 578 KiB, 123 frames | → 35, 273 KiB | → 202, 1395 KiB |

Text and Source read the same after; no gesture stops short. A page then held at most
64 KiB of laid-out text and 4,096 spans, with one 16-byte anchor per character (at most
1 MiB); a chain holds one such page besides the shown one, and pages grew to 73,584 bytes
before. Joins, cuts and layouts stay on the viewer worker, and the window's camera is
unchanged.

A scrollbar position lands within 0.2 % of the released thumb over rows of 1200 and 2400
bytes and within 0.4 % over 8000-byte rows, where Source places its camera on a whole row
(`source_minimum_origin`). In the mixed file it lands up to 1.6 % away, as it did before
this change: its first read estimates the target from the shown page's density, and one
correction read is allowed (`refine_position`). A piece of a row longer than one layout,
without a row break, keeps the viewport path: such a row shows as pieces, one per line,
which these tests do not cover.

**Layouts longer than one shaping run.** The renderer bounds the shaping runs of every
layout itself (`midnight-viewer-render`, `runs.rs`), so every layout it accepts, up to
256 KiB with tabs expanded (`MAX_TEXT_BYTES`), has the right caret, hit test, selection
and line range: Markdown and document blocks, whose text adds separators, tab-expanded
text and every other caller included. Parley starts a run where the font features change,
and a shaper drops a feature whose value is zero before it looks the feature up
(harfrust's `ot_map`, as in HarfBuzz), so every other piece of a layout longer than 64 KiB
gets the disabled feature `"MRUN" off` besides its own, and its glyphs and advances stay
those of one run. A piece ends at the last line start within 64 KiB, where shaping never
joins glyphs. Only a line longer than 64 KiB is cut inside, after its last whitespace that
fits, else at its last grapheme-cluster boundary, where a kerning pair or ligature across
the cut can be lost. One grapheme cluster longer than 64 KiB is cut between two of its
characters: its carets stay right, and parley, which counts a cluster's glyphs in 8 bits
(`ClusterData::glyph_len`), draws such a cluster wrong anyway. Layouts up to 64 KiB are
unchanged. Verified on Windows, 21 November 2026:
`tests/long_layouts.rs` compares the caret, a hit test one pixel into the character and a
three-byte selection at bytes past 65,536 with the same row laid out alone, over 100,000
bytes of 100-byte rows, unwrapped and wrapped at 300 px, and over 4,000 tabbed rows whose
52,000 bytes lay out as 88,000. All three failed before: the caret of byte 65,500 lay at
y 20,000 (the text's end) instead of 13,100, and with tabs that of byte 38,727 at 80,000
instead of 59,580. `runs_tests.rs` lays out six texts (Latin kerning and ligatures, Arabic
and Hebrew with digits, combining marks and emoji sequences, CJK with Hangul and
Devanagari, tabs with CRLF, Unicode line separators) in three typographies, one with paint
and style spans, cut at every line start, and finds every glyph, line metric, caret, hit
test, selection rectangle and pixel equal to those of one run; turning kerning and
ligatures off in the cut pieces instead fails it. Monospace Latin cut after its spaces is
equal too. Cuts at line starts cost no measurable time: 4,095 lines of 73,710 bytes laid
out in 13.1 ms without them and 13.2 ms with them. One 256 KiB line, whose cuts scan its
grapheme clusters, took 30.4 ms without cuts and 32.2 ms with them (release build, medians
of nine in the quietest of three interleaved rounds while other builds ran; the other
rounds varied by up to a fifth either way).

**Pages of 120 KiB.** With bounded runs, a joined page lays out up to 120 KiB
(`viewport::PAGE_BYTES`): fifteen plugin reads of 8 KiB, so the last of a chain's sixteen
takes the room left. 51 rows of 2400 bytes fill a page instead of 27, more than the 44 a
720-pixel window shows. At rest after a seek to the middle and the measurement's gestures,
0 to 12 px of the window stayed blank below the page's rows instead of 245 to 489 px, in
Text and Source (`rows_of_2400_bytes_fill_the_window_at_rest`, which fails with pages of
64 KiB). The camera may still pass a page's rows by up to a guard before the next refill
(`viewport::camera_span`). Pages of 1200-byte rows end at their line budget (76 rows).
Rows of 8000 bytes still leave most of the window blank: a page holds 15 of them (8
before), about 250 of the window's 720 px. A page holding the window and a guard below it
would take 144 KB of 2400-byte rows, more than sixteen reads hold. The measurement, pages
of 64 KiB → 120 KiB (Text; Source counts the same except opening, 8 → 12 reads over
1200-byte rows and 8 → 15 over longer ones):

| Rows | Open or seek | Four PageDown | Four bursts of five notches | Twenty Down | Four PageUp |
| --- | --- | --- | --- | --- | --- |
| 1200 bytes | 9 reads, 11 frames → 12, 14 | 34, 201 KiB, 62 frames → 28, 205 KiB, 52 frames | 32, 86 KiB → 19, 86 KiB | 8, 39 KiB, 26 frames → 3, 22 KiB, 24 frames | 35, 213 KiB, 63 frames → 27, 194 KiB, 50 frames |
| 2400 bytes | 9, 11 → 16, 18 | 55, 385 KiB, 82 frames → 57, 399 KiB, 85 frames | 36, 175 KiB → 36, 171 KiB | 12, 72 KiB, 25 frames → 12, 77 KiB, 26 frames | 58, 409 KiB, 86 frames → 59, 411 KiB, 87 frames |
| 8000 bytes | 9, 11 → 16, 18 | 169, 1321 KiB, 210 frames → 162, 1263 KiB, 190 frames | 74, 578 KiB, 123 frames → 78, 609 KiB, 139 frames | 35, 273 KiB, 46 frames → 34, 265 KiB, 43 frames | 202, 1395 KiB, 247 frames → 187, 1390 KiB, 217 frames |

Rows of up to 830 bytes read exactly as before, and the long-row tests above pass with
these pages, as do the chain tests (`a_joined_page_fills_one_layout_and_places_its_last_row`
now joins 2400-byte rows, whose full page passes 64 KiB). Pages of 128 KiB measured alike
but end at the sixteenth read, not at the layout's room. A page's layout grows with it: 64 KiB of
2400-byte rows take about 8 ms in a release build, 128 KiB about 16 ms, on the viewer
worker. A page's anchors take up to 1.9 MiB (16 bytes per character) instead of 1 MiB, and
a chain holds one page besides the shown one. Source pages of densely highlighted long rows
can still end at the 4,096-span budget before their bytes do.

A frame of the shown surface whose pixels do not reach the camera the window shows is
refused. This happens when the engine pages or restyles for the camera a request asked
for, which runs the request's lookahead (up to 128 px) ahead of the shown camera: a page
restyled for highlighting starts at that camera's row. The window then waited for pixels
nothing would paint, so the rest of a wheel burst was never shown. This is the acceptance
report of five notches moving 256 of 300 px in a long Rust file: the co-simulation
reproduces exactly that stop with highlighting prepared while a slow window presents
every sixth worker turn and its mailbox keeps only the newest frame
(`a_burst_over_a_page_restyled_for_its_lookahead_keeps_its_whole_travel`). The camera now
asks again for the shown camera itself, without lookahead (`raster_camera::Camera::receive`),
and every gesture completes; the window's own regression test is
`raster_frontier_tests::a_page_restyled_around_the_lookahead_does_not_stop_a_burst`. An
exploration of that change ran the highlighted Rust file in 13 configurations (windows
from 1000×560 to 2624×1440 at 100–200 %, plugin delays of 0–12 frames, bursts and single
inputs, 40 scrollbar positions each) through wheel notches, Up and Down, PageDown and
PageUp each followed by grouped arrows, and twenty Down presses, each to within 1 px. With
the camera asking again, no gesture lost travel. Before, only the windows presenting
every sixth or twelfth worker turn stopped, 44 px short on Windows and Linux alike; a
window presenting every fourth did not. That exploration only printed its table, and the
model's variant of a window that waited after a refused frame only showed the old stop:
both were removed with the long-row change above, and the co-simulation's window now
always asks again, as the real camera does. The window's tests over
continuing and corrected refills without restyling
(`raster_frontier_tests::keys_and_wheel_keep_their_travel_*`) pass with and without the
fix: they did not reproduce the report and stay as guards. The reported PageUp of −792 px
was not reproduced.

The settings codec and persistence contract are described in
[Viewer preferences](VIEWER_PREFERENCES.md). UI callbacks perform only small input
and presentation updates. Source reads/indexing remain in the independent plugin;
text shaping, rasterization and annotation preparation remain on backend workers.

## Video canvas and playback

Video uses an author-pixel canvas, closer to image presentation than text reflow.
The active media surface fits the current frame to the live canvas with aspect
ratio preserved and unused space around the centered frame. App palette changes
style the chrome and canvas background; they neither recolor video pixels nor
make their delivery wait for a text-palette acknowledgement.

Playback has dedicated, session-bound Play/Pause controls, a seek timeline,
position/duration text and pending/error feedback. The active media layout hides
text Find, typography and scrollbars. Timeline seeks use media controls rather
than text scroll positions; seek and replay frames retain their media-session and
applied-control fences.

Resizing the canvas, including a playback status row appearing or collapsing,
preserves the existing `MediaSessionId` and its generation. An admitted video
resize cancels the text scroll-position gate; it must not wait for a newer text
generation before accepting advancing video frames. Layout and media-control
checks still apply. Text and Source resize keep their distinct newer-generation
requirement. Image zoom/camera and retained image tiles keep their existing
presentation paths; this video fitting contract does not change them.

Current scoped native verification covers silent synthetic-AVI Play/Pause,
paused resize, seek, EOF and replay. It does not qualify audio, full codec
coverage, Linux or performance. See the [media verification checkpoint](../verification/2026-11-19-media-follow-up.md)
for source identities, receipts and remaining acceptance.

## Find (in development)

The new visible **Find** button opens the existing search field through the same
action as **Ctrl+F**. **F3** finds the next match and **Shift+F3** the previous
match; these are the default shortcuts and can be remapped. Existing search text
is retained when Find is opened again. The button is offered for text, binary,
Source, Markdown and HTML views; image and PDF document views hide it.

The visible Find integration selector passed in its earlier component. The later
footer selector and two keyboard tests passed within the 31-test continuation.
Options focus and Viewer46 diagnostics remain open as described below; this is
not final native/app input qualification. Office HTML search covers the current
generated bounded page, not the complete workbook or document.

## Source highlighting and dense pages

The independent Source plugin has 25 lexical profiles, including actual Lua, YAML,
CSS and Makefile styles; see its [catalogue](../../plugins/midnight-viewer-source/README.md).
`Makefile` and `GNUmakefile` reach it through whole-name association entries
(`makefile`, `gnumakefile`); see [viewer file types](../VIEWER_ASSOCIATIONS.md).
Lua keeps matched long-bracket strings/comments and short-string escape state.
YAML distinguishes mapping keys, quoted/plain/block scalars and flow punctuation;
C++ retains custom raw-string delimiters. These are lexical profiles, not semantic
analysis, language execution or complete-document validation.

When styles and anchors exceed the wire budget, the page is cropped at exact
character/source boundaries while retaining styles: the beginning for forward
requests and the end for backward requests, with continuation flags for the rest.
The plugin does not clear all colours to fit a larger page. Distributed/recent
complete-line checkpoints stay within 256 slots; foreground replay remains
bounded. Cold distant positions may still show plain text. Immutable API/SDK0.2 preparation is now locally published, and the host/engine
preparation gate has 98 selected passing checks. Source0.2 bounded recovery has
the post-review64 receipt below, with no unresolved owner failure in that scope;
this is not an arbitrary-token guarantee or full native cold-target qualification.

The post-review Source snapshot passed **64 unique checks / 0 failures** across
seven groups, strict `--lib --tests` Clippy/format0. Prior 99 is pre-fix evidence,
not a rerun or an additive total. Two real P2 RED→GREEN cases reduce the target
boundary predicate from 67,117,056 comparisons to 16 in one preparation callback
(excluding independent decoder/source lookups), and six offscreen pending cases
from 16 reads/262,144 bytes/Ready=false to 1 read/Ready=true. These are fixture
observations, not a general one-read or latency guarantee.

The conservative retained-syntax upper bound is 49,848 bytes, including the bounded
8KiB backward ring and overhead; the giant-line case measured566,776 bytes of
incremental Rust heap peak, not RSS. The subsequent five-library host/Viewer/
comparison slice passed **275 unique headless tests / 0 failures / 0 ignored**;
325 executions include 50 repeats. Its final native PDF fixture run passed 2/0
with the real C worker, using Viewer harness `f00316db…`, with no remaining child
workers. The earlier `b43d0280…` pre-lint run is a separate checkpoint. This is
functional captured-file fixture evidence, not a production native-filesystem
capture, benchmark or full-app result. The source closures were released after
that gate. Configured backend routes subsequently passed 77/0/2 with scoped
Clippy/format0. The canonical P3 UI packet is promoted: **104 source files /159 source-derived
selections**, not 159 executed passes. The continuation passed **31/0** (MainUI28,
Viewer footer1 and keyboard2); Root independently rehashed/read all seven exported
raw logs. With the initial 7/1, the selected 39 is **38 passed /1 Options-init-focus
failure**. The former footer failure and both keyboard tests pass on this snapshot.
The older MainUI/CompareUI55/0 retains its own source and overlaps these scopes.
P3 MainUI units initially recorded 20/1; after the fixture precondition correction,
the exact draft selector passed once. The other 20 were not rerun, so there is no
fresh 21-pass claim. Options focus still fails: the trace/screenshot show OK focused
while the expected pattern field is visible. The diagnostic compile qualified with
zero warnings/errors,1890 source guards and 323 feature sets; app/bindings unchanged.
The earlier Viewer46 timeout and subsequent45/1 retain their dated source.
On the canonical `fc4fcdf7…` shared-helper checkpoint, the exact former failure
passed first and the other 45 then passed: **46 actual passes /0 failures**.
Both raw logs retain their separate 1+45 counts; all nine document cases and
19 image-presentation cases passed, including the genuine immediate-open errors.
These overlap earlier scopes and are not additional unique passes or all 159.
Options focus, native/full-app and production migration acceptance remain open.

The latest parent diagnostic `office-9fe36f20a6` remains **0 passed /1 failed**:
completed real Find returned 0, Copy failed and Session stayed live. Its blank902×602
diagnostic frame is not validated fresh content/resize proof (resize_ok=false,
fresh_contrasted_frame=false). Source refusal remains required; no provider/Find/
loading root cause is established. Host47 +Viewer44 =91/0 retain their frozen source;
the later four assertion-style lint fixes passed strict Clippy0/format13files0 and
two overlapping resource tests, not 93 unique tests or a full91 rerun.

Root accepted the generic provider small-body navigation contract for private
implementation: captured main-frame GET/no-post targets of 0..1MiB are completely
admitted through genuine Resolve/ResourceRead/Data before replacement navigation;
larger targets retain streaming through the same resolved capability. Small-body
failure cannot fall back to streaming/old bytes. Preserve one body credit through
actual response release, exact one-shot request binding, a30-second admission
deadline and current authority checks. This changes no public API/SDK or dependency
pins. The strong retained-document boundary applies to eligible small targets;
large streaming retains its late-Read-denial limitation. Office V3's exact16 sources are now canonical (freeze `10d0a5e6…`), with
132 nonowned guards unchanged and public API/SDK, Cargo, executable and runtime
preserved. The unchanged pure navigation core `c6596c4c…` passed **34/0**, compile0;
the existing CEF version-metadata manifest warning remains, so this is not a
zero-warning gate. These tests exclude CEF/native execution and do not qualify
new V3 route/adapter integration. Five native cases await a legitimate complete
new C product; no new executable or deployment is claimed.

The selected MainUI/CompareUI55/0 retains its separate source; Engine was guarded
only. Canonical old 0.1/new0.2 packaging9/0 and fresh-cache sparse-consumer resolution
remain dependency-resolution evidence, not app/native qualification.


The normal Source producer reused16 fresh Cargo artifacts and the same `9915dfda…`
DLL with source/SDK/profile guards; no code recompilation/deployment or native
Source execution follows. P3v2 is canonical104/159; the source-derived159 is not a pass count.

Root accepted the Viewer/Compare live-appearance contract for private
implementation; it is not implemented or passing. Application UI font is separate
from document font. Frontend apply_appearance consumes actual applied/preview Theme,
density and UI font scalars for existing/new windows; backend restore_palette accepts
foreground/background/selection/accent only. Accepted revision is not painted/native
acknowledgement. Bounded latest requests, A→B→A, rollback and frame-generation fences
must preserve source/layout/position without recapture, relaunch or source I/O.
Source/Markdown semantic roles must recolor active/cached content, gutters and raster;
Office/author HTML CSS and original image/PDF pixels remain content-owned. Reuse
supported UI controls and preserve input/accessibility; no SDK expansion is selected.

The accepted opening-palette amendment adds host `Session::palette_revision()`
and `WindowSession::palette_revision()` getters of the current accepted revision.
They remain readable after close/disconnect, perform no admission or mutation and
are not paint acknowledgements. Frontends merge the four actual app colors before
open, then seed opening fences from the real getter before publishing/consuming
the session; `restore_palette` is for later changes. Preserve document fonts and
typed terminal ExternalStarted/Unknown outputs. Do not hardcode a universal initial
revision: current Viewer and Compare conventions differ. This is accepted private
implementation in progress, without Core DTO/public plugin SDK changes or passing
integration evidence.

Private Slint V2 generation passed all three build-script steps with zero
errors/warnings and 194 input guards unchanged. Main's ordinary and tooltip passes
finished in 57.438s and generated63,596,709 bytes of Rust; Viewer/Compare each took
about 1.3–1.4s. These are code-generation observations, not a performance benchmark.
Generated Rust is uncompiled; there is no input, paint, palette-acknowledgement or
full application result.
