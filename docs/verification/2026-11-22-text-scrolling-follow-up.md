# Text scrolling follow-up: 22 November 2026

This record covers four items that the [prepared text window](2026-11-21-prepared-text-window.md)
left open, because the owner's complaint was choppy scrolling. It also records a seam that lets
the acceptance suite drive viewers with native input.

The results come from an isolated whole-tree export (main `007c29c8` plus the prepared text
window and the panel's `motion_preference` files, then this change), built in its own target
folders with `CARGO_INCREMENTAL=0` on the shared Windows 11 development machine while other
agents' builds were running. Each figure is a single run, not a confidence interval. Nothing
here was run in the application window: no physical wheel or touchpad, and no Linux or macOS.

## The co-simulation's window now glides like the real one

The prepared-window co-simulation (`worker_prepared_tests.rs`) gave the window each frame of a
wheel glide as direct pixels, so every camera request was *settled*. The real camera
(`raster_camera::Camera::request`) reports a request as settled only once its wheel motion has
ended, and the engine reads the next page early only while a request is unsettled. The model
window now takes a `gliding` flag from the measured glide (`worker_frontier_tests.rs`). With
it, the 40-notch spin over 100 MB at the 100 px notch (about five screens a second) stalls
no frame, before or after this change. The 3 and 4 stalled frames that the earlier record
reported for that spin came from the model, not from the viewer. Every measurement below
uses the corrected model, before and after.

## 1. A Source scrollbar drag painted the window again for every position

**Defect.** In the Source view a scrollbar drag outside the shown page reads no page while
the thumb moves. Each position still started a new raster context, and a new context cleared
every prepared strip. The worker therefore painted the window's strips again for each
position. The window converted and uploaded them again too, because it keyed its converted
strips by surface.

**Fix.**
- Strips outlive a new raster context. Before they are published again, they are checked
  against their content hash (`worker_raster.rs` `Retained::reset`, `worker_bands.rs`).
- A strip at a page's edge lacks the ink of the rows beyond it. It used to have no hash and
  was never kept. It now hashes the page's edge, so a page with the same edge reproduces it.
- Strip IDs are unique in the process. The window keys its converted strips and images by ID,
  width and gutter, not by surface (`pixel_preparation.rs`, `raster_presentation.rs`).
- Because strips now outlive contexts, a new font, size, scale, margin or Markdown preset
  drops them all (`Key::typography`). A font with the same line height keeps every row in its
  place, so the content hash alone would keep the old font's pixels.

**Evidence.** `a_scrollbar_drag_paints_no_strip_again_while_its_page_stays` drags the
scrollbar in a 200,000-line Rust file at 1x for 61 frames without a page arriving. Before this
change the window was given 5 strips again over those frames. With strips that outlive
contexts but edge strips without a hash, it was given 2. With this change it is given none.
`a_new_font_with_the_same_line_height_paints_every_strip_again` changes a Markdown page's
font without moving a row: every strip is painted again. With the typography key mutated to a
constant, the strips of the former font are kept. That run fails with "strips of the
former font were kept: [(0.0, 1), (256.0, 2)] -> [(0.0, 1), (256.0, 2)]".

## 2. Late refills at fast spins: a camera at rest read nothing ahead

With the corrected model the fast spin no longer waits (above). Keys still did. A key leaves
the camera settled (PageDown moves a whole screen and stops), and a settled camera read the
next page only once it left the page. So eight PageDowns 120 ms apart over 100 MB stalled 3
frames at 1x in `page_down_never_waits_for_the_next_page` before this change, with a lag of
219 px.

**Fix.** A camera request at rest now starts the refill along its direction of travel too
(`worker_raster.rs` `apply_raster_target_with`). This applies to the Text, Source and hex
views. A Markdown page ends at whole blocks, which a reply may not pass
(`Continuation::seam`). The first version also prefetched Markdown pages at rest. The
existing `a_retained_pan_past_a_tall_table_continues_at_the_page_boundary` then failed: a
prefetch inside a tall table read the table again and lost the seam. Markdown pages now read
on only while the camera glides or once it leaves them. The PageDown test stalls no frame
after the change.

## 3. Unwrapped long rows: the defect, and why this step does not fix it

**Defect.** The window waits on most frames when Text or Source rows are wider than the window
and wrap is off. In the case measured, the rows are 2,400 bytes in a 1312 × 720 window at 1x,
with rows about 17 px tall.

- A page is one text layout of at most 120 KiB (`PAGE_BYTES`). That is 51 such rows, or 852 px,
  1.2 screens, joined from up to sixteen plugin reads of at most 8 KiB.
- The page cannot hold the window and a 256 px guard on each side. `camera_span` therefore lets
  the camera move one guard (256 px) per page, and the window shows blank space below the page's
  last row meanwhile. No strip can be prepared beyond the page.
- Every refill lays out its whole page again, about 120 KiB of text. Each row is shaped in full,
  although about 160 of its 2,400 columns show. That costs about 55 ms in a release build and
  150 ms in a debug build, for an advance of at most 256 px (about 15 rows, 36 KB of new text).
- Refills start halfway through that span, so one page can carry the camera only about
  2,300 px/s. A fast spin needs about 4,000 px/s.

In the release co-simulation at the 100 px notch, the 40-notch spin stalls 133 of 161 frames,
eight PageDowns stall 192 of 223 and a touchpad flick 44 of 106, as before this change
(139, 194 and 42)
([measurements](#measurements)). A page budget of 240 KiB, tried during this step, still
stalled 44 of 72 frames of the spin in the debug co-simulation and broke two chain tests, so it
was reverted.

**Proposed fix, not in this change.** Shape only what a refill adds:
- Lay out unwrapped Text and Source pages as a stack of row groups. These would be the
  renderer's composite pages, which tables already use.
- Keep the groups a refill's page shares with the shown page (the same text, colours, styles and
  options), and shape only the new rows.
- Then let a page of long rows hold the prepared screens. Each refill would still shape only
  about the rows it adds.

This changes the renderer's layout API (`midnight-viewer-render`) and the caret, selection and
scroll-anchor mapping at group boundaries, including a group's trailing line break. It needs an
interface review before implementation and its own tests. Horizontal windowing, which shapes
only the visible columns of long rows, is the alternative. It would also change copy, find and
selection mapping.

## 4. Viewers honour reduced motion

The viewers read Windows' animation effects (`SPI_GETCLIENTAREAANIMATION`) through the
panels' `motion_preference` module (`midnight_ui::system_reduces_motion`). The application
gives that reader to the viewer windows (`ViewerWindows::set_motion_preference`,
`midnight-explorer` `viewers.rs`). Each window reads the setting with its first native event
and whenever it gains the focus, together with the wheel setting.

With animation effects off:
- A wheel notch and a scroll key move their whole distance on the next frame. This covers the
  Text, Source, Markdown and hex views (`raster_motion`), PDF and image tiles (`image_motion`)
  and HTML pages (`web_input`).
- `NativeScroll` synthesises no touchpad momentum (`midnight-ui-input`
  `NativeScroll::set_reduce_motion`, a new method). The system's own touchpad momentum remains
  the system's.

Tests:
- `with_reduced_motion_a_notch_moves_whole_on_the_next_frame` (text and HTML)
- `with_reduced_motion_a_touchpad_flick_has_no_momentum`
- `with_reduced_motion_a_notch_and_a_key_move_a_tiled_image_at_once`

All three failed before the change and pass after it. Not verified: the Windows setting itself.
No test changes the system setting.

## Acceptance seam for native viewer input

The acceptance suite runs viewers on Slint's software platform, where winit never runs, so the
viewer's native observer saw nothing. `midnight_viewer_ui::acceptance_seams` (feature
`acceptance-seams`, which only `midnight-acceptance` is to enable) gives that observer the
events winit would deliver:
- `wheel` moves the pointer and turns the wheel, in lines or touchpad pixels with a phase.
- `focus` gains or loses the system focus.
- Anything the observer leaves alone reaches Slint as winit's backend converts it.
- The wheel frames (`RedrawRequested`) follow every 8 ms while the window moves and for 250 ms
  after an input.
- `override_settings` replaces the lines per notch and the reduced-motion setting for the test
  process until its guard drops.

The window's observer is the same closure in both builds. It now holds its wheel state in a
shared cell. It takes native shortcut authority only when winit backs the window, so seam events
leave a software window's Slint keys in charge. The panels' seam in `midnight-ui` follows the
same rules.

Tests (`acceptance_seams_tests`, with the feature):
- a seam notch moves the text as a native notch does, frame for frame
- overridden settings (one screen per notch, less motion) apply from the next focus until their
  guard drops
- nested guards restore in turn
- focus reaches the window
- seam events leave the Slint keys in charge of digit shortcuts
- other and closed windows get nothing

The application's feature resolution leaves the feature off (`cargo tree -p midnight-explorer -e
features -i midnight-viewer-ui` reaches the crate from `midnight-explorer` with its
`default` feature only).

## Measurements

Release co-simulation (`measure_prepared_text_scrolling`, `PREPARED_NOTCH=100`), before
and after run one after the other from the same release build settings. The before build is
the prepared text window with this change's tests. A second pair, run right after, gave the
same picture: PageDown stalled 4 to 6 frames before and none after, and the long-row
gestures stalled within about a tenth of the first pair. The table leaves out gestures that
ran the same before and after. The raw output is in
`_build/agent-txt/measure/` (`step2-before-paired-100.txt`, `step2-after-paired-100.txt`,
the 60 px pair, the repeated 100 px pair `step2-*-rerun-100.txt` and the per-mode costs
`step2-*-paired-modes.txt`).

| Case | Gesture | Stalls | Lag p95 / max px | Worker ms (turn p95) | Rasterized Mpx | New pixels MiB |
| --- | --- | --- | --- | --- | --- | --- |
| Text 100 MB, 1x | PageDown × 8, 120 ms apart | 6 → 0 | 576 / 576 → 0 / 0 | 68 (5.3) → 80 (11.1) | 9.1 → 11.4 | 35 → 44 |
| Text 100 MB, 2x | PageDown × 8 | 6 → 0 | 1152 / 1152 → 0 / 0 | 124 (6.1) → 136 (11.8) | 33.6 → 32.2 | 128 → 120 |
| Source (Rust), 1x | PageDown × 8 | 6 → 0 | 568 / 568 → 0 / 0 | 74 (6.0) → 92 (13.3) | 9.1 → 11.4 | 35 → 44 |
| Source (Rust), 2x | PageDown × 8 | 6 → 0 | 1037 / 1037 → 0 / 0 | 125 (5.1) → 162 (14.4) | 33.6 → 32.2 | 128 → 118 |
| 2400-byte rows wrapped, 1x | 12 notches back up | 3 → 0 | 54 / 121 → 0 / 0 | 213 (11.7) → 205 (10.7) | 6.0 → 5.0 | 19 → 19 |
| 2400-byte rows wrapped, 2x | 12 notches back up | 11 → 0 | 620 / 754 → 0 / 0 | 259 (13.6) → 290 (9.3) | 20.8 → 23.5 | 62 → 90 |
| Text 100 MB, 1x | 12 notches back up | 0 → 0 | | 25 (1.6) → 56 (4.3) | 3.0 → 5.4 | 10 → 20 |
| Text 100 MB, 2x | 12 notches back up | 0 → 0 | | 47 (3.7) → 113 (7.4) | 10.8 → 22.2 | 41 → 79 |
| 2400-byte rows wrapped, 1x | scrollbar drag 0.30 → 0.36 | 0 → 0 | | 438 (59.5) → 553 (56.7) | 13.1 → 20.5 | 50 → 74 |
| 2400-byte rows wrapped, 2x | scrollbar drag | 0 → 0 | | 595 (9.9) → 844 (73.3) | 50.4 → 71.9 | 192 → 264 |
| 2400-byte rows, 1x | 40 notches, 25 ms apart | 139 → 133 | 2384 / 2553 → 2365 / 2479 | 1252 (70.9) → 1252 (63.1) | 28.5 → 30.1 | 109 → 115 |

At the 60 px notch, the gestures before the drag leave the Source view in the state where
item 1's defect shows. There a drag of the Rust file rasterized 8.4 → 3.4 Mpx at 1x and
194.1 → 12.8 Mpx at 2x (uploads 636 → 49 MiB). PageDown stalled 3 → 0 frames at both scales
in both files.

The change costs work in two places:
- **A read-ahead page leans along the travel.** After eight PageDowns, 12 notches back up
  paint about twice the pixels (Text 100 MB: 3.0 → 5.4 Mpx at 1x, 10.8 → 22.2 at 2x), still
  without a stall. A PageDown's worker turn p95 grows from about 5 to 11–14 ms: the page read
  at rest is laid out then.
- **The drag over wrapped 2,400-byte rows rasterizes more.** It is 13.1 → 20.5 Mpx at 1x and
  50.4 → 71.9 at 2x, with 4 → 3 page layouts and 64 → 48 reads, and it settles in fewer
  frames (88 → 80, 100 → 96). The repeated pair gave the same. This is not explained yet. It
  is a cost, not a stall.

Per mode (`measure_prepared_modes`), layouts, rasters and glide steps are the same before and
after; their times differ within the noise of single runs.

## Commands and results

All builds ran in a shared build slot, with `CARGO_TARGET_DIR=C:\fmb\txt-target` and
`CARGO_INCREMENTAL=0`; the test executables ran outside it with `--test-threads 4`.

- `cargo test --no-run -p midnight-viewer-engine -p midnight-viewer-ui -p midnight-ui-input
  -p midnight-compare-ui --lib --tests --locked --offline -j 4`:
  - `midnight-viewer-engine`: 589 passed, 17 ignored (measurements)
  - `midnight-viewer-ui`: 226 + 35
  - `midnight-ui-input`: 53 + 8
  - `midnight-compare-ui`: 34 + 19
- The same for `midnight-viewer-ui` with `--features acceptance-seams`: 232 (the 226 and six
  seam tests) + 35.
- `cargo clippy -p midnight-ui-input -p midnight-viewer-engine -p midnight-viewer-ui
  -p midnight-compare-ui -p midnight-ui -p midnight-explorer --all-targets --locked
  --offline -j 4 -- -D warnings`: clean.
- The same for `-p midnight-viewer-ui --features acceptance-seams`: clean.
- `cargo fmt --all -- --check`: clean.
- Red runs:
  - the drag and PageDown tests on the before build (5 strips again; 3 stalled frames);
  - the font test on a build whose typography key is a constant (fails as quoted above);
  - the reduced-motion tests before the change;
  - `a_retained_pan_past_a_tall_table_continues_at_the_page_boundary` with Markdown pages
    prefetched at rest (no seam request).
- Release measurements: `cargo test --release --no-run -p midnight-viewer-engine --lib
  --target-dir C:\fmb\step2-rel`, then each executable with
  `measure_prepared_text_scrolling --ignored --nocapture --test-threads 1` (`PREPARED_NOTCH`
  60 and 100) and `measure_prepared_modes`.
- Not run:
  - the `midnight-ui` test suites (this change adds one function there);
  - the application window and the acceptance suite;
  - Linux and macOS.

## Open work

- Unwrapped long rows (item 3): incremental layout of row groups, after its interface review.
- The extra rasterization of the drag over wrapped long rows (above).
- The Source view still paints strips again when colours arrive after a page.
- Not run: the application window, a physical wheel or touchpad, Windows' animation-effects
  setting, Linux and macOS, and a quiet machine.
