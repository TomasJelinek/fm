# Source highlighting while scrolling: 22 November 2026

The owner reported that the Source viewer showed "Preparing source highlighting…" far too
often, and that scrolling was sometimes not smooth. Their example was a small C++ file: 150
rows, 6 KB, rows of up to 120 characters. That file is the owner's and is not copied anywhere:
the tests use synthetic C++ of the same shape (`worker_highlight_tests::cpp`: 150 rows of
5,935 bytes, rows of up to 116 bytes, LF line ends, no tabs, `//` comments), 3,000 rows of it
(120 KB) for a file within one preparation, and 56,000 rows (2.2 MB) for a large one.

The results come from isolated whole-tree exports on the shared Windows 11 development
machine while other agents' builds ran, built in their own target folder with
`CARGO_INCREMENTAL=0`. "After" is main `5ccd0eb7` with this change. "Before" is main
`dc022719` with the row groups as submitted (integrated as `8a2b87c6`), whose viewer engine
and renderer sources are byte for byte those of `5ccd0eb7` apart from the Office viewer's
files, plus a test-only adapter that lets the new co-simulation drive the old engine's
admissions. The untimed counts came out the same in three runs of the old engine; timed
figures are the range of three runs. None is a confidence interval. Nothing here was run in
the application window: no physical wheel or touchpad, and no Linux or macOS.

## Cause

The engine kept one preparation target: the exact view (`worker_syntax.rs`). Every scroll
retargeted it at the visible row, so every pause at a new place sent the Source viewer a
`PrepareSource` for that row and showed the notice at once, and a refresh followed when the
preparation was ready. The Source viewer already colours a page it reads whenever it holds
the lexer state at a line end at most 32 KiB before the page, and always at the file's start,
so the 6 KB file came coloured as read: every preparation, notice and refresh after a scroll
was needless. In the co-simulation below, twelve wheel notches over that file prepared it 29
times and showed the notice 28 times. The refresh could also replace the shown page by its
own: after eight PageDowns the page of the 6 KB file held 1 KiB, one screen, instead of the
whole file, so the next gesture needed new pages, and every refresh laid the page out again.

## Change

All of it is in the engine (`crates/midnight-viewer-engine/src/worker_syntax.rs`, with call
sites in `worker.rs`, `worker_position.rs` and `worker_loop.rs`). The Source viewer and the
viewer API are unchanged.

- **What is prepared.** At rest the worker looks at the shown page (`source_target`). A file
  of at most 256 KiB (`MAX_SCAN_BYTES`, one request's walk) is prepared whole, once per
  revision, unless the page holds all of it with no plain rows: a walk to the end leaves 128
  spread checkpoints at most a few KiB apart, so every page read from that revision comes
  coloured, and rows that came plain before are read again without preparing. In a larger
  file the worker prepares only rows the page holds plain: stretches of 2 KiB without a
  colour span (`PLAIN_GAP`), or a whole shorter page without one. It takes those nearest the
  view first: rows that reach the view from their start when that lies at most 64 KiB before
  the view's end (`REFRESH_REACH`, so that one refresh reads them through the view), else from
  a screen above the view; rows above the view back from their end.
- **How they are coloured.** The refresh reads the prepared rows (up to 4,096 rows, joined to
  120 KiB) and replaces exactly their styles in the shown page (`restyle_source_page`). The
  page keeps its text, anchors, selection and prepared screens; before, the refresh could
  replace the page by its own page. Row groups whose colours and styles stay keep their
  shaping. A page of row groups takes up to 65,536 colours (`MAX_ROWS_SPANS`), not a Source
  packet's 4,096. A refresh that completes while the view is not at rest keeps its page until
  the next rest.
- **No loops.** Rows a refresh leaves plain hold no token: they are settled and not prepared
  again while the page holds them. Rows found plain are remembered while the page holds them,
  so what a refresh leaves of them is prepared next however few rows it is; rows read again
  with colours no longer count. A target that does not change keeps its admitted work, and a
  finished one is not prepared again.
- **Scrolling.** Moving the camera within the page no longer changes the target or drops the
  work under way. A page request (a refill, a seek) still cancels a refresh under way.
- **The notice.** "Preparing source highlighting…" is shown only after the visible rows stay
  plain at rest for 300 ms (`NOTICE_DELAY`); the worker loop wakes for that deadline. Moving
  the camera, asking for a page or colouring the rows removes it.
- **Memory.** No second page is kept: a refresh's page (at most 120 KiB of text) is merged
  into the shown page and dropped. The rows found plain and settled are merged source ranges
  pruned to the shown page. The Source viewer's checkpoints and preparation state stay within
  their fixed 256 records and 64 KiB per file, so the window coloured beyond the page is
  bounded by what those checkpoints reach, not by memory the engine holds.

## Measurements

The worker's co-simulations (`worker_frontier_tests.rs`, untimed, and
`worker_prepared_tests.rs`, timed at 60 Hz with the worker's real turn costs) now carry a model
of the Source viewer's lexical state (`worker_highlight_tests::Lexer`). It keeps the Source
viewer's checkpoints with their limits (128 spread from 2 KiB apart, doubling when full, and
the 126 latest line ends; a lexed stretch keeps 17 line ends at least 4 KiB apart and its last
64). It colours a page only when a checkpoint lies at most 32 KiB before the page's anchor, or
when the page is the reserved target of a ready preparation, and then keeps the line ends it
lexed. A preparation walks from the checkpoint nearest before its target, at most 256 KiB per
request, keeping the line ends it passes, and its reply comes after its bytes at an assumed
16 MiB/s. The window is 1312x720 at 1x and 2624x1440 at 2x, the wheel notch 100 px. Release
builds.

### Work per gesture (untimed)

Preparations / refreshes / notices; "plain" counts frames that showed a row with a token
uncoloured. Gestures run in this order, from rest to rest.

| Gesture | 150 rows, before | after | 3,000 rows, before | after | 56,000 rows, before | after |
| --- | --- | --- | --- | --- | --- | --- |
| Opening the file | 1 / 1 / 1 | 0 / 0 / 0 | 1 / 1 / 1 | 1 / 0 / 0 | 1 / 1 / 1 | 0 / 0 / 0 |
| Jump (60 %, 30 %) | | | 2 / 1 / 2, 5 plain | 0 / 0 / 0 | 3 / 1 / 2, 8 plain | 3 / 1 / 0, 8 plain |
| 12 wheel notches down | 29 / 2 / 28 | 0 / 0 / 0 | 27 / 2 / 27 | 0 / 0 / 0 | 27 / 2 / 27 | 0 / 0 / 0 |
| 12 wheel notches up | 27 / 2 / 27 | 0 / 0 / 0 | 26 / 2 / 26 | 0 / 0 / 0 | 26 / 2 / 26 | 0 / 0 / 0 |
| Touchpad flick | 57 / 1 / 34 | 0 / 0 / 0 | 56 / 1 / 34 | 0 / 0 / 0 | 56 / 1 / 35 | 0 / 0 / 0 |
| 8 PageDowns | 4 / 2 / 3 | 0 / 0 / 0 | 4 / 1 / 4 | 0 / 0 / 0 | 4 / 1 / 4 | 0 / 0 / 0 |
| 8 PageUps | 5 / 1 / 3 | 0 / 0 / 0 | 4 / 1 / 4 | 0 / 0 / 0 | 4 / 1 / 4 | 0 / 0 / 0 |
| Scrollbar drag 30 % to 36 % | 54 / 1 / 8 | 0 / 0 / 0 | 2 / 1 / 2 | 0 / 0 / 0 | 4 / 1 / 4 | 0 / 0 / 0 |

No frame showed a row uncoloured during the gestures, before or after; only the jumps did,
until their rows were coloured. After the change every page read from the 3,000-row file came
coloured (76 of 76), and after the jump into 56,000 rows the walk of 661 KiB left checkpoints
that coloured every page the gestures read. The page at rest held the whole 150-row file
(5.8 KiB, 3.4 screens) after the change; before, the refresh's page replaced it and it held
1 to 5 KiB (1.0 to 3.4 screens), 1 KiB after the PageDowns. In the larger files the page at
rest held 10 to 12 KiB, 6.1 to 7.1 screens of these rows, before and after: the prepared text
window (the window, a screen behind, two or three ahead and two more).

### Timed

Three runs of each build in turn; a range is the smallest and largest of the three. Stalls
count frames on which the camera had travel left and did not move; worker time is the worker's
own wall time over the gesture (inputs, plugin events, layout, painting and highlighting).

| File, scale | Gesture | Stalls | Preparations | Notices | Worker time, ms |
| --- | --- | --- | --- | --- | --- |
| 150 rows, 1x | 12 wheel notches, 50 ms apart | 0 → 0 | 25 → 0 | 23 → 0 | 6-9 → 3 |
| 150 rows, 1x | 40 wheel notches, 25 ms apart | 0 → 0 | 61 → 0 | 8 → 0 | 183-212 → 0 |
| 150 rows, 1x | touchpad flick | 0 → 0 | 102 → 0 | 1 → 0 | 307-322 → 0 |
| 150 rows, 1x | 8 PageDowns, 120 ms apart | 0 → 0 | 8 → 0 | 8 → 0 | 68-79 → 0 |
| 150 rows, 1x | 12 wheel notches back up | 0 → 0 | 25 → 0 | 24 → 0 | 35-44 → 0-1 |
| 150 rows, 1x | scrollbar drag 30 % to 36 % | 0 → 0 | 45 → 0 | 8 → 0 | 24-29 → 3-5 |
| 150 rows, 2x | 12 wheel notches, 50 ms apart | 0 → 0 | 25 → 0 | 23 → 0 | 14-17 → 9-10 |
| 150 rows, 2x | 40 wheel notches, 25 ms apart | 0 → 0 | 60 → 0 | 7 → 0 | 508-552 → 0 |
| 150 rows, 2x | touchpad flick | 0 → 0 | 102 → 0 | 1 → 0 | 752-1006 → 0 |
| 150 rows, 2x | 8 PageDowns, 120 ms apart | 0 → 0 | 8 → 0 | 8 → 0 | 177-255 → 0 |
| 150 rows, 2x | 12 wheel notches back up | 0 → 0 | 24-25 → 0 | 23 → 0 | 82-96 → 1 |
| 150 rows, 2x | scrollbar drag 30 % to 36 % | 0 → 0 | 43-44 → 0 | 7 → 0 | 58-83 → 8-9 |
| 56,000 rows, 1x | jump to 30 % | 0 → 0 | 3 → 3 | 1 → 0 | 44-55 → 46-61 |
| 56,000 rows, 1x | 12 wheel notches, 50 ms apart | 0 → 0 | 24 → 0 | 24 → 0 | 23-29 → 20-23 |
| 56,000 rows, 1x | 40 wheel notches, 25 ms apart | 0 → 0 | 35 → 0 | 35 → 0 | 76-86 → 70-93 |
| 56,000 rows, 1x | touchpad flick | 0 → 0 | 61 → 0 | 42 → 0 | 34-37 → 28-33 |
| 56,000 rows, 1x | 8 PageDowns, 120 ms apart | 0 → 0 | 8 → 0 | 8 → 0 | 149-194 → 102-127 |
| 56,000 rows, 1x | 12 wheel notches back up | 0 → 0 | 23 → 0 | 23 → 0 | 22-28 → 17-24 |
| 56,000 rows, 1x | scrollbar drag 30 % to 36 % | 0 → 0 | 4 → 0 | 2 → 0 | 75-84 → 82-92 |
| 56,000 rows, 2x | jump to 30 % | 0 → 0 | 3 → 3 | 1 → 0 | 95-145 → 110-115 |
| 56,000 rows, 2x | 12 wheel notches, 50 ms apart | 0 → 0 | 24 → 0 | 24 → 0 | 40-54 → 30-37 |
| 56,000 rows, 2x | 40 wheel notches, 25 ms apart | 0 → 0 | 35 → 0 | 35 → 0 | 104-144 → 117-130 |
| 56,000 rows, 2x | touchpad flick | 0 → 0 | 61 → 0 | 42 → 0 | 58-80 → 52-63 |
| 56,000 rows, 2x | 8 PageDowns, 120 ms apart | 0 → 0 | 8 → 0 | 8 → 0 | 199-312 → 156-228 |
| 56,000 rows, 2x | 12 wheel notches back up | 0 → 0 | 23 → 0 | 23 → 0 | 46-62 → 41-48 |
| 56,000 rows, 2x | scrollbar drag 30 % to 36 % | 0 → 0 | 3 → 0 | 2 → 0 | 143-196 → 150-168 |

No gesture stalled a frame over the 150-row or the 2.2 MB file, before or after, so the
co-simulation does not reproduce a stall from highlighting. What the change removes is work on
the worker's thread while the content moves: over the small file the old engine prepared at
almost every pause in a gesture and spent up to 1 s of the worker's time on a flick that moved
nothing (it was at the end), with single turns of up to 41 ms; in the real viewer that time
competes with the frames the window asks for. After the change the worker's time on the small
file is that of the strips alone, its longest turn 4.6 ms, and the large file's falls within
or below the old range. The 40-notch spin moves 555 of 4,000 px over the small file (1,111 at
2x) before and after: it reaches the file's end.

Over 400,000 rows (16 MB, after only: the old engine was not measured on it), a jump to 80 %
walks 12.8 MB from the file's start in 50 requests. Its visible rows show plain for about 100
frames (1.7 s at the assumed rate), the notice appears once after 300 ms and goes when they
are coloured, and no gesture after it prepares or notices anything. Turning back up past the
page prepares and refreshes once at the next rest: after a walk that long the Source viewer's
spread checkpoints lie more than 32 KiB apart. The first gesture after the jump, 12 wheel
notches, stalled 1 frame at 1x and 2 at 2x in each of the three runs, on a worker turn of 26
to 43 ms that read and laid out the next rows; it did no highlighting work.

## Tests

New tests (`crates/midnight-viewer-engine/src`):

- `worker_highlight_tests.rs`: the lexical model, the synthetic C++, and three co-simulation
  tests counting preparations, refreshes, notices and frames that show a row with a token
  uncoloured, per gesture (12 wheel notches down and up, a touchpad flick, eight PageDowns and
  PageUps, a scrollbar drag from 30 % to 36 %): the 150-row file prepares, refreshes and
  notices nothing, opened or scrolled; the 3,000-row file is prepared once, when opened, and
  never again, also after a jump to 60 %; in 56,000 rows a jump to 30 % prepares the rows it
  shows plain and colours them, and then no gesture prepares, refreshes or notices anything.
- `worker_prepared_tests.rs`: `a_small_source_file_stalls_no_frame_and_prepares_nothing`, the
  timed co-simulation of the measured gestures over the 150-row file: no stalled frame, no
  preparation, refresh or notice, no plain frame.
- `worker_syntax_tests.rs`: the notice waits 300 ms of plain visible rows at rest and goes when
  they are coloured; it never shows while the camera moves; a file within one walk is prepared
  whole once and its plain rows then only refreshed; a refresh that completes away from rest
  colours its rows at the next rest.
- `worker_source_overlap_tests.rs`: a refresh away from the view colours its rows and the
  visible rows come next, from the start of their plain rows, or from a screen above the view
  when those start more than 64 KiB before it; rows a refresh leaves plain are settled and
  never prepared again; what a refresh leaves of rows found plain is prepared next however few;
  rows read again with colours are not prepared; a page of row groups is coloured past a
  packet's 4,096 colours.

Red first: the co-simulation tests, and the new or changed unit tests that the old engine can
compile, ran against it (the "before" tree with the measurement adapter) and failed. Opening
each co-simulated file already prepared, refreshed and showed the notice once: `(1, 1, 1)`
where `(0, 0, 0)` (150 and 56,000 rows) and `(1, 0, 0)` (3,000 rows) are wanted; the timed
test failed the same way. A file within one walk was prepared at the view (anchor 8230,
forwards), not whole from its end; a refresh completed away from rest was dropped; the cropped
refresh replaced the page (it started at 8306, not 8192); a scroll within the page retargeted
the preparation (anchor 8268 in a new epoch); the restyle of a page of 4,000 colours stopped at
the packet cap (4,000 colours kept, 4,500 wanted); a refresh starting inside the visible row
was not applied. The tests of the notice delay, of the targets and of found and settled rows
name functions the old engine does not have and were not compiled against it.

Changed tests, which encoded the old behaviour:

- `in_page_scroll_retargets_latest_visible_anchor_and_keeps_admitted_prefix_work` became
  `in_page_scroll_keeps_the_target_and_its_admitted_work`.
- `excluded_visible_caret_gets_one_paced_corrective_target_then_eventual_styles` and
  `a_second_offscreen_corrective_reply_settles_until_the_visible_anchor_changes` became the
  overlap tests above: there is no corrective target any more.
- `cropped_dense_restyle_keeps_exact_source_selection_visual_anchor_and_retained_camera` now
  expects the page to keep its rows (start 8192, the same selection indices); the refresh
  restyles the rows it holds.
- `source_prepared_partial_leading_row_keeps_exact_camera_until_complete_style_refresh` now
  expects a refresh that starts inside the visible row to colour the rows it holds, at the
  same exact camera; the complete refresh still shows the reference pixels.
- `actual_aggregate_style_budget_exhaustion_keeps_the_loaded_selection_and_page` runs on a
  wrapped page, one layout, whose cap is still a packet's.

Results on the export:

- `midnight-viewer-engine --lib` (debug): 611 passed, 0 failed, 19 ignored (the measurements
  among them), run from the package directory with four test threads. The new and changed
  tests (52 of them, filtered by module) then passed 30 runs in a row. Before the rows above
  the view were read from their start (`REFRESH_REACH`), the large-file co-simulation failed
  2 of 17 such runs: a PageUp right after the jump showed rows above the view plain for a
  frame, because they were prepared in a second cycle that the rest before it did not wait
  for.
- Clippy on `midnight-viewer-engine` with `--all-targets -D warnings`: no warning.
- `cargo fmt --all -- --check`: clean.

## Not verified

- The application window: no physical wheel, touchpad or scrollbar, no screen. The notice
  delay and the refresh's restyle were exercised only by the worker's tests and the
  co-simulations.
- The Source viewer's real walk rate: the co-simulation assumes 16 MiB/s. At that rate the
  2.2 MB jump's walk of 661 KiB takes about 40 ms; the notice would need the visible rows to
  stay plain for 300 ms, a walk slower than about 2.2 MB/s for that jump.
- Rows without any token in long stretches (prose in XML or Markdown, long lists of bare
  identifiers) are taken for plain rows: each such stretch of the page is prepared and read
  once more per page, then settled. Nothing measures that cost here.
- Walks longer than about 2.3 MiB: the Source viewer's 128 spread checkpoints then lie up to
  about 44 KiB apart (a model of its checkpoint store, rows of 40 bytes), more than the 32 KiB
  a read may lex before its page, so pages read backwards far from the walk's end can come
  plain and are prepared at the next rest.
- Linux and macOS, and the full application test suites.
