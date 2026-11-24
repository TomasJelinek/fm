# Viewer scrolling

The UI owns pointer feedback. The viewer engine owns file reads, decoding,
search, text layout and raster preparation. Dragging a scrollbar must never wait
for these operations or enqueue an unbounded history of obsolete positions.

Wheel notches, scroll keys and touchpad momentum in every viewer mode (Text, Source,
Markdown, hex, image, PDF, HTML) and in the comparison window glide for the length that
Options › Appearance › **Scrolling glide** sets, and move at once when it is Off or the
system asks for less motion; see
[Scrolling glide setting](SCROLLING.md#scrolling-glide-setting). A wheel notch travels
the share of the system's notch that **Scrolling speed** sets
([Scrolling speed setting](SCROLLING.md#scrolling-speed-setting)).

## Scrollbar input and text acknowledgements

The shared `PositionBar` retains the pointer's original grab offset, track length
and thumb extent for the duration of a drag. It paints the local target directly.
A stationary grab/release does not reload content. A changed content context,
resize or pointer cancellation ends the old gesture. An image's context is its
surface, which changes with its zoom, viewport or page. A document in the Continuous
layout keeps its context as the document, zoom and viewport: frames painted for
earlier steps of a drag across pages do not end it, and it lands where the thumb is
released (`image_camera::position_context`). Documents scroll over retained tiles; see
[Documents](#documents-pdf).

Markdown uses the same asynchronous viewport pipeline. Its independent plugin
counts semantic block lines, rather than only literal newlines, before cropping
the page. A long lookahead with many later blocks must not downgrade the visible
page to plain text. Parsing and safe-checkpoint discovery share one parser pass.
Each immutable session keeps an LRU of at most 16 semantic pages and 1 MiB (the
plugin's `page_cache.rs`), keyed by anchor, direction and viewport budgets. Revisited
resident pages require no source read or parse; closing/reopening never carries cache
across revisions. Uncached distant seeks without a trustworthy block checkpoint keep the
bounded literal fallback. There is no eager full-file Markdown index.

The GitHub preset lays out semantic blocks before rasterization. Heading rules,
code/inline-code backgrounds, quote borders and table stripes are host geometry;
they do not inject text into source anchors. Selection paints over backgrounds.
Compact/Comfortable, plain text and source retain their own presentation paths.

`TextPosition` retains at most one desired position per axis. Its axes are
**vertical, horizontal**. A rejected enqueue retains the latest target for retry;
newer accepted navigation cancels obsolete retries. File/mode changes fence the
old context only after an actual transition was accepted. Selecting the already
active mode is a no-op.

The internal `ViewerInput::Position` carries a context generation, monotonic input
sequence, axis and normalized position. `PositionQueue` replaces consecutive
still-pending absolute inputs only for the same axis and generation. Other inputs
remain ordering barriers. The consumer seals a position before doing work, so a
later pointer event receives a new delivery rather than mutating active work.

The engine keeps one active vertical scrollbar viewport and one latest deferred
target. Continuous input must not repeatedly cancel the active read: completed
intermediate pages remain useful while the pointer moves. The UI accepts forward
acknowledgement progress without clearing a newer desired thumb position. It
rejects older completed sequences and unacknowledged page replacements. Only the
latest target's acknowledgement clears that axis's pending intent. Navigation
barriers clear the deferred target along with the old context.

The backend retains native byte offsets as `u64`. An absolute vertical seek keeps
its latest normalized target through the bounded read, then maps that target into the
newly shaped page before acknowledging it. The new page's byte density can change
the visible fraction, so the first returned source anchor is not itself a success
acknowledgement. Requests include a bounded context prefix, limited by the guard,
a quarter of the current page and half its available scroll span. If the loaded
page still cannot cover the target, at most one further bounded correction read
is allowed. A newer deferred target supersedes that correction. There is no
whole-file index or recursive read loop.

A final acknowledgement requires a successfully completed request whose usable
pixels cover the mapped target. An error, cancellation or still-uncovered correction
establishes a fresh context and publishes the retained view without a success
acknowledgement; a no-match search also publishes its new context. Horizontal
intent survives a matching page arrival, but cannot override newer relative
horizontal scrolling. Old context/sequence replies cannot pull a pending thumb
backwards.

These messages are application-owned, not a change to the external viewer plugin
ABI. Plugins continue to receive bounded viewport requests through the shared API.

Behavioral reference: Salamander's viewer keeps the latest tracking position while the
thumb moves, updates the content on a 20 ms timer and applies the final position on
release to avoid a flash at the old location. This implementation uses asynchronous
backend messages and bounded coalescing instead; no Salamander code is copied.

## Retained text pixels and delivery

Text, source, Markdown and binary/hex retain prepared strips (`ViewerRasterBands`):
the body and the source gutter, 256 rows each, several screens around the window
(the [prepared text window](SCROLLING.md#prepared-text-window)). Their surface
identity names a layout/camera context; a strip's ID names its immutable pixels in the
whole process (so a new surface keeps the strips its page still paints the same way, and
the window keeps their conversions and images), and a frame's pixel identity changes
when its set of strips does. The frontend moves over
the strips without waiting for the backend and keeps the gutter horizontally fixed;
each strip becomes one image when it first arrives. Strips of one viewer hold at most
64 MiB and those of all viewers 256 MiB; the engine's budget and the UI's frame check
take these from `midnight-core` (`VIEWER_PREPARED_TEXT_BYTES`,
`VIEWER_PREPARED_TEXT_TOTAL_BYTES`). Refills keep the request ceilings: 12 KiB for
text and source, 64 KiB for Markdown and 4 KiB for binary, at most 512 lines (the viewer
engine's `worker.rs`); the prepared screens and a higher DPI do not enlarge those source
budgets. An uncovered frontier pauses displayed motion until usable pixels arrive; it is
not an inferred end of file. A camera at rest stands on a whole raster pixel, where
strips are exact.

A wheel notch moves the text, Source, Markdown and hex views, PDF and image tiles, the
comparison and an HTML page by Chromium's distance on Chromium's glide, wall-clock timed
([wheel distance](SCROLLING.md#wheel-distance), [wheel glide](SCROLLING.md#wheel-glide)).
A glide that reaches the edge of the prepared strips waits there and resumes with the
remaining distance when the strips arrive.

Known limit: the renderer uploads a strip's texture when it first draws it, in the frame
the strip scrolls into view (about 1.3 MiB at 1312 px wide, 2.6 MiB at 2624 px). The pinned
Slint FemtoVG renderer keys the texture of a pixel-buffer image by the item that draws it
and skips items outside the clip, so a strip cannot be uploaded before it is visible
without renderer changes. Cutting strips into four images of 64 rows, so that a frame
uploads less, made frames worse in native runs (more items and images per strip) and was
not kept ([record](../verification/2026-11-23-scroll-speed.md)).

A refill paints only the lines, Markdown blocks, table cells and glyphs whose ink
can reach the raster. That reach is a conservative bound, not a font guarantee:
4 em vertically and 8 em horizontally around each glyph origin (stacked marks
carry their own origins), plus the exact extent of every underline and
strikethrough. It covers the glyphs of real text fonts; a malformed or unusual
font whose ink extends further can lose that ink at raster edges. The engine
allocates each body, gutter and viewport-only frame buffer once, as the shared
`Arc<[u8]>` the frame carries, and paints it while it is still unique
(`rasterize_into`, `paint_over_into`). Delivering a strip therefore costs no
second allocation or full-surface copy; the UI's preparation worker converts each
strip once and reuses the conversion while later frames still carry it.

Raster pixel coordinates map through the frame's retained logical viewport,
not through the current clipping canvas. Source/Markdown controls appearing with
the first frame therefore cannot squeeze existing text. The body, gutter,
translation, selection and legacy rebase-resume mapping use the same effective
geometry in each axis, including when backend pixel limits reduce resolution.
A matching ordinary-to-retained opening transition preserves that extent;
metadata for the same retained surface does too. Parent-captured DPI is applied
before the child's first physical sizing; later native scale changes still cause
normal viewport updates. See [presentation](VIEWER_PRESENTATION.md#first-display-and-raster-geometry).

One presentation worker per viewer lives in `midnight-viewer-ui`. It validates
buffer dimensions and byte budgets, then copies RGBA into safe
`SharedPixelBuffer<Rgba8Pixel>` allocations off the UI thread. Only creation of
Slint images from those shared buffers and window updates happen on the UI thread.
The engine still publishes toolkit-independent pixels and metadata; reads,
parsing, shaping and rasterization stay on their backend workers. This adapter
adds no source reads, larger decoded pages or whole-file loading.

At most one prepared batch waits for UI consumption. Until it is taken, the
worker does not drain another batch, so the source mailbox keeps its
eight-output bound, latest-frame coalescing and reserved raster-control slot.
There is no task or growing queue per frame. A failed event-loop wake is retried
without requiring another source frame. Delivery is event driven; in addition,
each open viewer runs one 16 ms UI timer (`midnight-viewer-ui` `lib.rs`) for the work that has no completion
event: it acknowledges layout geometry and detects canvas or DPI changes, takes
a prepared batch whose wake was lost, retries position requests that a full input
queue refused, advances image motion fallback and presents the late font catalog.
An empty poll does not wake the preparation worker. The conversion cache reuses shared
body/gutter buffers for the same generation, surface, pixel identity, dimensions
and viewport; it does not retain the source pixel Arcs or their backend reservation.
Frontend shared buffers remain separate from the backend raster storage budget.

Delivery keeps the generation, frame revision, position sequence,
surface and rebase checks. Prepared pixels must match the delivered frame revision.
Rejected frames cannot replace the accepted buffer lease; conversion failures
leave control/status delivery available. Accepted replacement buffers keep the
previous display alive through image replacement, then retire its lease away from
the event loop. Closing signals cancellation without joining the preparation
thread. The worker reclaims its pending batch/cache, and late UI deliveries use
backend retirement rather than updating a closed window.

## Images

Image presentation retains immutable raster tiles and changes their positions
without uploading all pixels for every input event. The visible camera and a
pending requested camera are separate: a thumb can point at a target whose pixels
are still loading. A visible camera advances only when its required tiles exist.
Surface identity and sequence checks prevent a late reply from moving a new view.
A scrollbar target that still waits for its tiles outlives a new surface: the camera
maps the waiting thumb positions onto the new surface's extent and asks the new
surface for them, so a frame painted for an earlier drag step cannot leave the view
behind the released thumb. Home and End (`viewer.first`, `viewer.last`) move the camera
the same way to the image's start or end along the direction it scrolls in: down when
it is taller than the view, else across. The backend's Seek would not move it, since
the camera, not the backend, owns a retained image's view (`image_key_tests`).
Decoder work and tile caching remain on backend workers. The UI texture cache
copies each newly arriving 256×256 tile into a Slint buffer on the event loop and
reuses it by tile identity, so panning copies only newly exposed tiles. For images it
keeps at most 128 tiles and 32 MiB, and the UI refuses a frame with more tiles. These
bounds, which the backend tile cache uses too, are defined once in `midnight-core`
(`VIEWER_TILE_CACHE_BYTES`, `VIEWER_TILE_CACHE_ENTRIES`).

## Documents (PDF)

Documents use the image tile presentation, at exactly the shown zoom. Measurements:
[PDF scrolling](../verification/2026-11-21-pdf-scrolling.md).

**Canvas and tiles.** The engine keeps one tile canvas per document layout: the current
page in the Paged layout, the column of every page (`image_tiles::CANVAS`, pages 24
points apart, `PAGE_GAP` in `document_view.rs`, and centred) in the Continuous layout. Tiles are 256 × 256 premultiplied
RGBA keyed by canvas, scale and tile position; a page's rotation and crop belong to the
page, so they need no key of their own. One raster pixel is one physical pixel at every
view size (`image_scene::document_raster`): the cache budget bounds memory, never a
smaller rendering of what is shown at rest. Only a view beyond the tile position bound
(`VIEWER_TILE_POSITION_BOUND`, 16,384 raster pixels with room for guard rows, wider than
three 4K screens) is reduced.

**Bands.** Missing tiles are rendered in bands, rectangles of whole missing tiles of at
most 16 tiles across and down and 64 tiles in all (`MAX_BAND_EDGE`, `MAX_BAND_TILES` in
`image_scene.rs`), within the provider's 4096-pixel edge
and 4-megapixel viewport bounds (`Scene::next_band`). A band starts at the first missing tile of the
plan, spans the adjacent missing tiles of its row and then whole rows in the direction
of motion. The image actor asks the provider once for each page the band meets and
copies its pixels straight into the band's tiles (`document_render::render_band`); only
an antialiased edge where two pages meet within one pixel blends. Tiles that meet no
page (between and beside pages, beyond the canvas) are created transparent without
rendering. One band is in flight at a time, on the actor, never on the UI thread or the
viewer worker. The plan holds the visible tiles first, then the guard tiles: one column
beside the view and half a view of rows ahead in the direction of motion, at most six
rows (`viewer_document_guard_rows`), nearest first.

**Publication is never starved.** A band finished for an older camera keeps every tile
that the latest camera's plan needs (`Scene::complete_band`); other tiles of the same
canvas and scale are kept while the budget has room. No completed work is discarded
because a newer camera arrived. A frame is published whenever every visible tile of the
latest camera exists and the source was validated for its surface, by a band or a
validation requested since the latest camera change other than a pan; guard tiles are
published as they arrive. A pan over retained tiles therefore never waits for a source
check (the UI shows the same tiles without asking); the pan's own check follows when
the actor is idle, so a changed source is still noticed at once. The provider validates
the captured source before and after every page slice, and a band without slices
validates it once. A camera the UI asks for while the visible tiles of the camera in
progress are still rendering waits until that camera is published
(`ImageView::deferred_pan`), so a fast scrollbar drag shows the pages it passes instead
of chasing every new target and showing none.

**Camera.** The UI's tiled camera (`image_camera`, `image_motion`) moves the retained
tiles at display rate without a backend round trip and asks the engine to follow with
`ViewerImageInput::PanTo { surface, sequence, origin }`. A pan keeps the surface. Every
other camera change (page turn, zoom, Fit, layout, resize, and `Scroll`/`Seek` input
that did not come from the tiled camera) gets a new surface, so the UI adopts the
engine's camera; textures are reused by tile identity across surfaces. Coordinate
inputs (zoom anchors, layout switches) refer to the shown surface, so they are accepted
for any frame of it (`ImageFrames::surface_frame`); a resize still fences them.
`ViewerImageInfo::extent` is the scrollable extent: the page in the Paged layout, the
whole column in the Continuous one. Tiles are drawn on whole physical pixels: the
centring margin is rounded, the canvas phase removed and a document camera snapped to
whole raster pixels, so glyph edges are not resampled between screen pixels.

**Page steps.** Ctrl+PageDown, Ctrl+PageUp and the page buttons send
`ViewerInput::DocumentPage { frame, delta }`, a step relative to the latest requested
page, not to the page shown. It carries no coordinates, so the engine accepts it for any
frame published for the open document (`ImageFrames::accepts_page_step`), including one
the window still showed while the page before was rendering; a frame of another
document, or one never published, is refused. Steps add up and are clamped to the
document's pages (`ImageView::page_step`): n presses move n pages, never past the first
or the last page, and no step is queued. A step made while the visible tiles of the
camera in progress are still rendering waits with those before it until that camera is
published, as a pan does, so a held key shows the pages it passes instead of chasing
every step and showing none, and stops as soon as the key is released. Another camera
input (a key scroll, a seek, a zoom, a layout switch, a resize) turns the waiting steps
first, so it applies to the page they reach. A page turn retires the coordinates of
every earlier frame (`ImageFrames::page_turned`): a zoom anchor or layout switch made on
the previous page is refused until the new page is published.

**Budget.** The backend tile cache and the UI texture cache each keep
`viewer_document_tile_cache_bytes(view)` for a raster view of `view` pixels: the shown
and prefetched tiles and half again (at most 64 MiB more), and at least three views
within 32–96 MiB when that is more. Pixels in all on the backend (cached, still shown
after eviction, or reserved for the band in flight) stay within twice the budget.
Examples:

| Raster view | Budget each (backend, UI) |
| --- | --- |
| 1920 × 1080 | 33.75 MiB (135 tiles) |
| 3000 × 1950 | 73.5 MiB (294 tiles) |
| 3840 × 2160 | 101.25 MiB (405 tiles) |
| 4000 × 2600 | 121.5 MiB (486 tiles) |
| 7680 × 4320 | 262 MiB (1048 tiles) |

GPU textures mirror the UI cache. Images keep their 32 MiB / 128-tile budget and their
bounded raster geometry.

**Provider.** The PDF plugin keeps Hayro's caches (parsed fonts, glyph outlines and
bitmaps, decoded images) for the two pages rendered last at their scale, so a page's
later bands reuse them; any other page or scale replaces the older entry. The host
sends the plugin bounded `PageViewport` requests.

**Text viewers** keep their retained-raster path (`worker_raster.rs`, `raster_camera`),
owned separately. The same rules apply there: keep completed work that covers the
latest target, publish on coverage, and render ahead off the input path.

**Zoom.** A zoom step shows the retained tiles at once at the new zoom, scaled around
the step's anchor (`Textures::preview_zoom`; steps compose), until the new surface's
visible tiles arrive and replace them at exact resolution.

**Known limit.** A scrollbar jump to a region with no retained tiles keeps the old
view until the new region's visible tiles arrive (one or a few bands); there is no
low-resolution placeholder layer.

## Verification

`midnight-viewer-ui/tests/position_bar.rs` checks actual software-rendered thumb
pixels for both axes, grab offsets, reversals, stale metrics and cancellation.
The viewer UI's presentation tests exercise the real manager callbacks and
delayed frames. `scrollbar_drag_tests` drags the real vertical bar of a PDF in four
pointer steps while the backend answers each step, with a new surface per step in
the Continuous layout, forward and back, and in the Paged layout; the camera test
`the_released_thumb_wins_over_a_new_surface_painted_for_an_earlier_drag_step`
covers tiled images. Engine tests check bounded ingress, input barriers and exact
viewport completion. Opening regressions compare actual body/gutter marker bounds
before and after source controls appear, including retained activation and hit
testing. Placement tests check captured DPI before the first physical sizing and
preserve later native scale changes. One-pixel checkerboards at a fractional scale
and canvas origin prove texel-exact retained and viewport-only presentation; the
`texel_alignment` example repeats that check in a native window. Pixel-preparation
tests exercise one-batch backpressure, metadata buffer reuse, failed wakes,
invalid buffers and cancellation. Renderer tests compare culled painting with
unculled painting, including far decorations and ink above table cells.
Generated fixtures and logs belong under `_build/`.

A scrollbar position reads its page from some context ahead of the byte the position
names. The position's page drops the row it starts inside (`Worker::trim_position_start`,
as a backward chain drops its cut row) when the row after it ends at or before the named
byte, so it starts at a row and the page read above it keeps its context: the first notch
up after a scrollbar drag (it wants screens above the window) glides on without handing
the camera over to a corrected surface (`RasterRebase`). Tests:
`worker_frontier_tests::a_notch_up_after_a_scrollbar_position_needs_no_corrected_surface`
(the landing, no handoff, a notch down, up and five down, a notch at once, PageDown and
PageUp) and the ignored measurement `measure_handoffs_after_scrollbar_positions` (Text and
Source, wrapped and unwrapped, windows of 600 and 720 pixels at 1x and 900 at 1.5x,
positions 0.3, 0.4 and 0.5; it prints each landing and handoff). Known
limit: a position whose context holds fewer than two rows (very long rows) keeps its cut
row and still meets a handoff, and so do the pages of correction reads, which stay whole.
Across a handoff the window keeps the whole travel with winit's frames and with the
acceptance seam's (`acceptance_seams_tests::a_notch_up_across_a_corrected_surface_*`; see
[Wheel distance](SCROLLING.md#wheel-distance)).

For native measurements, build the `midnight-viewer-ui` example
`performance_probe` in debug and stage it beside trusted viewer packages in the
isolated QA runtime. Run it with an owned fixture directory and report path:

```text
performance_probe <fixtures> <report> --scrollbar
```

The probe sends real pointer events to the rendered vertical scrollbar. It matches
each measured thumb position to the latest pointer target, records visible content
updates while dragging, and measures final content readiness separately. Image
readiness uses the displayed camera, not the optimistic scrollbar target. Use
`--count` separately for source-read counts and `--load` for synthetic CPU load.
Wheel runs with `--extended-wheel` add generated Markdown and binary fixtures plus
any read-only files listed in `MIDNIGHT_VIEWER_PROBE_REAL_FILES` (a platform path
list); `--real-wheel` measures only those listed files and requires the variable.
Do not compare debug measurements with release results or report thumb latency as
document-render latency. Native measurements must not access user preferences.
The native test fails if content changes fewer than twice during the gesture;
moving the thumb alone is not an acceptable result.

Opening proof captures Slint offscreen rerenders without gestures:

```text
performance_probe <fixtures> <report> --d3d --native-parent --opening-proof --file=<source-file>
```

`--native-parent` captures placement from an already native parent, matching the
application's viewer launch path. Standalone logical-size startup has no parent
DPI to seed and is a distinct scenario. Keep startup modes consistent in before
and after comparisons. At each observed viewer redraw, the probe latches the publication revision and geometry; it rejects a snapshot if that state changes before capture. These crops match the observed redraw state, but do not read the compositor framebuffer or cover Slint's direct pre-show draw. Opening snapshots are excluded from cadence comparisons.

Measurements: [text viewer follow-up](../verification/2026-11-11-text-opening-scroll.md).
