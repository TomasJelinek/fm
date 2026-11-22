# PDF scrolling, sharpness, typed zoom and remembered layout — 21 November 2026

The user reported "PDF scrolling unusable slow" (a three-page, text-heavy Letter PDF in
Continuous layout at Fit width, 206.7%, a window about 2000 logical pixels wide on a
high-DPI display) and jagged text at 305.2% in Continuous layout. They also asked for a
typed zoom value and for the Paged/Continuous choice to be remembered. The user's
documents were not used: every measurement uses synthetic PDFs.

Results come from an isolated export of commit `2b901122` plus this change's files, built
in its own target directory, on the shared Windows 11 development machine while other
agents' builds were running (individual runs, not confidence intervals).

## Fixtures and probes

- `plugins/midnight-viewer-pdf/examples/render_probe.rs` generates two Letter reports
  (3 and 300 pages): a heading, three paragraphs of 10.5-point Times text, two bullet
  lists per section, an 8 × 4 table with 0.5-point borders and a 64 × 64 RGB logo on
  every page, with the standard fonts the plugin embeds. It times in-process rendering
  (no sandbox, pipe or host) and `--write DIR` stores the files.
- `crates/midnight-viewer-engine/src/pdf_scroll_probe_tests.rs` (ignored test; without
  `MIDNIGHT_PDF_SCROLL_FIXTURES` it returns at once, as in CI's pass over the ignored
  tests) opens a fixture through the real sandboxed plugin in a protected private
  installation (`MIDNIGHT_PDF_VIEWER_TEST_APPLICATION`), switches to Continuous, sets
  the zoom, and
  paces input at 60 Hz for 1.5 s: a wheel flick of 1,200 logical px/s, then a thumb drag
  from 10% to 60% of the document. Former whole-view frames are counted as they arrive;
  with retained tiles the probe plays the UI's tiled camera, which moves to the wanted
  position on a display frame only when its tiles have arrived. Views: 2000 × 1300
  logical pixels at device scale 1 and 2 (4000 × 2600 physical), zoom Fit, 100%, 200%
  and 400%. With retained tiles the probe then takes one zoom step (× 1.25 around the
  view's centre) and measures the time until the new zoom's visible tiles have arrived
  (`zoom_step_ms`); the UI shows the retained tiles scaled meanwhile. The probe of the
  former pipeline had no zoom step.
- `scratchpad run-probe.ps1` (not in the repository) samples the probe's and the PDF
  workers' working set and CPU time every 100 ms.

```powershell
render_probe --write _build\agent-pdf\fixtures
$env:MIDNIGHT_PDF_VIEWER_TEST_APPLICATION = '<private installation>\midnight-explorer.exe'
$env:MIDNIGHT_PDF_SCROLL_FIXTURES = '_build\agent-pdf\fixtures'
midnight_viewer_engine-<hash>.exe --ignored --exact tests::pdf_scroll_probe::pdf_scroll_probe --nocapture
```

"Dev" is the workspace `dev` profile with the PDF plugin's own dev profile (dependencies
at opt-level 3, the plugin at 2). "Release-like" is `--release` with
`CARGO_PROFILE_RELEASE_LTO=off` and 16 codegen units (the probe binary only) and the
release PDF package (fat LTO).

## Root causes

1. **Starved publication.** The engine rendered the whole view for each camera
   revision and abandoned or dropped any render whose revision had been superseded
   (`ImageView::complete_document`, the `wanted` revision in `document_render::compose`).
   The UI sends a wheel step or a thumb position every 16 ms, longer than one render, so
   during a gesture no frame was ever published: the picture froze until input stopped.
   Before, in the 16 scenarios, for both the wheel and the drag: **0 frames during
   1.5 s of input** (1 frame in the two wheel gestures that reached the end of the
   three-page document at once), the picture still for 1.42 s, the first new frame
   26–98 ms (release-like) or 120–340 ms (dev) after the input stopped.
2. **Every step rendered everything again.** Without starvation, each step still
   rendered every visible page slice, piped the RGBA through the sandbox, composed a
   full-view raster and uploaded it as a new texture. In-process whole-view renders took
   12–24 ms (dev plugin) per step, plus transport, composition and upload.
3. **Rendered below the screen's resolution.** Views over 4 megapixels were rendered
   at a reduced density and enlarged on screen: the 4000 × 2600 view was rendered at
   2540 × 1651 and stretched 1.57 times, which made text soft and uneven ("jagged").
   Hayro's own text is antialiased and hinted: a new plugin test finds grey edge pixels
   on every glyph at 1×, 2.5× and 4×.
4. **Caches thrown away per render.** The plugin created a new Hayro `RenderCache` for
   every request, re-parsing fonts and rasterizing glyphs again for each band of a page.

## Render cost per page (in-process, dev plugin)

`render_probe` built from the export (the final plugin sources), one run at 21:20 while
other agents' builds ran:

| Zoom | Page pixels | Whole page | Bands of 1024 rows, fresh caches | Same, shared caches | Quarter scale |
| --- | --- | --- | --- | --- | --- |
| 100% | 612 × 792 | 6.4–9.9 ms | | | |
| 164% | 1005 × 1300 | 11.9–15.7 ms | 16.4 ms | 13.4 ms | 7.3 ms |
| 200% | 1224 × 1584 | 13.8–17.9 ms | | | |
| 328% | 2010 × 2601 | 31.1–39.0 ms | 37.0 ms | 30.1 ms | 9.4 ms |
| 400% | 2448 × 3168 | 33.8–49.1 ms | 51.7 ms | 42.1 ms | 9.7 ms |

Retaining the caches of the two pages rendered last saves 18–19% of a page rendered in
bands (19–25% in an earlier run on a busier machine). A page in bands costs about as
much as the whole page at once; a quarter-scale page costs 7–10 ms, which is what a
low-resolution placeholder would add per page (not implemented).

## Before (HEAD 2b901122)

Release-like engine, release plugin, sampled run: frames during input 0 of 90 display
frames in 30 of 32 gestures (1 in the two that reached the end of the three-page
document at once); the probe process peaked at 56.9 MiB working set and used
23.1 s CPU, the 16 PDF workers peaked at 31.1 MiB and used 21.5 s CPU in total, rendering
views that were then discarded.

| File | Device | Zoom | Raster | Gesture | Frames during input | Picture still | First frame after input |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 3 pages | 1 | Fit 164.1% | 2000 × 1300 | wheel / drag | 0 / 0 | 1424 / 1424 ms | 48 / 41 ms |
| 3 pages | 2 | Fit 328.3% | 2540 × 1651 | wheel / drag | 0 / 0 | 1424 / 1425 ms | 58 / 49 ms |
| 300 pages | 1 | 200% | 2000 × 1300 | wheel / drag | 0 / 0 | 1426 / 1424 ms | 46 / 55 ms |
| 300 pages | 2 | 400% | 2540 × 1651 | wheel / drag | 0 / 0 | 1423 / 1424 ms | 78 / 96 ms |

The dev run shows the same zero frames, with 120–340 ms until the first frame after the
input.

## After (HEAD 2b901122 plus this change's files)

Dev engine (the export's test binary), dev PDF package built from the export, one sampled
run at 22:00 while two or three other cargo builds ran on the machine (20 logical
processors).

**Wheel**, all 16 scenarios: the view showed the wanted position on 90 of 90 display
frames, never stood still and was at its final position 0–2 ms after the input stopped.

**Scrollbar drag** from 10% to 60% of the document:

- Three-page document, all 8 scenarios: 90 of 90, never still; its whole column of tiles
  is retained.
- 300-page document: each display frame's target lies half a view to four views beyond
  the previous one, so no frame shows the exact target (0 of 90). The engine finishes the
  camera in progress and publishes it before moving on, so the view shows 4–13 of the
  pages the drag passes instead of none. The final position appears 177–257 ms after the
  input stops at device scale 1 up to 200%, 609 ms at 400%, and 0.95–1.62 s at device
  scale 2, where 4000 × 2600 physical pixels are rendered at exact resolution (the former
  pipeline rendered 2540 × 1651 and enlarged it, in 0.12–0.34 s).

| File | Device | Zoom | Raster | Gesture | Frames at the wanted position | Longest still | Final position after input | Engine frames | Page renders |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 3 pages | 1 | Fit 164.1% | 2000 × 1300 | wheel / drag | 90 / 90 | 0 / 0 ms | 0 / 0 ms | 90 / 90 | 6 |
| 3 pages | 2 | Fit 328.3% | 4000 × 2600 | wheel / drag | 90 / 90 | 0 / 0 ms | 1 / 0 ms | 88 / 63 | 6 |
| 300 pages | 1 | 200% | 2000 × 1300 | wheel / drag | 90 / 0 | 0 / 1697 ms | 1 / 257 ms | 90 / 12 | 37 |
| 300 pages | 2 | 400% | 4000 × 2600 | wheel / drag | 90 / 0 | 0 / 2785 ms | 1 / 1345 ms | 85 / 4 | 18 |

**Zoom step** (× 1.25, not measured before): the new zoom's exact tiles arrived after
99–194 ms at device scale 1 (335 ms at 400% in the 300-page document) and 395–812 ms at
device scale 2. The UI shows the retained tiles scaled to the new zoom at once.

**Page renders** (plugin page-slice requests during both gestures): 0–10 for the
three-page document, whose pages are rendered once and kept, and 18–66 for the 300-page
one, whose drag passes about 150 pages.

**Memory and CPU** (the whole run, 16 scenarios): the probe process (engine and tile
caches) peaked at 240.6 MiB working set and used 44.5 s CPU; the 16 PDF workers peaked at
46.0 MiB (Hayro caches of two pages retained) and used 7.2 s CPU in total. A 4000 × 2600
view at exact resolution has a 121.5 MiB tile budget in the backend (and as much in the
UI), and everything alive stays within twice that; the former pipeline peaked at
56.9 MiB with reduced density and no retention.

**Release-like** (the release-like engine test binary and the release PDF package of the
export, one sampled run at 22:43): the wheel again showed the wanted position on 90 of 90
display frames in all 16 scenarios (final position 0–7 ms after the input), and the drag
in the three-page document on 90 of 90. In the 300-page document the drag showed 6–18 of
the pages it passes and the final position 116–194 ms after the input at device scale 1
and 472–596 ms at device scale 2; the former pipeline showed nothing during the drag and
its final, smaller rendering 26–98 ms after the input. A zoom step's exact tiles arrived
after 69–176 ms at device scale 1 and 217–398 ms at device scale 2. The probe process
peaked at 231.5 MiB and used 20.1 s CPU (before: 56.9 MiB, 23.1 s); the PDF workers
peaked at 46.2 MiB and used 10.6 s CPU in total (before: 31.1 MiB, 21.5 s).

| File | Device | Zoom | Raster | Gesture | Frames at the wanted position | Longest still | Final position after input | Engine frames | Page renders |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 3 pages | 1 | Fit 164.1% | 2000 × 1300 | wheel / drag | 90 / 90 | 0 / 0 ms | 0 / 3 ms | 90 / 90 | 6 |
| 3 pages | 2 | Fit 328.3% | 4000 × 2600 | wheel / drag | 90 / 90 | 0 / 0 ms | 1 / 0 ms | 90 / 80 | 6 |
| 300 pages | 1 | 200% | 2000 × 1300 | wheel / drag | 90 / 0 | 0 / 1602 ms | 2 / 162 ms | 90 / 17 | 50 |
| 300 pages | 2 | 400% | 4000 × 2600 | wheel / drag | 90 / 0 | 0 / 1968 ms | 3 / 528 ms | 90 / 7 | 34 |

**Trade-off.** After a fast drag across a long document the final view now takes longer
than before (162 ms instead of 55 ms, 528 ms instead of 96 ms in the rows above): it has
2.5 times the pixels at device scale 2, and the engine first finishes the camera in
progress, which is what shows the passed pages during the drag. Not implemented: while a
newer camera waits, rendering only the visible tiles of the camera in progress (no guard
rows), and a release signal from the scrollbar that lets the engine skip to the target.

The text viewer's path is not part of this change (its own agent owns it); text
measurements are in that agent's report.

## Tests

From the export (HEAD `2b901122` plus this change's files), dev profile, each test binary
run directly with `--test-threads=4`:

| Binary | Result |
| --- | --- |
| `midnight-core` lib / integration | 124 passed / 159 passed, 2 ignored |
| `midnight-platform-common` lib / integration | 162 passed / 175 passed |
| `midnight-viewer-engine` lib | 580 passed, 15 ignored (the probe among them) |
| `midnight-viewer-ui` lib / integration | 219 passed / 34 passed |
| `midnight-viewer-pdf` (own workspace) `tests/render.rs` | 9 passed |

`cargo clippy -p midnight-core -p midnight-platform-common -p midnight-viewer-engine
-p midnight-viewer-ui -p midnight-ui -p midnight-explorer --all-targets -- -D warnings`
and the PDF workspace's `clippy --all-targets -- -D warnings` pass.

**Red runs.** Each decision was reverted in a copy of the export and the tests written for
it were run against that build:

| Reverted | Failing tests |
| --- | --- |
| Exact resolution (views over 4 Mpx rendered smaller, as before) | `documents_render_at_exact_physical_resolution_and_images_keep_bounded_geometry` (density 0.444 instead of 0.625), `bands_of_large_dpi_views_stay_within_the_provider_viewport_bounds` (2730 × 1536 instead of 7680 × 4320) |
| Tile retention beyond the current plan | `scrolling_down_and_back_renders_each_tile_once_and_each_page_in_few_bands` (tiles rendered up to 11 times), `repeated_pan_reuses_pixels_and_guards_only_request_the_new_frontier` |
| A camera waits for the one in progress (fast drags) | `a_fast_drag_publishes_the_cameras_it_passes` (0 cameras shown on the way) |
| A pan of a validated surface publishes at once | `a_ready_camera_of_a_validated_surface_publishes_at_once` |
| Bands at most 16 tiles down | `bands_of_narrow_pages_stay_within_the_provider_edge` ("Document viewport exceeds its raster limit.") |
| Plugin caches kept for two pages | `page_bands_join_without_seams_and_reuse_bounded_caches` |
| The layout choice is saved | `layout_selection_waits_for_publication_and_retries_a_fresh_frame_generation` |
| Typed zoom: clamping, Escape, the user's decimal separator | `values_outside_the_range_are_clamped_and_reported`, `values_outside_the_range_clamp_with_a_hint_and_text_is_never_silently_ignored`, `escape_or_leaving_the_field_keeps_the_zoom_and_the_viewer_open`, `the_users_decimal_separator_shows_and_reads_the_zoom` |
| Settings schema 36 | the three `document_layout_tests` that write, export and refuse schema 36 |

In the same red run the office test
`office_real_generated_links_resolve_from_bootstrap_catalog_and_nested_sheets` also
failed ("Source changed since capture": the temporary file's change time moved); it
passed in the green run and none of the reverted code is on its path.