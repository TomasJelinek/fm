# Panel rectangle selection

The panel background owns rubber-band gestures. Each layout reserves a 10 logical
pixel background gutter immediately before the pinned Fluent scrollbar. Empty
space below rows and unused Brief/Thumbnail cells also accepts a left mouse drag.
Row pointer areas keep click, modifier click and native file drag behavior. A plain background click only focuses the panel.

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

`SelectionRanges` validates at most 4096 (`MAX_SPANS`) sorted nonoverlapping inclusive spans
against a captured source extent. Details/Thumbnails map row-major cells; Brief
maps column-major cells inside vertically stacked pages. Parent navigation rows
are excluded. Contiguous large rectangles compact; too many separated spans report
the explicit range budget without partially committing a rectangle.

Native requests share the exact immutable `DirectoryIndex` and carry navigation,
selection context and monotonic selection revision. `SelectionSnapshot::from_bits`
accepts only indexes whose every row has a unique native path, so native
selections never cover archive or Network Neighborhood rows; archives use the
ID-based path below. Workers resolve bits and totals;
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
Stale archive response indexes and selected sets enter the retirement
queue, including the last owner of an obsolete listing.

## Selection by mask

Select and Deselect by mask (Ctrl+= or Ctrl++ and Ctrl+−, keypad + and −, or the F10
menu) and the keypad's invert (*) reuse the
three selection backends. `SelectionAction::Mask`, `ArchiveSelectionAction::Mask`
and `TreeSelectionAction::Mask` carry one `MaskSelection` (pattern, select or
deselect, folders). The worker compiles the pattern with the panel filter's Masks
grammar, ignoring case, and matches only rows of the captured view: native rows by
the final component of their raw path, archive members by their listed caption,
tree rows by their entry name. Folders and directory links take part only when
`folders` is set; other marks are kept. Native mask requests need a complete view:
a request queued while a listing streams fails without changing marks, and so would
an invalid mask. The dialog sends none. OK first checks the storage bounds and the
Masks grammar (`MaskSelectionPreferences::validate_mask`, with `FilterMasks`, the
parser the filter compiler uses too); a mask that passes is then compiled on a
backend worker, as the panel filter dialog has its filter checked:
`Command::ValidateSelectionMask` carries a `MaskValidationRequest` (an identity that
only grows, across openings too, and the pattern), the engine's filter validation
controller (`crates/midnight-engine/src/filters.rs`) answers only the latest one
with `Event::SelectionMaskValidated`, and
`midnight_platform_common::filter::validate_selection_mask` compiles exactly the
matcher the selection workers compile, refusing one over its 2 MiB budget (`REGEX_BYTES`; for
example thousands of `?`) with a reason. The dialog stays open until the answer;
an invalid or over-budget mask keeps it open with the reason and is neither applied
nor remembered, and answers to an earlier OK or a closed dialog are dropped.
Results fed to the panel while the mask is checked refuse the selection and close
the dialog. Loading settings checks only the storage
bounds, so a mask an earlier build remembered still loads and the dialog then says
what is wrong with it. Tree masks reach every selectable shown row, expanded
descendants included, never context-only ancestors. Keypad * sends `Invert` or
`InvertAll` following the remembered folder choice, and in search results fed to a
panel it inverts the list's marks the same way. The presenter keeps the dialog
draft and saves the last mask and folder choice (`MaskSelectionPreferences`) only
after the backend accepted the mask and its selection request was accepted.

## Tests

Core `selection_ranges`, engine `selection::tests::rectangle` and `archive_selection`,
UI unit `selection_rectangle` and input-level `selection_rectangle_ui`; the `drag_drop`
tests cover row gesture ownership. Design:
[rubber-band selection plan](../superpowers/plans/2026-11-07-rubber-band-selection.md).
