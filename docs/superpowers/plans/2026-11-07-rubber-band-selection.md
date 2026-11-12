# Rubber-band selection implementation plan

SELECT-01 is authorized by the accepted implement-all goal and the parent review of
the bounded range API. No dependencies are added. Native paths and archive entry
IDs remain backend identities; display rows are meaningful only in a captured
immutable view.

## Behavior and resource contract

- Start a left mouse drag on empty list background, including unused grid cells.
  Existing FileEntry pointer areas retain all row clicks and file dragging.
  A background TouchArea contains the existing virtualized PanelList, so children
  receive input first and the SDK supplies actual pointer modifiers.
  A 10 logical pixel background gutter before the independent vertical scrollbar
  remains reachable even in a full Details viewport. Every layout uses the same
  reduced grid width; horizontal content displacement cancels the capture.
- Unmodified drag replaces marks, Shift adds, Ctrl toggles against the captured
  selection. Ctrl+Shift adds. A six logical pixel threshold distinguishes a drag.
- Capture navigation, immutable display index, search revision, selection revision,
  layout and parent-row offset. Require a complete stable listing and no pending
  selection/search work. Cancel on any captured identity/layout change, navigation,
  modal input, focus loss, Escape or lost pointer ownership. Cancellation never
  sends a selection mutation.
- Convert the rectangle to at most 4096 sorted disjoint inclusive index spans.
  Details/Thumbnails use row-major cells; Brief uses column-major cells within each
  vertically stacked page. Skip the parent row. Bound construction work as well
  as the resulting message. Report resource exhaustion rather than partial marks.
- Keep a presentation-only preview; only visible rows consult it. Release submits
  one backend request. Queue refusal removes the preview, retains the original
  selection and displays the existing selection queue error.
- A short UI timer scrolls near vertical edges, clamps to the actual content
  bounds and recomputes from the content-space anchor. It stops on cancellation
  or release. Existing idle row snapping pauses during the gesture.
- Archive selection requests/results carry an independent revision and the
  captured immutable view. A stale result cannot replace a new view's selection.

## Files and implementation sequence

1. `midnight-core/src/selection.rs`: validated shared span set and selection mode;
   `tests/selection_ranges.rs`: empty/disjoint/Unicode-independent identity,
   overlap/order/range/resource rejection and replace/add/toggle behavior.
2. `midnight-engine/src/selection.rs`: apply spans on the captured native view;
   focused unit tests for disjoint results, prior selections, streaming rejection
   and stale revisions. Reject incomplete sources rather than retargeting.
3. `midnight-core/src/archive.rs`, narrow `protocol.rs` ArchiveSelect/Selection
   fields, `midnight-engine/src/lib.rs` existing archive arm and a focused archive
   selection helper/test: preserve provider IDs and return view/revision provenance.
   Coordinate these narrow edits with the checksum agent's actor additions.
4. New `midnight-ui/src/selection_rectangle.rs` and tests: pure geometry, compact
   spans for all three layouts, parent exclusion, scroll edges and range budget.
5. New presenter marquee module/tests, `selection_preview.rs`, `archives.rs`:
   captured gesture identity, bounded immediate preview, commit/refusal/cancel,
   archive revision rejection. Keep filesystem and large selected-set work backend.
6. `panel_pointer.rs` / new selection bridge, narrow `lib.rs` registration and
   `ui/file-pane.slint` background pointer wrapper + clipped rectangle overlay;
   narrow `ui/main.slint` geometry/callback forwarding after parent build release.
   Preserve the direct PanelList repeater and unchanged FileEntry row handlers.
7. Deterministic input integration tests: row drag arbitration, all layouts,
   Ctrl/Shift, autoscroll/offscreen selection, cancellation and view replacement,
   denied queue, archives, density/font/window variants. Run focused native and UI
   suites plus strict Clippy, formatting and source-size checks. Store evidence in
   `_build/goal/selection/` and document any unverified native input boundary.

## Completion evidence

- [x] Contract/backend RED then GREEN, native and archive identity tests.
- [x] Geometry and presenter regressions GREEN.
- [x] Windows real Slint pointer/layout integration GREEN with unchanged row drag behavior.
- [ ] Windows and Linux backend checks, strict Clippy, formatting/source-size.
- [ ] Update feature documentation and `_build/goal/selection/report.md` with exact
  implemented behavior, explicit bounds and remaining evidence limits.
