# Long unwrapped rows in row groups: 22 November 2026

This record covers the third item of the [text scrolling
follow-up](2026-11-22-text-scrolling-follow-up.md). Unwrapped Text and Source rows wider than
the window stalled. A page of at most 120 KiB held 1.2 screens of 2400-byte rows, too few to
prepare screens ahead, and every refill shaped its whole page again for at most a guard
(256 px) of travel. In the step 2 measurement the 40-notch spin over such rows stalled 133 of
161 frames at 1x.

The results come from an isolated whole-tree export (main `4c8c17a6` plus this change) and a
work tree of the same code with red-run switches, built in their own target folders with
`CARGO_INCREMENTAL=0` on the shared Windows 11 development machine while other agents' builds
ran. Each figure is a single run, not a confidence interval. Nothing here was run in the
application window: no physical wheel or touchpad, and no Linux or macOS.

## Row groups

An unwrapped Text or Source page is a composite page (`tables::Composite`, as tables use) of
groups of whole rows, each its own parley layout (`TextRenderer::layout_rows`,
`midnight-viewer-render/src/rows.rs`). A group holds at most 8 KiB of rows and stays within
one layout's colour spans, styles and lines; a longer row is a group of its own.

- **Reuse.** A group's key is its options, colours and styles (`group_key`). A refill keeps a
  group of the shown page that the new page repeats whole, at the shift a source anchor of both
  pages gives (`Page::shift_to`), with the same key. It shapes only the rows it adds and the
  group that its new edge cuts.
- **Exactness.** Each group is broken into lines at its place in the page: parley's line `y`
  starts from the sum, in `f64` and in page order, of the line heights above it, as one layout
  sums them. A group's lines therefore lie where one layout puts them, bit for bit, whatever
  groups the page reused. Every group but the last ends after a line break; its own layout's
  empty line after it is left out of the line count, the scroll anchors and the page's height.
  A hit test picks the group whose first row's top is the last at or above the point, as parley
  picks a line by line tops (`Layout::line_for_offset`); a point on a group's own empty last
  line goes to its last row. A line at a height picks the group whose last row's bottom is the
  first below it. Rows of fallback fonts reach above and below their line, so these edges are
  not the groups' line positions: the first build placed groups by their line positions and
  failed the hit test at the row `row 001 café 東京`.
- **Placing reused groups.** A reused group is broken into lines again at its new place (7 to
  8 ms for 96 groups of 691,200 bytes in a release build). Its lines' paint reach
  (`glyphs::line_bounds`) moves with it instead of being measured again (42 to 47 ms for those
  groups): it only culls painting, with an ink reach wider than any glyph. A refill of that
  page that shapes 28,800 bytes takes 28 ms; laying the 720,000 bytes out afresh takes 211 to
  216 ms.
- **Bidirectional rows.** Parley resolves one layout as one bidirectional paragraph whose
  direction is that of its first strong character. A group of several rows with right-to-left
  letters would order them by the row its group started with, and so by the scroll history. A
  row with right-to-left letters, Arabic numbers or explicit embeddings is therefore a group,
  and a paragraph, of its own: its direction comes from its own first strong character, as the
  design asks ("bidi keeps the row's common context") and as Unicode's bidirectional algorithm
  does per paragraph. **Behaviour change:** one layout of a page gave every row the direction of
  the page's first strong character, so a right-to-left row could flip when the page started
  elsewhere; wrapped rows still do.
- **A parley defect.** Parley 0.11.1 counts a cluster at a wrong index in a line's advance when
  a ligature, combining sequence or emoji sequence starts a cluster outside the layout's first
  run: its line breaker passes an absolute cluster index to `Run::get`, which takes one
  relative to the run (`line_break.rs`, `run.get(self.state.cluster_idx + 1)`). Such a line's
  advance, and so a page's width and the highlight of a line selected whole, depend on other
  rows of the same layout: the emoji row of the tests measured 135.24 px in one layout of its
  text and 142.65 px alone, while its clusters sum to 142.66 px. Carets, hit tests, glyphs and
  copies do not use the line advance. The row group tests compare widths and the highlights of
  whole lines only where both layouts' advances equal their clusters'.

The renderer tests (`rows_tests.rs`):

- `row_groups_answer_and_paint_as_one_layout`: three texts (short and long rows, an empty row,
  tabs, CRLF, combining marks, emoji sequences, CJK, Hangul and Devanagari with fallback fonts,
  U+2028 and U+0085 inside a group, a last row without a line break, trailing empty rows) in two
  typographies (the default; serif 15 px at 1.25x, bold italic, tabular numbers, tab width 8,
  line height 1.5), at groups of one row, 7 bytes, 40 bytes and 8 KiB, against one layout of the
  same text. Width, height, line count and every scroll anchor; the caret at every character
  boundary; hit tests on a grid and on and around every line's top and bottom, and the line at
  each of those heights; the selection rectangles and copied text of every range between the
  offsets around each group edge (the edge, the last column and line break of the group's last
  row, the next row's first characters); the pixels without a selection and with one across
  every group edge, as a Find match is painted.
- `coloured_long_rows_in_groups_answer_and_paint_as_one_layout`: 300 coloured and styled rows.
- `a_row_with_right_to_left_text_is_a_paragraph_of_its_own`: the carets of a page starting at a
  right-to-left row equal those of a page starting earlier; every group size keeps each such
  row alone and answers and paints as a group per row.
- `a_refill_shapes_only_the_rows_it_adds`: refills forwards and backwards reuse every group of
  the old page the new one holds whole, shape at most the added rows and one group, and answer
  and paint as one layout.
- `changed_groups_are_shaped_again`: a new colour, other options and a cut last row.
- `groups_stay_within_one_layouts_limits`: densely coloured rows and 5,000 empty rows, which
  one layout refuses, are laid out in smaller groups.

Red runs, each with one mechanism switched off in the work tree (`ROWS_MUTATION`):

| Switched off | Failing tests | First failure |
| --- | --- | --- |
| Scroll anchors leave out a group's end | 4 | "scroll anchors" count |
| Hit tests and lines by group edges (tables' nearest part instead) | 4 | "hit test 113.1, -2" |
| Line count leaves out a group's empty last line | 5 | "lines" |
| Reuse checks the key | `changed_groups_are_shaped_again` | reused the recoloured group |
| Groups of whole rows (cut every 61 bytes) | 5 | group ranges, heights, `InvalidRange` |
| Groups found by their first row's top (line `y` instead) | `row_groups_answer…` | "hit test -3, 21.066172" |
| A point on a group's empty last line goes to its last row | `row_groups_answer…` | "hit test -3, 45.00758" |
| A right-to-left row is a group of its own | `a_row_with_right_to_left…` | "caret 69", x 49.6 against 0.0 |
| Lines by their bottoms (by tops instead) | `row_groups_answer…` | "line at 21.066172" |
| Groups within one layout's colour spans | `groups_stay_within…` | `LimitExceeded` |

## Pages that hold the prepared screens

Shaping only the rows a refill adds is not enough. A page must also hold the screens whose
strips are prepared, and a refill of long rows must not hold the camera.

- **Budget** (`worker_chain::Budget`). A request for a page of row groups keeps the rows it
  holds of the shown page and reads at most 120 KiB and 4,096 colour spans more, still in at
  most 16 ranges, up to 1 MiB (`MAX_ROWS_BYTES`) and 65,536 spans. The row a read continues
  (after the page's last line break) grows to at most 120 KiB and 4,096 colour spans, so that
  any group fits one layout. Any other page is one layout of at most 120 KiB, as before.
- **Thresholds** (`worker_raster.rs`). A page that is one layout starts a refill no further
  than half its span from its edge, as before, since a refill lays it out whole. A page of row
  groups starts one at the strips' own thresholds. A page of rows so long that one request
  cannot read every screen it asks for (`Worker::long_rows`) refills until it also holds the
  two screens a page holds beyond the strips (`Bands::growth_thresholds`), reads on after each
  refill until it does (`resume_raster_target`), and grows at its edge from where it starts
  when it already keeps the screen against the travel (a read from a row page's own start
  holds every row it has, `Worker::overlap`), while a request can still add to it.
- **The camera during a refill.** A refill of long rows reads up to 16 ranges, one plugin
  reply each; in the co-simulation that is one a worker turn. The worker held the camera where
  the refill started until the refill's page came, so the window stalled once it reached the
  end of the strips published before. Now a page of row groups lets its camera move on over the
  rows it holds, and the refill's page takes the camera where it is. Any other camera still
  waits for its refill's page.

The engine tests (`worker_row_group_tests.rs`):

- `a_refill_shapes_only_the_rows_it_adds`: three PageDowns over 2400-byte rows in Text and in
  Source shape at most the bytes they read plus one 8 KiB group per layout.
- `a_page_of_long_rows_grows_to_hold_the_prepared_screens`: after a PageDown and a rest the
  page holds 3.9 screens or more below the window and a quarter screen or more above it.
- `a_refilled_page_answers_copies_and_finds_as_a_fresh_layout`: at every group edge of a
  refilled page, its anchors, carets, hit tests, selections and copies equal those of its rows
  laid out afresh, which answer as one layout (the renderer tests above). One layout of such a
  page cannot be made: it is larger than 256 KiB.
- `the_camera_moves_on_while_a_refill_reads`: during a refill of 16 ranges the worker's camera
  is the window's at every frame.
- `wrapped_rows_keep_one_layout`.

Red runs: without reuse `a_refill_shapes_only_the_rows_it_adds` fails ("3769005 bytes shaped in
7 layouts for 838839 bytes read"). Each of the one-layout budget, the capped thresholds, the
strips' thresholds without the two extra screens, reading on after a refill and growing from
the page's own edge fails `a_page_of_long_rows_grows_to_hold_the_prepared_screens` (0.20 and
-0.02 screens, 0.93 and 1.12, 0.75 and 3.76, 0.50 and 1.42, 0.50 and 1.42 above and below the
window). A camera that waits for the refill fails `the_camera_moves_on_while_a_refill_reads`.

## Measurements

Release co-simulation (`measure_prepared_text_scrolling`, `measure_prepared_modes`), run back
to back: before is main's viewer code (a build of step 2 with its diagnostic switches off),
after is the work tree's build of this change with its red-run switches off. A gesture's frames
run until the view is at rest and no refill is pending, so they now include the reads ahead at
rest after it.

**Unwrapped 2400-byte rows at 1x, 100 px notch**

| Gesture | Stalls/frames | Layouts (KiB) | Reads |
| --- | --- | --- | --- |
| wheel 12 notches, 50 ms apart | 42/59 → 27/115 | 8 (959) → 5 (610) | 26 → 80 |
| wheel 40 notches, 25 ms apart | 137/165 → 5/117 | 19 (2280) → 4 (481) | 82 → 61 |
| touchpad flick | 48/104 → 0/103 | 15 (1800) → 12 (345) | 33 → 40 |
| PageDown x8, 120 ms apart | 206/237 → 55/163 | 24 (2880) → 7 (880) | 118 → 112 |
| wheel back up 12 notches | 48/62 → 8/117 | 8 (956) → 4 (487) | 26 → 63 |
| scrollbar drag 0.30 to 0.36 | 0/104 → 0/98 | 5 (600) → 5 (522) | 80 → 80 |

**Unwrapped 2400-byte rows at 2x, 100 px notch**

| Gesture | Stalls/frames | Layouts (KiB) | Reads |
| --- | --- | --- | --- |
| wheel 12 notches, 50 ms apart | 50/71 → 27/87 | 8 (959) → 4 (487) | 28 → 64 |
| wheel 40 notches, 25 ms apart | 178/211 → 22/143 | 25 (3000) → 6 (738) | 90 → 93 |
| touchpad flick | 59/106 → 0/105 | 15 (1800) → 13 (226) | 36 → 25 |
| PageDown x8, 120 ms apart | 257/291 → 79/180 | 32 (3840) → 7 (887) | 128 → 112 |
| wheel back up 12 notches | 60/76 → 0/120 | 9 (1075) → 4 (485) | 27 → 63 |
| scrollbar drag 0.30 to 0.36 | 0/86 → 0/87 | 4 (480) → 4 (480) | 64 → 64 |

**Unwrapped 2400-byte rows at 1x, 60 px notch**

| Gesture | Stalls/frames | Layouts (KiB) | Reads |
| --- | --- | --- | --- |
| wheel 12 notches, 50 ms apart | 21/49 → 7/84 | 10 (1199) → 4 (487) | 18 → 64 |
| wheel 40 notches, 25 ms apart | 79/101 → 0/109 | 13 (1560) → 5 (414) | 51 → 48 |
| touchpad flick | 19/103 → 0/103 | 20 (2400) → 12 (155) | 27 → 20 |
| PageDown x8, 120 ms apart | 181/210 → 58/158 | 24 (2880) → 7 (875) | 117 → 112 |
| wheel back up 12 notches | 28/52 → 0/92 | 9 (1075) → 3 (367) | 18 → 48 |
| scrollbar drag 0.30 to 0.36 | 0/97 → 0/99 | 5 (600) → 5 (515) | 80 → 80 |

**Unwrapped 2400-byte rows at 2x, 60 px notch**

| Gesture | Stalls/frames | Layouts (KiB) | Reads |
| --- | --- | --- | --- |
| wheel 12 notches, 50 ms apart | 28/51 → 8/89 | 8 (959) → 4 (487) | 18 → 64 |
| wheel 40 notches, 25 ms apart | 96/123 → 0/97 | 16 (1920) → 7 (411) | 54 → 48 |
| touchpad flick | 23/103 → 0/103 | 17 (2040) → 15 (170) | 24 → 19 |
| PageDown x8, 120 ms apart | 227/261 → 66/165 | 32 (3840) → 7 (873) | 127 → 112 |
| wheel back up 12 notches | 34/55 → 0/115 | 7 (836) → 3 (365) | 17 → 48 |
| scrollbar drag 0.30 to 0.36 | 0/105 → 0/101 | 5 (600) → 5 (515) | 80 → 80 |

Each refill shaped the rows it added and at most the group its new edge cut. Counted in a
diagnostic run of the same spin: 125,209, 127,129, 130,489 and 126,169 bytes shaped for
122,880 added, and 109,849 for 106,288.

**Target not met reliably.** The 40-notch spin with the 100 px notch at 1x stalled 5 frames in
the measurement and 11 in a second run of the same build; a build before its last change
(which moved the reused groups' paint reach instead of measuring it again) stalled 0 and 18.
What remains is throughput under load. A refill's worker turn takes 80 to 130 ms: about 37 ms
shapes its 120 KiB, the rest places the reused groups and paints the 1 to 3 strips at its old
edge, whose hash its new edge changes. The co-simulation answers a plugin read, 8 KiB or about
three rows of 2400 bytes, only between worker turns, so such a turn holds the reads back; the
window needs about four such rows a frame at the spin's peak. Shaping a refill's rows as its
replies arrive, a few groups a turn, would keep the turns short; it is not done here.

Other rows stall no frame, before or after: wrapped 2400-byte rows, 60-byte rows over 100 MB and
Source over generated Rust, at both notches and scales. Unwrapped 60-byte rows and Source shape
less (30 KiB instead of 54 KiB for the 40-notch spin) and now read once during a touchpad
flick (none before), where the uncapped thresholds start a refill. Opening a view (the modes
measurement) took 8.7 ms to lay out 35 KiB of 60-byte rows at 1x after, 15.2 ms before, in
single runs.

## Rasterization while dragging over wrapped rows (13.1 → 20.5 Mpx)

In the step 2 measurement the scrollbar drag from 0.30 to 0.36 over wrapped 2400-byte rows at
1x rasterized 20.5 Mpx against 13.1 Mpx before. A diagnostic build of step 2 (not in the tree)
could switch each of its mechanisms off. Only the read-ahead at rest mattered: without it the
drag rasterized 13.10 Mpx in 4 layouts and 64 reads, as before step 2, and with it 20.82 Mpx in
3 layouts and 48 reads. Keeping strips across raster contexts and hashing the strips at a
page's edge changed neither figure.

Counted over the drag's 60 positions, the window showed 7 of them, from 62 strips (95 at 2x),
with the read-ahead, and 4, from 37 strips (70 at 2x), without it. That is 8.9 strips per shown
position against 9.3 at 1x, and 13.6 against 17.5 at 2x. The read-ahead lets more thumb
positions land on a page that is already read, so more of them are shown, and each shown
position paints the strips its window needs. It is not waste, so it stays.

## Tests

In the export, debug builds with `CARGO_INCREMENTAL=0`:

- `cargo test --no-run -p midnight-viewer-render -p midnight-viewer-engine -p midnight-viewer-ui
  -p midnight-compare-engine --lib --tests --locked --offline`, then each test executable:
  `midnight-viewer-render` 46 passed and its integration tests 60 passed (3 ignored),
  `midnight-viewer-engine` 594 passed (17 ignored), `midnight-viewer-ui` 226 passed and its
  integration tests 35 passed, `midnight-compare-engine` 177 passed (4 ignored).
- With `--features acceptance-seams`, `midnight-viewer-ui` 232 passed and its integration tests
  35 passed.
- `cargo clippy -p midnight-viewer-render -p midnight-viewer-engine -p midnight-viewer-ui
  -p midnight-compare-engine -p midnight-explorer --all-targets --locked --offline -- -D warnings`
  found two lints in `rows_tests.rs` (a vector of one range, `Some(x).filter`). After their fix
  it reported nothing, and the renderer's tests passed again (46, and 60 with 3 ignored).
- `cargo fmt --all -- --check` reported nothing.

The wrapped, Markdown, hex, table and document tests are unchanged and pass in those runs:
apart from the new test modules (`rows_tests.rs`, `worker_row_group_tests.rs`), the only test
file changed is `worker_tests.rs`, by the line that includes the new module.
