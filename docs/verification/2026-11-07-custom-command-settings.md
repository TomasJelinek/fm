# Custom Commands settings and drag ordering

The settings editor uses stable command IDs and the existing asynchronous
preferences transaction. Dragging only changes the proposed insertion position.
A valid changed drop persists one reordered configuration and keeps the command
selected; shortcut bindings remain attached to the same command ID. A drop in
the original position does not save. No gesture launches an external command.

The list scrolls near its visible edges. Escape, right-click, pointer cancellation,
closing Options, configuration changes and an outside drop cancel the gesture.
An unapplied form draft prevents reordering. Up/Down buttons remain available.
See [the settings contract](../developer/USER_COMMAND_SETTINGS.md) and
[the implementation plan](../superpowers/plans/2026-11-07-command-settings-drag-reorder.md).

## Regressions and review

- The initial pointer test failed with no saved order before drag routing existed.
- A captured drag released outside the actual 320-pixel-wide window originally
  saved an order because the inner list extended beyond its ancestor viewport.
  Drop validation now intersects the list and the Options content viewport.
- A draft's executable/icon edits cannot silently reconstruct a lossy native
  parent path. Unchanged native paths are retained exactly.
- A real wheel-input check found that the long form did not expose enough scroll
  content: Apply remained below the visible viewport. This is a separate layout
  regression from drag ordering. The page now sits in an unconditional layout
  that contributes its minimum height, with bottom spacing for the action row
  and the horizontal scrollbar overlay.

An early scrollbar test incorrectly assumed a vertical scrollbar was present
when content height equaled viewport height. Its failure is not evidence of a
production scrollbar defect. The corrected case requires actual overflowing
content before testing a drop on the scrollbar.

The interaction follows Salamander's list insertion, scrolling and cancellation
behavior in `src/edtlbwnd.cpp` and User Menu handling in `src/dialogs4.cpp`.
No Salamander code or assets are copied. No dependencies were added.

## Qualification

The final Windows Cargo run passed 479 tests: library 459, command settings 12,
toolbar 2, dialog movement 5 and general Options 1. The final portable footer
fixture was also freshly linked against the same production libraries and all
12 settings cases passed. This extra run is not added to the unique test count.

Linux passed library 456, checksums 3, Copy/Move dialog 2, editor options 1 and
toolbar 2. Its final settings rerun passed all 12 cases, giving 476 tests across
these explicit scopes. The footer fixture now scrolls the row into view and
measures its exposed area, rather than assuming identical font metrics on both
platforms. It still verifies cancellation when releasing over the footer.

The settings cases cover both drag directions, a nonselected source, stable IDs,
one-save/no-op behavior, dirty drafts, stale/deleted configurations, reentrant
cancellation, Escape/right-click, ancestor clipping, a real overflowing scrollbar,
virtualized edge scrolling, keyboard reveal and wheel scrolling to a visible
Apply button followed by an actual pointer click. Layout captures also cover
54 theme/density/window combinations.

Logs: `windows-drag-final-3.log`, `windows-drag-direct-final.log`,
`linux-drag-final-3.log` and `linux-drag-footer-final.log` under
`_build/goal/commands/`. The isolated layout probe independently established the
scrolling regression and correction before the final main-window rebuild.
Strict all-target Clippy passed for the UI and application on Windows and Linux;
the updated portable fixture is included. Workspace Rust formatting passed.
These software-window tests do not by themselves qualify native GPU startup.
Release startup evidence follows in the checkpoint report. Production preferences
are not modified by the fixtures.
