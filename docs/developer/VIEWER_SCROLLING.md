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

The backend retains native byte offsets as `u64`. It acknowledges a vertical
position only when its exact viewport request supplied a usable page and completed
successfully. An error/cancellation establishes a fresh context and publishes the
retained view; a no-match search also publishes its new context. Horizontal intent
survives a matching page arrival, but cannot override newer relative horizontal
scrolling. Old context/sequence replies cannot pull a pending thumb backwards.

These messages are application-owned, not a change to the external viewer plugin
ABI. Plugins continue to receive bounded viewport requests through the shared API.

Behavioral reference: Salamander `src/viewer3.cpp`, `OnVScroll` and the
`SB_THUMBTRACK`/`SB_THUMBPOSITION` handlers, retains the latest tracking position,
updates content on a 20 ms timer and explicitly applies the final position on
release to avoid a flash at the old location. This implementation uses asynchronous
backend messages and bounded coalescing; no Salamander code is copied.

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
viewport completion. Generated fixtures and logs belong under `_build/`.

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
