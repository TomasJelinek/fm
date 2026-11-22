# Prepared text window and faster wheel: 21 November 2026

The user reported that the text viewers had good inertia but should scroll faster, and
that scrolling was very choppy. They asked for prepared text to be kept several pages
ahead and behind, so that it is not computed all the time, and allowed some memory for
it. This record covers the Text, Source, Markdown and hex views (`midnight-viewer-engine`,
`midnight-viewer-ui`, `midnight-ui-input`, `midnight-core`). The HTML viewer follows the
same notch distance.

The results come from an isolated export of commit `2b901122` plus this change's files,
built in its own target directories on the shared Windows 11 development machine while
other agents' builds were running. Each figure is a single run, not a confidence
interval. The tests and checks were then repeated on `007c29c8` plus this change
([Tests](#tests)).

## Causes

Measured before the change (release, co-simulation below, 60 px notch):

1. **Constant refills.** A retained text frame was one snapshot of the window and its
   256-pixel guards, and the page held the same rows. Once the camera passed the middle
   of its page, every camera request started the next refill: a plugin read, a layout
   of the joined page and a repaint of the whole snapshot, one after another while the
   content moved. A 40-notch wheel spin over the 100 MB file at 1x rasterized 50 Mpx in
   12 reads and kept the worker busy for 664 ms of the 1.2 s gesture (908 ms at 2x).
2. **Stalls at the guards.** A glide or a flick moves 1,000 to 4,000 px/s, and 256 px of
   guard lasted a few frames. The camera then waited at the edge of its pixels until
   the next snapshot arrived: 4 to 23 frames per gesture at 1x, and up to 43 at 2x, in
   the 100 MB file and the Rust file.
3. **A new texture per refill.** Each snapshot became a new image of the window and its
   guards: 116 to 317 MiB of new pixels per gesture at 1x, and up to 745 MiB at 2x.
4. **Re-uploaded strip textures (found by the native runs).** The first build of the
   prepared window replaced the window's whole strip model for each new strip list.
   Slint's repeater then re-creates every row (`Repeater::model`), and the FemtoVG
   renderer has no shared cache key for pixel-buffer images (`ImageCacheKey::Invalid`),
   so every shown strip's texture was uploaded again several times a second. Frames
   that brought a new strip still took 21 to 25 ms (p50) at 2x.

## Design

The [prepared text window](../developer/SCROLLING.md#prepared-text-window) keeps the
text prepared several screens around the window as world-aligned strips of 256 rows,
each painted once and then reused:

- `worker_bands.rs` keeps strips one screen against the direction of travel and two
  along it (three while a camera request moves more than a sixteenth of a screen). It
  keeps them a further screen in both directions before dropping them. A frame
  publishes the contiguous run around the camera (`ViewerRasterBands`). The strips
  under the window and one beyond each edge are painted before the frame; the others
  are painted between inputs in 4 ms slices, nearest first along the travel.
- A page holds the prepared screens and two more along the travel. A refill starts
  while the page holds fewer screens ahead than the strips want, so one comes about
  every two screens. A refill, restyle or selection keeps every strip whose content
  hash it reproduces.
- The strips of one viewer hold at most 64 MiB, and those of all viewers 256 MiB
  (`VIEWER_PREPARED_TEXT_BYTES`, `VIEWER_PREPARED_TEXT_TOTAL_BYTES`, `raster_budget.rs`).
  When the budget is full, strips beyond the window go first.
- The window turns each strip into one image when it first arrives. The image keeps its
  Slint row while the strip is shown: rows are only appended and removed, at heights
  below a base that moves at most every 32 screens, and `raster-bands-origin` places
  that base. Moving over prepared strips therefore creates no image and uploads no
  texture. A camera at rest stands on a whole pixel, where strips are exact.
- A wheel notch moves `LINE_PIXELS` = 100 logical px (was Slint's 60) at Windows'
  default of three lines per notch, in proportion to the system setting
  (`notch_pixels_for`; `SPI_GETWHEELSCROLLLINES` is read by the viewer with its first
  native event and on focus). The glide (200 ms ease-out) and the momentum decay
  (`TIME_CONSTANT` 0.8 s) are unchanged, so a notch travels 5/3 as far on the same curve.

## Co-simulation

`worker_prepared_tests.rs` (ignored `measure_prepared_text_scrolling` and
`measure_prepared_modes`) runs the real worker, a simulated basic or Source plugin
that answers a frame later (Source replies carry keyword, string, number and comment
colours) and the window's retained camera at 60 Hz. Every frame, the window moves its
camera within the pixels it has. The worker takes a turn only when its previous one has
finished. A turn's measured cost (the worker's own wall time for inputs, plugin events,
layout, rasterization and preparation, not the simulations) keeps it busy for as many
frames before its outputs arrive, through a mailbox that keeps the newest frame. Wheel
notches glide as `WheelMotion` does; keys move their whole distance; the scrollbar asks
for each position. A frame stalls when the camera has travel left and cannot move. "New
pixels" counts pixel bytes the window had not received before: a whole new snapshot,
or new strips only. The window is 1312 × 720 at 1x and 2624 × 1440 at 2x.

Before is HEAD `2b901122` with only the harness and counters added
(`_build/agent-txt-before`); after is this change. Both ran in release, with
`--ignored --nocapture --test-threads 1`:

```powershell
$env:PREPARED_NOTCH = '60'   # or 100
midnight_viewer_engine-<hash>.exe measure_prepared_text_scrolling --ignored --nocapture --test-threads 1
midnight_viewer_engine-<hash>.exe measure_prepared_modes --ignored --nocapture --test-threads 1
```

### Same notch (60 px), before → after

The table lists stalled frames, how far the shown camera trailed the input (p95 / max),
the worker's time for the gesture (the p95 of one turn), the pixels rasterized and
received, plugin reads, and the most bytes the prepared strips held at once.

| Case | Gesture | Stalls | Lag p95 / max px | Worker ms (turn p95) | Rasterized Mpx | New pixels MiB | Reads | Strips MiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Text 100 MB, 1x | 12 notches, 50 ms apart | 4 → 0 | 27 / 42 → 0 / 0 | 490 (28.1) → 18 (3.6) | 34.9 → 1.0 | 133 → 4 | 3 → 0 | 16.7 |
| Text 100 MB, 1x | 40 notches, 25 ms apart | 13 → 0 | 104 / 150 → 0 / 0 | 664 (25.2) → 29 (3.7) | 50.4 → 1.7 | 192 → 6 | 12 → 0 | 23.1 |
| Text 100 MB, 1x | touchpad flick | 5 → 3 | 18 / 77 → 0 / 66 | 1048 (25.2) → 58 (1.4) | 75.8 → 3.7 | 289 → 14 | 3 → 2 | 19.2 |
| Text 100 MB, 1x | PageDown × 8, 120 ms apart | 23 → 3 | 689 / 689 → 571 / 571 | 328 (23.9) → 69 (8.2) | 30.4 → 6.7 | 116 → 26 | 16 → 2 | 25.6 |
| Text 100 MB, 1x | 12 notches back up | 4 → 0 | 31 / 50 → 0 / 0 | 538 (34.3) → 6 (0.2) | 34.8 → 0.0 | 133 → 0 | 4 → 0 | 21.8 |
| Text 100 MB, 1x | scrollbar drag 0.30 → 0.36 | 0 → 0 | | 584 (19.2) → 508 (20.7) | 74.6 → 66.5 | 285 → 197 | 58 → 44 | 12.8 |
| Text 100 MB, 2x | 12 notches, 50 ms apart | 7 → 0 | 116 / 157 → 0 / 0 | 624 (47.9) → 30 (3.9) | 89.4 → 4.0 | 341 → 15 | 4 → 0 | 61.5 |
| Text 100 MB, 2x | 40 notches, 25 ms apart | 31 → 0 | 366 / 462 → 0 / 0 | 908 (56.7) → 58 (4.5) | 127.8 → 6.7 | 488 → 26 | 9 → 0 | 61.5 |
| Text 100 MB, 2x | touchpad flick | 14 → 3 | 86 / 207 → 0 / 135 | 1389 (64.8) → 72 (4.5) | 178.2 → 11.4 | 680 → 44 | 5 → 2 | 61.5 |
| Text 100 MB, 2x | PageDown × 8 | 42 → 3 | 2102 / 2330 → 1108 / 1108 | 594 (45.5) → 115 (5.9) | 125.7 → 25.5 | 480 → 97 | 19 → 2 | 61.5 |
| Text 100 MB, 2x | scrollbar drag | 0 → 0 | | 810 (52.6) → 715 (38.3) | 142.1 → 139.7 | 542 → 446 | 28 → 36 | 48.7 |
| Source (Rust), 1x | 40 notches, 25 ms apart | 12 → 0 | 96 / 109 → 0 / 0 | 684 (22.9) → 28 (3.8) | 54.8 → 1.7 | 209 → 6 | 12 → 0 | 23.1 |
| Source (Rust), 1x | touchpad flick | 4 → 3 | 6 / 46 → 0 / 59 | 1074 (22.4) → 54 (0.2) | 83.1 → 3.4 | 317 → 13 | 4 → 2 | 17.9 |
| Source (Rust), 1x | PageDown × 8 | 17 → 3 | 714 / 714 → 546 / 546 | 272 (15.8) → 79 (7.6) | 30.4 → 7.0 | 116 → 27 | 16 → 2 | 25.6 |
| Source (Rust), 1x | scrollbar drag | 0 → 0 | | 23 (0.0) → 109 (11.6) | 1.6 → 10.8 | 6 → 37 | 60 → 58 | 12.8 |
| Source (Rust), 2x | 40 notches, 25 ms apart | 31 → 0 | 322 / 398 → 0 / 0 | 903 (56.7) → 69 (5.9) | 123.3 → 6.7 | 470 → 26 | 9 → 0 | 61.5 |
| Source (Rust), 2x | touchpad flick | 15 → 3 | 99 / 213 → 0 / 121 | 1335 (52.6) → 78 (5.2) | 195.3 → 11.4 | 745 → 44 | 4 → 2 | 61.5 |
| Source (Rust), 2x | PageDown × 8 | 43 → 3 | 2330 / 2558 → 1059 / 1059 | 597 (42.5) → 137 (6.5) | 125.7 → 25.5 | 480 → 97 | 19 → 2 | 61.5 |
| Source (Rust), 2x | scrollbar drag | 0 → 0 | | 57 (0.1) → 727 (36.1) | 6.2 → 172.6 | 24 → 559 | 60 → 40 | 48.7 |
| 2400-byte rows wrapped, 1x | 40 notches, 25 ms apart | 3 → 0 | 27 / 67 → 0 / 0 | 1091 (71.5) → 151 (9.8) | 33.9 → 3.0 | 130 → 12 | 0 → 0 | 24.3 |
| 2400-byte rows wrapped, 1x | PageDown × 8 | 17 → 0 | 464 / 464 → 0 / 0 | 390 (29.8) → 284 (12.1) | 25.9 → 8.7 | 99 → 33 | 0 → 0 | 34.6 |
| 2400-byte rows wrapped, 2x | 40 notches, 25 ms apart | 33 → 0 | 315 / 395 → 0 / 0 | 1186 (95.5) → 276 (13.3) | 88.0 → 12.8 | 336 → 49 | 0 → 0 | 61.5 |
| 2400-byte rows wrapped, 2x | PageDown × 8 | 50 → 0 | 2558 / 2558 → 0 / 0 | 1002 (68.2) → 478 (12.7) | 119.5 → 30.9 | 456 → 118 | 0 → 0 | 61.5 |
| 2400-byte rows, 1x | 40 notches, 25 ms apart | 93 → 88 | 992 / 1101 → 923 / 978 | 892 (63.0) → 767 (62.6) | 18.4 → 19.3 | 70 → 74 | 52 → 51 | 7.7 |
| 2400-byte rows, 1x | touchpad flick | 38 → 36 | 206 / 275 → 109 / 139 | 1396 (92.8) → 562 (53.7) | 29.1 → 13.2 | 111 → 50 | 19 → 20 | 7.7 |
| 2400-byte rows, 2x | 40 notches, 25 ms apart | 113 → 114 | 2481 / 2745 → 2476 / 2666 | 1344 (85.8) → 1268 (94.2) | 88.7 → 65.8 | 338 → 248 | 51 → 51 | 27.6 |

The scrollbar drag has no lag: the window waits for each position. The full output,
including the gestures left out here, is in `_build/agent-txt/measure/`
(`before-release-final-60.txt`, `after-release-final-60.txt`, `after-release-final-100.txt`).

### New notch (100 px), after only

With the faster notch the same scripts travel 5/3 as far. The 40-notch spin then moves
about five screens a second and stalls 3 frames at 1x in the 100 MB file (lag max
222 px), 4 at 2x (534 px), and 3 in the Rust file at 1x and 2x. All other wheel
gestures and the flick stall none, except the unwrapped 2400-byte rows (154 of 177
frames for the spin at 1x). PageDown × 8 stalls 6 frames at both scales, against 23 and
42 before at 60 px.

### Per mode, without a plugin (`measure_prepared_modes`)

A page of several screens is laid out, the first frame painted, then the rest of the
prepared window, then a 30-step glide in 24-pixel camera steps:

| Mode, scale | First frame ms | Prepared window ms (Mpx) | Glide step p95 ms | Glide rasterized Mpx |
| --- | --- | --- | --- | --- |
| Text, 1x | 17.1 → 20.8 | → 21.4 (1.7) | 9.5 → 4.0 | 4.8 → 1.0 |
| Source, 1x | 16.6 → 20.0 | → 19.6 (1.7) | 8.3 → 4.1 | 4.8 → 1.0 |
| Markdown, 1x | 13.9 → 12.6 | → 11.8 (1.7) | 5.7 → 2.3 | 4.8 → 1.0 |
| Hex, 1x | 25.8 → 34.7 | → 31.7 (1.7) | 15.0 → 6.2 | 4.8 → 1.0 |
| Text, 2x | 33.2 → 33.6 | → 56.1 (6.7) | 23.9 → 4.1 | 12.3 → 2.0 |
| Source, 2x | 27.0 → 27.1 | → 46.7 (6.7) | 17.4 → 5.6 | 12.3 → 2.0 |
| Markdown, 2x | 22.3 → 20.4 | → 26.8 (6.7) | 14.7 → 3.1 | 12.3 → 2.0 |
| Hex, 2x | 48.8 → 43.2 | → 64.0 (6.7) | 25.4 → 5.7 | 12.3 → 2.0 |

The prepared window is painted after the first frame, in 4 ms slices between inputs; a
glide step's p95 is such a slice.

## Native wheel runs

`performance_probe --wheel --sustained --d3d --file=<fixture>` (the probe in
`crates/midnight-viewer-ui/examples`) opens one file in a real viewer window, 1000 × 700
logical pixels (`SLINT_SCALE_FACTOR` 1 or 2), with FemtoVG on DX12 and the release basic
and source viewers. It posts 96 wheel notches (`WM_MOUSEWHEEL`, ±120) to its own window
every 50 ms (`MIDNIGHT_VIEWER_PROBE_WHEEL_MS=50`): 48 down, then 48 up. The desktop
cursor does not move. For every redraw that shows a new position, the probe records
when the redraw finished on the UI thread (present included, not scan-out) and how
many images the window had created. Fixtures: `plain.txt` (1.7 million rows of 60
bytes, 102 MB) in the Text view and `source.rs` (15,000 generated Rust functions,
11.8 MB) in the Source view. The panel is 3840 × 2400 at 60 Hz on a laptop with Intel
Iris Xe and NVIDIA RTX 3500 Ada graphics; the adapter wgpu chose was not recorded.

The plugin host accepts a runtime only when its executable and every folder above it
can be written by the current user, SYSTEM, Administrators or TrustedInstaller alone
(`midnight-plugin-host` `trust_windows.rs`). Every folder of the worktree, and `D:\`
itself, grant Authenticated Users Modify, and the session's temporary folder grants
another group Modify, so a runtime there cannot load. The probes and the viewers were
therefore copied to a new folder under `%LOCALAPPDATA%`, which inherits only the user,
SYSTEM and Administrators. They ran from there with
`MIDNIGHT_VIEWER_PROBE_EXPLICIT_RUNTIME=1`, and the folder was deleted afterwards. No ACL
was changed and nothing was registered.

"Late" counts gaps over 25 ms between two frames of the same notch. Image frames are
gaps across a new strip (after) or a new snapshot (before); other frames are the rest,
from the probe's `wheel_continuous` row. Before moves 60 px per notch, after 100 px.

| Case | Build | Late | Image frames p50 / p95 / max ms | Other frames p50 / p95 / max ms | Images |
| --- | --- | --- | --- | --- | --- |
| Text, 1x | before | 54 | 25.4 / 27.3 / 29.1 | 15.9 / 16.8 / 17.8 | 88 |
| Text, 1x | strips, model replaced | 6 | 20.2 / 31.6 / 33.9 | 16.5 / 17.7 / 38.5 | 57 |
| Text, 1x | strips, rows kept | 6 | 16.6 / 29.3 / 29.8 | 16.6 / 18.7 / 29.7 | 53 |
| Text, 2x | before | 68 | 30.8 / 35.2 / 65.9 | 3.4 / 17.4 / 20.0 | 116 |
| Text, 2x | strips, model replaced | 28 | 25.1 / 33.6 / 41.7 | 15.6 / 18.7 / 32.6 | 88 |
| Text, 2x | strips, rows kept | 3 | 15.4 / 29.3 / 55.4 | 16.6 / 19.2 / 30.3 | 96 |
| Source, 1x | before | 15 | 16.7 / 26.6 / 30.8 | 12.8 / 16.9 / 22.8 | 314 |
| Source, 1x | strips, model replaced | 2 | 16.7 / 24.1 / 30.7 | 16.6 / 22.2 / 24.7 | 165 |
| Source, 1x | strips, rows kept | 25 | 16.8 / 41.6 / 78.2 | 16.5 / 29.1 / 422.0 | 153 |
| Source, 2x | before | 98 | 31.1 / 34.3 / 73.9 | 2.3 / 16.9 / 37.4 | 171 |
| Source, 2x | strips, model replaced | 27 | 21.2 / 31.0 / 41.1 | 19.7 / 23.4 / 31.5 | 224 |
| Source, 2x | strips, rows kept | 4 | 16.5 / 23.9 / 47.3 | 16.5 / 19.9 / 79.4 | 235 |

Before, frames came in bursts: two redraws 2 to 3 ms apart, then a gap of a snapshot
(p50 of the other frames 2.3 to 3.4 ms at 2x). The final build's Source run at 1x is
unresolved. It had one 422 ms pause between two notches and several 35 to 73 ms
redraws in frames that created no image, which draw only about ten textured quads. CPU
use was 96%, with 9 cargo/rustc/link processes and 5.5 GB of memory free, when it was
sampled right after the runs. A paired rerun on a quiet machine is open. The Source view
created about three times as many strip images as the Text view for the same travel.
This was not investigated; the likely cause is restyles, which paint again the strips
whose colours changed when a page's colours arrive after its text.

## Tests

New tests, all headless:

- `midnight-viewer-engine`, `worker::tests::prepared` (`worker_prepared_tests.rs`, the
  timed co-simulation):
  - `scrolling_inside_the_prepared_window_prepares_nothing_again`: turning back and
    forth within the window reads, lays out and paints nothing, at 1x and 2x.
  - `a_slow_worker_never_stalls_motion_inside_the_prepared_window`: the worker takes a
    turn only every twelfth frame, and ten notches still never stall.
  - `a_refill_keeps_the_strips_of_rows_it_shows_unchanged`: over six screens in one
    gesture, the window is given a strip again only at a page's edges (at most two
    strips at each edge, four per refill), and the worker paints the new strips and
    those few.
  - `prepared_strips_stay_within_the_viewer_budget`: the 4-megapixel window at 2x.

  `raster_budget::tests` were rewritten for strips: reservation, rollback, body and
  gutter owners, the per-viewer and total budgets, concurrency.
- `midnight-viewer-ui`, `raster_strip_tests`: the camera glides over prepared strips
  on the window's own clock without new images or worker replies; new strips become
  images and shown ones keep theirs; shown strips keep their rows and the window its
  model while strips come and go.
- `midnight-ui-input`, `wheel_lines::tests`: the default notch, proportions, bounds and
  one screen per notch.
- `midnight-core`, `viewer::raster::tests`: strip lists must be contiguous whole-pixel
  runs within the budget.

Existing tests that read retained pixels (raster, position, palette, syntax, overlap
and chain tests) now read the stacked strips, and the wheel tests expect the shared
notch distance. The comparator's `scroll_tests` expect `LINE_PIXELS`.

Debug build, then each test executable from its package folder with `--test-threads 4`.
The final run used a whole-tree export of `007c29c8` plus this change (target folder of
its own, `CARGO_INCREMENTAL=0`):

```powershell
cargo clippy -p midnight-core -p midnight-ui-input -p midnight-viewer-engine -p midnight-viewer-ui -p midnight-compare-ui --all-targets --locked --offline -j 4 -- -D warnings
cargo test --no-run -p midnight-viewer-engine -p midnight-viewer-ui -p midnight-ui-input -p midnight-core -p midnight-compare-ui --lib --tests --locked --offline -j 4
cargo fmt --all -- --check
```

| Package | `2b901122` + change: passed | `007c29c8` + change: passed | Failed | Ignored |
| --- | --- | --- | --- | --- |
| `midnight-viewer-engine` (lib) | 577 | 585 | 0 | 16, 17 |
| `midnight-viewer-ui` (lib, integration) | 212, 34 | 223, 35 | 0 | 0 |
| `midnight-ui-input` (lib, integration) | 53, 8 | 53, 8 | 0 | 0 |
| `midnight-core` (lib, integration) | 124, 158 | 127, 159 | 0 | 2 |
| `midnight-compare-ui` (lib, integration) | 33, not run | 34, 19 | 0 | 0 |

Clippy is clean, and the formatting checks pass (`rustfmt --check` on every changed
file, and `cargo fmt --all -- --check` on `007c29c8` plus this change). The ignored
engine tests include the native PDF tests (`pdf_navigation_tests`,
`pdf_scroll_probe_tests`), which need a trusted PDF package
(`MIDNIGHT_PDF_VIEWER_TEST_APPLICATION`). They were not run: this change does not
touch the image and document path that they drive (`image_scene`, `image_tiles`,
`image_view`, `image_worker`, `worker_image`), and strip preparation does nothing
outside the retained text modes. PDF and image wheel notches do move the shared 100 px
now.

### Red runs

Each regression test was run against a mutation that reverts the property it guards,
compiled into a separate copy of the export (`_build/agent-txt-mut`) behind an
environment switch. That copy's test executables were kept before the export was
built again. Each test passed with its switch off and failed with it on:

| Mutation | Test | Failure |
| --- | --- | --- |
| Nothing prepared beyond the window (`prepare_strips_turn` returns at once) | `a_slow_worker_never_stalls_motion_inside_the_prepared_window` | 2 frames waited for pixels, lag max 75 px |
| No strip reused across pages (`strip_hash` is `None`) | `a_refill_keeps_the_strips_of_rows_it_shows_unchanged` | the window was given 6 strips again over 1 refill, in 10 of 10 runs (2 without) |
| Strips beyond the window dropped at once (`evict_strips`) | `scrolling_inside_the_prepared_window_prepares_nothing_again` | the view never came to rest: strips were painted and dropped in turn |
| The per-viewer budget not enforced (`reserve_strip`) | `prepared_strips_stay_within_the_viewer_budget` | 80 MiB of strips |
| The strip model replaced for every strip list (`Strips::show`) | `shown_strips_keep_their_rows_while_strips_come_and_go` | the window did not keep its model |

The first red runs found two weak tests, which were then strengthened. The slow-worker
test set a worker period that the timed frame ignored. The refill test bounded only the
strips painted, which loose reuse still met. A first mutation of the refill thresholds
never took effect, because a threshold is capped at half the page. The first copy of the
export also shared the target folder with the export, which has the same package
metadata, so their executables overwrote each other. The table comes from the second
copy's executables (built alone, after `cargo clean -p midnight-viewer-engine -p
midnight-viewer-ui --profile dev`), and the test counts above come from the export built
again afterwards. On `007c29c8` the unmutated refill test counted 3 strips again in 9
of 10 runs, against its first bound of two per refill. Which frames reach the window
depends on the worker's timing, so the test now allows the strips at both page edges,
four per refill. The mutation's 6 still exceeds that, and the test passed 10 of 10 runs
on `007c29c8`.

## Not verified

- A physical wheel or touchpad: the native probe posts wheel messages to its own window.
- The full application: the probe drives a viewer window with the release basic and
  source viewers. The Markdown and hex views were measured only in the co-simulation,
  without a plugin.
- Windows wheel settings other than three lines natively (unit tests only), Linux,
  macOS and scales other than 1x and 2x.
- A quiet machine: every timing ran while other agents' builds ran.
- `midnight-ui`: its tests were not run with this change. Panels deliver X11 touchpad
  lines themselves at 60 px, but their momentum estimate goes through
  `midnight-ui-input`, whose `LINE_PIXELS` is now 100. HEAD's
  `x11_fractional_line_stream_follows_the_finger_then_coasts` expects 60 px lines;
  the panel change of 21 November moves panels and that test to `LINE_PIXELS`.

## Open work

- Pair a rerun of Source at 1x, before and after, on a quiet machine.
- Unwrapped rows wider than the window: a page of at most 120 KiB holds too few of them
  to prepare screens ahead, so they stall as before. This needs horizontal windowing of
  long rows, or pages that are larger for them.
- A scrollbar drag inside the loaded page starts a new raster context for each
  position, so the window's strips are painted again each time: 173 Mpx in a one-second
  drag of the Rust file at 2x, against one snapshot before. Strips could be kept across
  such a seek.
- At 100 px per notch, a fast spin (about five screens a second) still stalls 3 or 4
  frames when a refill arrives late.
- The Source view paints strips again when colours arrive after a page.
