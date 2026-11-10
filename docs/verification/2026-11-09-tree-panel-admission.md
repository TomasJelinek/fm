# Inline tree panel admission, 2026-11-09

This report qualifies the current browsing slice. TREE-01 remains open; it does
not claim typed operation parity, complete filtering/watches, or native plugin
end-to-end UI qualification.

## Delivered behavior

- Lazy session enablement on explicit directory disclosure, preserving ordinary
  flat-panel behavior before expansion.
- Exact root entries/index and selection-revision migration; a later flat search
  or selection cannot be overwritten by a pending tree publication.
- Atomic projection/selection adoption, exact Commit ordering and retry after
  controller rejection, virtualized depth/disclosure data, and stale callback
  rejection.
- Occurrence selection with bounded immediate feedback, range/rectangle input,
  pending-toggle replay, failed-selection recovery and stable focus on collapse.
- Typed native/archive directory navigation. Original root regular files retain
  their explicit launch path through the captured original-root alias. Folder-only
  shortcuts cannot launch them, and descendant IDs cannot substitute root IDs.
- Explicit action admission guards for unconnected tree consumers. Leaving a tree
  creates a fresh backend selection context, including failed refresh restoration,
  so pre-tree native marks cannot silently return.

Implementation and current limits are described in
[TREE_PANEL.md](../developer/TREE_PANEL.md). No dependencies, index entries,
commits or remote branches were changed by this UI task.

## Evidence

Windows checks used repository-local Cargo/Rustup, Rust 1.99.1, `--locked
--offline`, `_build` output, debug information disabled for dev/test, and
incremental compilation disabled.

| Check | Result | Log under `_build/` |
| --- | --- | --- |
| Tree presenter regressions | 14 passed | `tree-panel-review-green.log` |
| UI unit suite with the independently tracked toolbar test excluded | 577 passed, 1 filtered | `tree-panel-final-units.log` |
| Installed tree bridge plus shared disclosure widgets | 4 passed | `tree-panel-focused-integration.log` |
| UI all-target Clippy with `-D warnings` | Passed | `tree-panel-final-clippy.log` |
| Source-size policy | Passed | `tree-panel-final-source-size.log` |
| Broad integration run | 72 passed before one dialog failure stopped the run | `tree-panel-final-integration.log` |

The installed bridge test clicks the actual disclosure widget. The widget tests
cover both panes, all three layouts, stale/replaced presses, cancelled movement
and deep indentation/compact geometry. Presenter tests additionally cover archive
root identity, controller backpressure, caret/selection continuity and explicit
operation refusals. They do not enumerate files or substitute a copied tree
implementation for the presenter.

Causal RED logs were recorded before the relevant fixes: `tree-panel-red.log`,
`tree-panel-unit-red.log`, `tree-panel-selection-red.log`,
`tree-panel-admission-red.log`, `tree-panel-folder-only-red.log`,
`tree-panel-review-red.log`, and `tree-panel-insert-red.log`. Independent review
found four issues: folder-only launch, stale flat selection revival, search
overtaking pending adoption, and rejected toggles advancing the caret. Their
regressions failed before the fixes and passed afterward.

The final source manifest is `_build/qa/tree-panel/source-manifest.json` and
records the 30 UI source/test/document files involved. The only change after the
577-unit run was an equivalent match guard requested by Clippy; the focused
integration and final lint runs compiled that final source.

## Open qualification gates

The full UI suite is **not** reported green:

1. `scroll_input::pane_tests::toolbar_tests::delayed_native_toolbar_touch_cannot_retarget_after_panel_or_location_changes`
   fails at `scroll_input_toolbar_tests.rs:171`: the mouse click after cancellation
   produces no callback. It also fails in isolation; see
   `_build/tree-panel-toolbar-recheck.log`. This uses a bare window/file-key bridge,
   not the tree presenter.
2. `dialog_headers_move_and_escape_keeps_existing_close_behavior` fails at
   `tests/dialog_drag.rs:289` because the popup-title lookup returns `None`.
   Four other dialog-drag tests pass. This also uses bare widget fixtures. The
   root task owns investigation of both unrelated UI failures.

The broad integration run stopped at the second failure, so later integration
binaries were not qualified by that run. The two tree integration binaries were
run explicitly afterward and passed. The root task's intentionally failing new
delete-confirmation test was not run by this UI task.

Typed descendant/root operations in adopted trees remain guarded, including
mutation, mixed Copy/Move, clipboard, viewer/editor, comparison, sizes, checksums,
Custom Commands, native context menus and dragging. File comparison currently
refuses a launch if either pane has adopted a tree. Tree query/filter routing,
expanded-branch watches, container discovery/attachment, nested archives and
expanded-state retention on refresh remain open. Windows/Linux native plugin and
application qualification still belongs to the wider production integration.
