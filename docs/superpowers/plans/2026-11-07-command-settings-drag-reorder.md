# Custom Commands settings drag ordering

The user requested drag-and-drop ordering in the existing settings list. Keep
the accessible Up/Down controls and the accepted stable-ID preference transaction.
This changes only settings presentation/controller code; it does not launch a
command or alter the backend, schema, protocol, shortcut identities or native paths.

## Behavior and contracts

- A left press captures the command ID, a monotonic drag token, the immutable
  configuration's weak owner and its revision. Moving at least six logical pixels
  begins ordering; releasing without that movement remains ordinary selection.
- A thin drop indicator marks an insertion boundary between rows. The list's
  stable viewport owns the pointer throughout the gesture, so virtualized source
  rows may scroll out without changing the dragged identity. Scrollbar gestures
  retain their existing ownership.
- Edge scrolling runs at a bounded rate while dragging. Geometry uses logical
  coordinates, the list's actual content offset and one shared row height. At
  most 128 configured rows are involved; no entries or filesystem data are read.
- Drop revalidates token, ID, immutable configuration/revision, bounds and clean
  draft state before cloning/reordering. It then uses the existing validated
  `commit_user_command_list` path once. Same-position drops do not save.
- Dirty drafts refuse reordering with an explicit message. Escape, cancelled
  pointer capture, right-button press, closing/reopening Options, configuration replacement/deletion
  and stale callbacks cancel without reordering or discarding a draft.
- Rename/order changes retain command IDs and bindings. Save refusal keeps the
  accepted in-memory order explicitly unsaved through the existing save status.

## Files and sequence

1. New unhooked `presenter/user_command_settings/reorder.rs` and
   `reorder_tests.rs`: bounded pure capture/validation/order calculation, tested
   directly against the existing core contract before UI source release.
2. Narrow `presenter/user_command_settings.rs`: configuration revision, active
   gesture/token, begin/cancel/drop controller methods; call the existing list
   commit helper. Add presenter regressions for one save, stale/cancelled drops,
   dirty draft preservation, deleted IDs and no-op order.
3. Narrow `user_command_settings_presentation.rs`: publish gesture/configuration
   identity and bind controller callbacks without retaining RefCell borrows across
   Slint changes. Existing live form updates stay unchanged.
4. Narrow `ui/user-command-options.slint`: viewport pointer capture, indicator,
   threshold, bounded edge scrolling, cancellation and row selection. Preserve
   list accessibility and Up/Down actions. Coordinate bindings generation with
   root before this source changes.
5. Extend `tests/user_command_settings_ui.rs`: actual Slint pointer drag in both
   directions, one preference save, Escape/dirty/stale cancellation, autoscroll
   and continued Up/Down accessibility. Add captures only when useful.
6. Update `docs/developer/USER_COMMAND_SETTINGS.md` and the settings report with
   final evidence. Existing qualification remains explicitly the pre-drag build.

## Gates

- [x] Pure drag controller RED then GREEN (4).
- [x] Presenter transaction/cancellation regressions GREEN (3); actual widget
  ordering RED then GREEN through the same pointer path.
- [x] Actual settings drag/scroll and accessibility tests GREEN on both OSes (12).
- [x] Windows/Linux affected tests and strict Clippy, formatting/source size.

The earlier Linux UI/app strict qualification completed before wiring this feature.
Final Windows/Linux gates now use the frozen implementation and all 12 widget cases.
The visible rectangle intersects ancestor content clipping and excludes scrollbar
overlays using one documented Fluent 1.18.1 metric. The initial alleged outer
scrollbar failure was an invalid fixture: measured content height equaled viewport
height, so no vertical scrollbar existed at that release point. It is not evidence
of a production overlay defect. The corrected 64-command fixture verifies actual
inner-list overflow and cancellation on its real scrollbar.

The actual outside-ancestor release remains causally reproduced. A separate real
wheel check found the long page had viewport-height content while Apply was below
it. A preferred-height binding was tested and rejected. The compiler only derives
Flickable extent from direct non-repeated layout children; page9 now uses an
unconditional layout wrapper with its conditional page inside. The exact wrapper
and theme/overlay bottom padding passed the isolated real runtime probe: content
300→752, wheel offset−452, Apply y321–353 above overlay boundary366.
The final regression requires
wheel scrolling to reveal Apply and a physical pointer click to save. The compact
320×200 footer-drop regression remains; redundant wheel setup was removed.

## Reference behavior

Salamander's `src/edtlbwnd.cpp::MoveItem` (around369) validates insertion
boundaries, adjusts a downward insertion and preserves the moved selection. Its
mouse path (around984) uses a movement threshold, timer scrolling and an insertion
marker; the right-button/cancel path (around1015) aborts the gesture. User Menu
consumes `EDTLBN_MOVEITEM2` in `src/dialogs4.cpp` (around2580). This implementation
follows those interaction contracts through the existing immutable configuration
transaction. No reference code or assets are copied.
