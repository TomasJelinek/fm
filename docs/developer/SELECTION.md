# Panel rectangle selection

The panel background owns rubber-band gestures. Each layout reserves a 10 logical
pixel background gutter immediately before the pinned Fluent scrollbar. Empty
space below rows and unused Brief/Thumbnail cells also accepts a left mouse drag.
Existing row pointer areas retain click, modifier click and native file drag
behavior. A plain background click only focuses the panel.

Movement of six logical pixels starts the rectangle. Unmodified dragging replaces
the marked set, Shift adds, Ctrl toggles and Ctrl+Shift adds. Modifiers are captured
on pointer down. Only visible delegates repaint during the gesture; release sends
one backend selection action. A refused queue restores original marks and displays
the selection queue error. The timer scrolls near vertical edges while retaining
the anchor's content coordinates.

Escape, lost pointer ownership, host/panel focus changes, modal input, navigation,
sorting/filter changes, another selection action and changed layout cancel the
capture. Horizontal content displacement cancels rather than reinterpreting cells.
A stable complete listing and resolved selection are required to start; loading,
streaming and pending search/selection work do not expose an unstable source.

## Shared contract

`SelectionRanges` validates at most 4096 sorted nonoverlapping inclusive spans
against a captured source extent. Details/Thumbnails map row-major cells; Brief
maps column-major cells inside vertically stacked pages. Parent navigation rows
are excluded. Contiguous large rectangles compact; too many separated spans report
the explicit range budget without partially committing a rectangle.

Native requests share the exact immutable `DirectoryIndex` and carry navigation,
selection context and monotonic selection revision. Workers resolve bits and totals;
the UI neither walks the complete listing nor builds selected native paths.
Archives preserve opaque provider entry IDs even when display names collide or raw
member names are not UTF-8. Their selection responses return the captured immutable
view, independent selection revision and explicit result/error; stale responses
cannot replace the current view's marks. At most two archive computations or
blocked publications retain worker permits.

The gesture capture keeps a weak index owner, preventing address reuse while
allowing old large listings to be reclaimed by backend retirement. Its cancellation
token suppresses a preview even when cancellation arrives during a presenter
borrow; actual clearing/repaint can wait for the next short timer turn. Model
notifications happen after mutable presenter and gesture borrows have ended.
Stale archive response indexes and selected sets enter the existing retirement
queue, including the last owner of an obsolete listing.

## Verification

The implementation plan is
[`2026-11-07-rubber-band-selection.md`](../superpowers/plans/2026-11-07-rubber-band-selection.md).
Evidence and qualification limits are recorded under `_build/goal/selection/`.
Focused suites are core `selection_ranges`, engine `selection::tests::rectangle`
and `archive_selection`, UI unit `selection_rectangle` and actual input
`selection_rectangle_ui`. Existing `drag_drop` tests qualify unchanged row gesture
ownership. Windows/Linux final UI checks are coordinated with checksum integration;
consult the report rather than inferring completed qualification from source alone.
