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
