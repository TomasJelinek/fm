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
Font sizes are 8–48 logical pixels (`viewer_preferences.rs`). Keyboard and pointer
selections reach the ViewerPreferences listener immediately, including a selection followed
by Escape before another frame arrives. Each mode restores its own family and
size. An unavailable saved family remains selected until explicitly replaced;
the renderer uses its usual glyph fallback. Programmatic restoration does not
save a new edit.

An open Find bar keeps its place in a short window. When the window cannot hold the
Find bar, this row (or the HTML viewer's row of Back, Forward, Reload, Online, JavaScript
and zoom), the status and footer rows and one control height of text, the row gives way
and the Find bar shows **Close**, which closes Find and brings the row back; in plain
text **Aa** does the same. Find or Ctrl+F opens Find again in its place, and the HTML zoom keys work
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
28-pixel padding and a 760-pixel maximum reading width (`text_margin` and `line_height`
in the viewer engine's `worker_text.rs`). Dimensions are logical pixels and scale with
display DPI. Preset changes and font changes reflow the
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

Selected text, decided per glyph cluster, is painted in the theme's selection text
color (`ViewerAppearance::selection_text`, the Theme's `selection-text`), syntax and
link colors included, together with its underline and strikethrough lines; they
return when the selection ends. A color that would show at less than 4.5:1 contrast
(WCAG) against the selection, as painted over the viewport background, is lightened
or darkened only as far as needed, keeping its hue (black or white when no lightness
of that hue reaches 4.5:1). A fully transparent selection text color keeps each
glyph's own color under the same rule. Unselected text keeps its colors; the text
comparator's find highlight follows the same rule, and the HTML and image viewers
are unaffected. While a viewer or comparator window is inactive, its selection uses
the theme's dimmer inactive pair: the view is painted again, without a new layout
(a palette that changes only the selection pair skips the recoloring layout). The colors
and their rules are in the [theme record](../THEME_PROVENANCE.md#text-selection).

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

## Whole-file selection and Copy

Ctrl+A (`viewer.select-page`) in a Text, Source or Markdown view selects the whole file
(`crates/midnight-viewer-engine/src/worker_whole.rs`). Every page the view shows is painted
selected while the selection lasts, through scrolling, resizing, wrapping and font changes, and
the status line ends with **Whole file selected** and the file's size. A click, a drag, a
Find match, another file or mode, or Ctrl+A again replaces it. The hex view's Ctrl+A
selects the rows the window shows; the HTML viewer passes Ctrl+A to Chromium, which
selects all in the page; image and PDF views have no text selection. Painting is
unchanged: the renderer gets the rows a raster shows, as for a selection of those rows,
and nothing is computed per frame without a selection.

Copy (Ctrl+C) of the whole file reads it on the viewer's backend worker, never on the
UI thread. The worker asks the shown plugin session for forward Viewport pages of the
protocol's largest size (256 KiB, 4096 lines) from the start of the file, one at a time,
and joins each page once its request completed (`whole_copy.rs`):

- **Text and Source pages** join end to end. The text is the decoded text the view
  shows: the file's encoding decoded (UTF-8, UTF-16 with its byte-order mark), CR, LF
  and CRLF as LF, controls and malformed bytes in the decoders' visible forms. Wrapping
  is layout alone and adds no line break; a long line arrives as partial fragments of
  one line and stays one line. A page that starts before the copied end continues at
  its exact anchor for that end; one that starts after it ends the copy.
- **Markdown pages** join block by block as a page shows them: a blank line between
  blocks and the list, quote and rule marks. A block the previous page already holds is
  skipped (pages keep whole table rows around their anchor), and the rest of a block a
  page cut at its end continues that block without a break.

The clipboard gets the text once, whole, after the last page, as `ViewerOutput::CopyText`,
with LF line endings on every platform, as the selection Copy gives them; the window
moves it on to the clipboard service without copying it again. A copy that does not end
copies nothing and leaves the clipboard as it was. It ends with:

- **Esc** while the copy runs (`ViewerInput::CancelCopy`; the window knows from
  `ViewerOutput::Copying`, and the next Esc closes the window), another selection,
  another file or mode, or closing the window;
- **the limit:** at most 64 MiB of UTF-8 text (`whole_copy::LIMIT`). Windows keeps
  clipboard text as UTF-16 in one movable global allocation (at most twice the UTF-8
  size), and X11 serves a long text in incremental transfers, so both hold it, but every
  paste copies all of it again into the receiving program. Text and Source decoders
  give at least one text byte for four source bytes, so a file over 256 MiB is refused
  at once; otherwise the read stops when the text passes the limit. The notice suggests
  copying the file itself (F5);
- **a changed file:** the session reads the revision captured when the file opened. A
  read after the file changed on disk fails, and so does a page of another revision;
  the copy then says that nothing was copied.

A copy taking longer than 300 ms shows its progress ("Copying the whole file… 42%") and
how much text it holds. An empty file copies empty text.

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
window with either renderer, including a control drawn with a stretched mapping,
for example after `$env:SLINT_SCALE_FACTOR = '1.25'`:
`cargo run -p midnight-viewer-ui --example texel_alignment -- --femtovg`.

The UI crate's bounded pixel-preparation worker converts immutable backend RGBA
of text and legacy surfaces into safe shared Slint pixel buffers off the event
loop. Slint images and window state remain on the UI thread; the engine and
independent plugins do not acquire a Slint dependency. The worker reuses converted
buffers for metadata-only updates. Image and PDF tiles are the exception: the UI
texture cache copies each new 256×256 tile (256 KiB) into a Slint buffer on the
event loop when it first arrives and reuses it by tile identity afterwards. Like the
backend tile cache, it keeps at most 128 tiles and 32 MiB for an image
(`VIEWER_TILE_CACHE_ENTRIES`, `VIEWER_TILE_CACHE_BYTES`) and a budget that depends on the
view for a document ([Viewer scrolling](VIEWER_SCROLLING.md#documents-pdf)). Panning therefore copies only newly exposed
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

A page holds the [prepared text window](SCROLLING.md#prepared-text-window) (the window,
one screen against the direction of travel and two or three along it) and two more
screens along the travel, so a refill comes about every two screens of travel. An
unwrapped Text or Source page is laid out in [row groups](SCROLLING.md#row-groups): a
request still reads at most 16 ranges and 120 KiB, but the page keeps the shown page's
rows and grows past 120 KiB, up to 1 MiB, and a refill shapes only the rows it adds.

The basic and Source plugins answer a Text or Source viewport with at most about 8 KiB.
One anchor per character and the wire item budget limit a reply to about 13,000
characters, so larger plugin pages are not possible within the current API. The engine
joins byte-capped replies up to the request's line budget, in at most 16 reads and one
text layout: 120 KiB laid out (`viewport::PAGE_BYTES`), a tab counted at its widest (four
bytes), and 4,096 styled spans (`worker_chain.rs`). A retained refill asks for the rows
around the camera, and the shown page already holds most of them: the page moves a little
each time the camera nears its edge. Such a request reads only the rows past the shown
page's edge (before its start, backwards) and joins its reply to the rows the page holds.
The joined page is the one a whole read gives: both end at the line budget or, with
longer rows, where one layout is full (to within a character, as each read stops before a
character that does not fit). A reply that does not continue those rows makes the engine
read the request whole. Scrollbar positions keep whole reads. The deterministic
co-simulation in `worker_frontier_tests.rs` (with the ignored measurement
`measure_page_reads_by_row_width`) runs the real worker against a simulated plugin and a
model of the window's retained camera.

**Rows longer than a page holds.** A joined page stays within one layout
(`worker_chain::fit`): a chained read asks only for the room left, and a reply is cut at an
anchor to fit. A page whose rows are too long for it to hold the window and a guard on
each side (its bytes or styled spans over those pixels would exceed a page's 120 KiB or
4,096) keeps the retained camera and lets it move at least a guard (256 logical pixels),
as far as its last row's top and past its first row, while the window shows blank space
below the page's rows; a refill keeps the rest of the page below the camera
(`viewport::camera_span`). A page that ends the file keeps its end at the window's bottom
unless it is shorter than the window. A refill that would read no new row is not sent, and
a camera outside its page asks for a page holding it without a guard when a guarded one
cannot advance. Pages that fill the window and a guard below it, and pages of shorter
rows, keep their range: a small fragment of short rows still takes the viewport path when
its camera cannot pass a row. Known limit: very long rows (8000 bytes, say) still leave
most of the window blank, since a page of 120 KiB holds only about fifteen of them. Tests:
`worker_long_row_tests.rs` (rows of 1200, 2400 and 8000 bytes and a mixed file, in Text
and Source, wrapped and unwrapped: every gesture moves its whole distance) and
`worker_chain_tests.rs`.

A scrollbar position's first read estimates the target from the shown page's density,
and one correction read is allowed (`refine_position`); Source places its camera on a
whole row (`viewport::source_minimum_origin`). A position over long rows can therefore
land slightly away from the released thumb. A piece of a row longer than one layout,
without a row break, keeps the viewport path: such a row shows as pieces, one per line.

**Layouts longer than one shaping run.** The renderer bounds the shaping runs of every
layout itself (`midnight-viewer-render`, `runs.rs`), so every layout it accepts, up to
256 KiB with tabs expanded (`MAX_TEXT_BYTES`), has the right caret, hit test, selection
and line range: parley 0.11 keeps a glyph cluster's offset inside its shaping run in 16
bits (`ClusterData::text_offset`). Markdown and document blocks, whose text adds
separators, tab-expanded text and every other caller are included. Parley starts a run
where the font features change, and a shaper drops a feature whose value is zero before
it looks the feature up (harfrust's `ot_map`, as in HarfBuzz), so every other piece of a
layout longer than 64 KiB gets the disabled feature `"MRUN" off` besides its own, and its
glyphs and advances stay those of one run. A piece ends at the last line start within
64 KiB, where shaping never joins glyphs. Only a line longer than 64 KiB is cut inside,
after its last whitespace that fits, else at its last grapheme-cluster boundary, where a
kerning pair or ligature across the cut can be lost. One grapheme cluster longer than
64 KiB is cut between two of its characters: its carets stay right, and parley, which
counts a cluster's glyphs in 8 bits (`ClusterData::glyph_len`), draws such a cluster wrong
anyway. Layouts up to 64 KiB are laid out as one run. Tests: `tests/long_layouts.rs`
(caret, hit test and selection past byte 65,536 against the same row laid out alone,
unwrapped, wrapped and with tabs) and `runs_tests.rs` (six scripts and layouts in three
typographies, cut at every line start, equal to one run glyph for glyph and pixel for
pixel).

A page's layout time grows with its size, on the viewer worker. A page's anchors take up
to 1.9 MiB (16 bytes per character of 120 KiB), and a chain holds one page besides the
shown one. Source pages of
densely highlighted long rows can still end at the 4,096-span budget before their bytes
do.

A frame of the shown surface whose pixels do not reach the camera the window shows is
refused. This happens when the engine pages or restyles for the camera a request asked
for, which runs the request's lookahead (up to 128 px) ahead of the shown camera: a page
restyled for highlighting starts at that camera's row. The camera then asks again for the
shown camera itself, without lookahead (`raster_camera::Camera::receive`), so every
gesture keeps its whole travel. Tests:
`a_burst_over_a_page_restyled_for_its_lookahead_keeps_its_whole_travel` in the
co-simulation and `raster_frontier_tests::a_page_restyled_around_the_lookahead_does_not_stop_a_burst`
in the window; `raster_frontier_tests::keys_and_wheel_keep_their_travel_*` guard
continuing and corrected refills.

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

Playback has dedicated, session-bound Play/Pause controls, a seek timeline and
position/duration text in a one-row media bar. Pending feedback ("Starting playback…",
"Pausing…", "Seeking…", "Buffering…") and errors lead the viewer's status line below
it (errors in the error colour), so a press never adds or removes a row and the video
above keeps its size. The active media layout hides
text Find, typography and scrollbars. Timeline seeks use media controls rather
than text scroll positions; seek and replay frames retain their media-session and
applied-control fences.

Resizing the canvas preserves the existing `MediaSessionId` and its generation. An admitted video
resize cancels the text scroll-position gate; it must not wait for a newer text
generation before accepting advancing video frames. Layout and media-control
checks still apply. Text and Source resize keep their distinct newer-generation
requirement. Image zoom/camera and retained image tiles keep their own presentation
paths.

Failures are shown at the front of the status line with the backend's sentence,
never a raw I/O text: "The video helper could not load its libraries (access denied):
<path>. The file does not let the helpers' sandbox read it, …" when the sandboxed helper
cannot open an FFmpeg library (one not granted to the workers' AppContainer, which the
host grants in a private installation, see the [worker SDK](WORKER_SDK.md)) or
"…: <path> is missing",
"The video helper stopped: <reason>" for a helper that ended, "This video could not be
read: <reason>" for a failed read of the file, "Reading or decoding the video took too
long" at the deadline, and "Media format or codec is unsupported" for content that no
decoder recognises. The bundled FFmpeg profile decodes VP8, VP9, Theora, FFV1, raw video
and Opus in Matroska/WebM, AVI, Ogg and MP4/MOV. When its helper reports that it has no
decoder for a video (MMV2 failure cause 5, with the codec's number and the decoders it
tried), the viewer starts the platform helper on the same capture: on Windows the
Media Foundation helper, which decodes H.264, HEVC, AV1, MPEG-2/4, VC-1/WMV and Motion
JPEG with AAC, MP3 or Windows Media Audio, the usual content of `.mp4` and `.wmv`
files; on Linux the helper that loads the distribution's FFmpeg, else GStreamer, which
also read `.wmv` and `.asf` (ASF) files. On Windows, a file named `.mp4`, `.m4v`,
`.mov`, `.3gp`, `.asf`, `.wmv` or `.avi` (by its suffix; the host does not parse the
video) starts with the Media Foundation helper instead, and the bundled helper follows
when that one cannot open it for any reason but cancellation. The first helper starts
while the viewer window lays out its canvas, as an image provider does, and opens the
video once the canvas size is known. A
codec that neither has is named with what provides it, for example "This video uses
HEVC; Windows has no decoder for it installed (HEVC Video Extensions)", on Windows N
"... (Media Feature Pack)", or on Linux the distribution's packages ("This video uses
HEVC; install your distribution's FFmpeg libraries (package ffmpeg) or GStreamer's
gst-libav plugin (package gstreamer1.0-libav) to view it."); the host words these
sentences from the codec number (`midnight-media-host` `system_decoders`), helpers send
no text. No helper shows a system dialog: each starts with critical-error and crash
dialogs off.

Evidence: [media record](../verification/2026-11-19-media-follow-up.md) (a silent
synthetic AVI: Play/Pause, paused resize, seek, end of file and replay). Audio, full codec
coverage and performance are not verified.

## Find

The **Find** button opens the search field through the same action as **Ctrl+F**.
**F3** finds the next match and **Shift+F3** the previous match; these are the default
shortcuts and can be remapped. The search text is kept when Find is opened again. The
button is offered for text, binary, Source, Markdown and HTML views; image and PDF
document views hide it. Known limit: Office HTML search covers the generated bounded
page, not the complete workbook or document.

## Source highlighting and dense pages

The independent Source plugin has 25 lexical profiles, including actual Lua, YAML,
CSS and Makefile styles; see its [catalogue](../../plugins/midnight-viewer-source/README.md).
`Makefile` and `GNUmakefile` reach it through whole-name association entries
(`makefile`, `gnumakefile`); see [viewer file types](../VIEWER_ASSOCIATIONS.md).
Lua keeps matched long-bracket strings/comments and short-string escape state.
YAML distinguishes mapping keys, quoted/plain/block scalars and flow punctuation;
C++ retains custom raw-string delimiters. These are lexical profiles, not semantic
analysis, language execution or complete-document validation.

The host colors each role (`midnight_viewer_engine::role_color`): keywords and
headings take the theme accent, strings, comments, numbers, types, functions and
links, and errors take fixed tints, and other roles the body text. Each color moves
toward black or white only as far as it must to reach `midnight_core::content_minimum`
against the page: 5:1, a margin over WCAG AA's 4.5:1, and 7:1 where the theme's body
text reaches 15:1 (the Contrast and High Contrast themes). Markdown quotes keep the
same contrast or take the body color.
`unselected_syntax_colors_of_every_theme_keep_their_contrast_on_the_page` in
`crates/midnight-viewer-ui/tests/selection_contrast.rs` checks every role in every
theme.

When styles and anchors exceed the wire budget, the page is cropped at exact
character/source boundaries while retaining styles: the beginning for forward
requests and the end for backward requests, with continuation flags for the rest.
The plugin does not clear all colours to fit a larger page. Distributed/recent
complete-line checkpoints stay within 256 slots; foreground replay remains
bounded. Cold distant positions may still show plain text.

The engine asks for preparation only at rest and only for what the shown page needs
(`worker_syntax.rs`). A file of at most 256 KiB, one preparation's walk, is prepared whole
once per revision unless its page already holds all of it in colour: every page read from
it then comes coloured, and a small file, whose page holds it whole and coloured as read,
is never prepared. In a larger file only the rows the page holds plain are prepared,
nearest the view first, and coloured in place: the page keeps its rows, selection and
prepared screens. Scrolling within the page prepares nothing, and "Preparing source
highlighting…" appears only when the visible rows stay plain at rest for 300 ms; rows show
plain until their colours arrive. See [Source highlighting](SCROLLING.md#source-highlighting)
and its [record](../verification/2026-11-22-source-highlighting.md).

## Live appearance

Open viewer and comparator windows follow the application's theme, density and interface
font size (`ViewerWindows::apply_appearance` in `midnight-viewer-ui`, `apply_appearance`
in `midnight-compare-ui`). Chrome and the document palette (foreground, background,
selection and accent; `restore_palette` in the backends) change without reopening or
reading the file again, and an inactive window uses the theme's inactive selection pair.
Document fonts, zoom and layout stay as they are. Office documents, author HTML CSS and
image or PDF pixels keep their own colors.
