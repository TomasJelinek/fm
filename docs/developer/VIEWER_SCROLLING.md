# Viewer scrolling

The UI owns pointer feedback. The viewer engine owns file reads, decoding,
search, text layout and raster preparation. Dragging a scrollbar must never wait
for these operations or enqueue an unbounded history of obsolete positions.

## Scrollbar input and text acknowledgements

The shared `PositionBar` retains the pointer's original grab offset, track length
and thumb extent for the duration of a drag. It paints the local target directly.
A stationary grab/release does not reload content. A changed content context,
resize or pointer cancellation ends the old gesture.

Markdown uses the same asynchronous viewport pipeline. Its independent plugin
counts semantic block lines, rather than only literal newlines, before cropping
the page. A long lookahead with many later blocks must not downgrade the visible
page to plain text. Parsing and safe-checkpoint discovery share one parser pass.
Each immutable session keeps an LRU of at most 16 semantic pages and 1 MiB, keyed
by anchor, direction and viewport budgets. Revisited resident pages require no
source read or parse; closing/reopening never carries cache across revisions.
Uncached distant seeks without a trustworthy block checkpoint retain the existing
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

Behavioral reference: Salamander `src/viewer3.cpp`, `OnVScroll` and the
`SB_THUMBTRACK`/`SB_THUMBPOSITION` handlers, retains the latest tracking position,
updates content on a 20 ms timer and explicitly applies the final position on
release to avoid a flash at the old location. This implementation uses asynchronous
backend messages and bounded coalescing; no Salamander code is copied.

## Retained text pixels and delivery

Text, source, Markdown and binary/hex can retain a bounded body raster and a
separate source gutter. Their surface identity names a layout/camera context;
immutable pixel identities distinguish new allocations from metadata updates.
The frontend moves covered pixels without waiting for a new backend raster and
keeps the gutter horizontally fixed. Each guard targets 256 logical pixels per
side, converted using the effective raster scale, including the initial parent
DPI. The combined body and gutter still fit the 24 MiB snapshot cap, so guards
shrink when necessary. Guard refills retain the existing 12 KiB text and 512-line
request ceilings; a higher DPI does not enlarge those source budgets. An uncovered
frontier pauses displayed motion until usable pixels arrive; it is not an inferred
end of file. Settling can request exact fractional glyph phase.
The [retained-raster contract](../superpowers/plans/2026-11-11-retained-viewer-raster.md)
defines coverage, continuation and snapshot budgets.

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
parsing, shaping and rasterization stay on their existing workers. This adapter
adds no source reads, larger decoded pages or whole-file loading.

At most one prepared batch waits for UI consumption. Until it is taken, the
worker does not drain another batch, so the source mailbox retains its existing
eight-output bound, latest-frame coalescing and reserved raster-control slot.
There is no task or growing queue per frame. A failed event-loop wake is retried
without requiring another source frame. The conversion cache reuses shared
body/gutter buffers for the same generation, surface, pixel identity, dimensions
and viewport; it does not retain the source pixel Arcs or their backend reservation.
Frontend shared buffers remain separate from the backend raster storage budget.

Delivery preserves the existing generation, frame revision, position sequence,
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
Decoder work and cache preparation remain on backend workers.

## Verification

`midnight-viewer-ui/tests/position_bar.rs` checks actual software-rendered thumb
pixels for both axes, grab offsets, reversals, stale metrics and cancellation.
The viewer UI's presentation tests exercise the real manager callbacks and
delayed frames. Engine tests check bounded ingress, input barriers and exact
viewport completion. Opening regressions compare actual body/gutter marker bounds
before and after source controls appear, including retained activation and hit
testing. Placement tests check captured DPI before the first physical sizing and
preserve later native scale changes. Pixel-preparation tests exercise one-batch
backpressure, metadata buffer reuse, failed wakes, invalid buffers and cancellation.
Generated fixtures and logs belong under `_build/`.

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
and after comparisons. At each observed viewer redraw, the probe latches the publication revision and geometry; it rejects a snapshot if that state changes before capture. These crops match the observed redraw state, but do not read the compositor framebuffer or cover Slint's direct pre-show draw. Opening snapshots are excluded from cadence comparisons;
these probe capabilities alone do not establish a passing native qualification.

The latest measured results, matched-redraw opening scope, GPU/software limits and
focused regression evidence are in [the text viewer follow-up](../verification/2026-11-11-text-opening-scroll.md).
