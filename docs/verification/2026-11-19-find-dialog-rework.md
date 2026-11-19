# Find window: search folders, syntax help, error details, resizable columns

Implemented in `crates/midnight-ui` (UI and presenter only; no engine, core API or
dependency change):

- **Search folders**: one dropdown lists every root with a checkbox
  (`ui/find-roots.slint`). Ticking keeps the list open; clicking a name chooses the
  folder Remove removes. Add sits before Remove and opens a one-line entry (typed
  absolute path, Enter or OK) with Browse…, which sends `PickPath` (Directory)
  bound to that Find session (`presenter/find_roots.rs`). Only ticked roots enter
  `StartFind`; none ticked is refused before the queue. The old "Add another
  absolute folder path" field under More options is gone. Roots keep their raw
  native `PathBuf`; the window updates its root model in place on a tick.
- **Syntax help**: Help beside the mode chooser, or F1, covers the results with
  mode-specific help and clickable examples (`ui/find-help.slint`); text follows
  `midnight-platform-common` `filter.rs` and `filter_expression`.
- **Errors**: the query error and the status line under the results are
  clickable (icon and text, tooltip "Click for more details") and open the details
  page, which has Copy and Back. The footer Details/Query button was removed.
- **Columns**: every result column, including the identical-group column,
  resizes by dragging the separator after its header; widths live for the
  window's session and a drag never sorts.

## Verification (2026-11-19, Linux build image, software renderer)

Run from this worktree with sources touched first (shared target directory):

- `cargo test -p midnight-ui --lib find` — 25 passed, 0 failed (includes the new
  `presenter::find_roots_tests`: ticked-roots-only search, root limit counting
  unticked roots, Browse bound to its own window with stale/relative answers
  refused, close cancels an open chooser).
- `cargo test -p midnight-ui --test integration -- find path_elision dialog_theme::find tool_reflow`
  — 36 passed, 0 failed (includes the new `find_window_controls` tests for the
  folder list/Add/Browse, error and status lines opening details, help with
  examples and F1, and dragging all five column separators).

Not verified here: the native folder chooser itself (portal/Windows dialog) and
installed native-desktop behaviour; the full workspace suite was not rerun.
