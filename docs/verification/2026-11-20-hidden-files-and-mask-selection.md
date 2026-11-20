# Show hidden files and selection by mask — 20 November 2026

Scope: acceptance findings AF-4 (no hidden-files toggle) and AF-6 (keypad + − *
started a filename search) of the [files acceptance run](2026-11-20-acceptance-acc-files.md).
Behavior is described in [keyboard shortcuts](../KEYBOARD_SHORTCUTS.md#hidden-files-and-selection-by-mask)
and the [selection contract](../developer/SELECTION.md#selection-by-mask).

## Design

- **Show hidden files** is one saved choice (`UserPreferences::show_hidden_files`,
  default shown) for both panels. `Command::ShowHiddenEntries` tells the engine,
  which records it per pane (`SnapshotState::hide_hidden`). Native listings drop
  hidden entries while they are read (streamed batches included), archive listings
  after the provider lists them, and inline-tree projections prune hidden rows with
  the folders' loaded contents. The platform policy is `hidden_entries`: dot-names
  on Unix, `FILE_ATTRIBUTE_HIDDEN` on Windows. The count travels in
  `DirectoryIndex::hidden` (listings) and `TreeProjection::hidden` (trees), so no
  event changed shape.
- A change re-reads both panels in place: the presenter tells the engine first, then
  sets each pane's refresh hint, so the existing background re-read keeps the rows
  that stay, moves a cursor on a removed row to the next surviving row and rebases
  the marks onto the new listing. A first listing still in progress (startup, before
  the settings were read) is asked for again. Trees use their maintenance refresh;
  a tree session rebuilds its projection when the choice it was built with changed,
  even if no listing changed. Archive panes are listed again.
- The panel's status line under the list shows an "N hidden" field (also in the
  path field's tooltip) while hidden entries are left out.
- Because hidden entries are absent from the listings, Ctrl+A, invert, mask
  selection, folder sizes without a selection, Make File List, F3's next/previous
  file and Compare directories only see shown entries. Find walks the file system
  itself and is unaffected.
- **Selection by mask** adds `MaskSelection` actions to the native, archive and tree
  selection workers, which compile the mask with the panel filter's Masks grammar
  (case-insensitive) and change only rows of the captured view. Keypad keys reach the
  application through the native key hook as `NumpadAdd`, `NumpadSubtract` and
  `NumpadMultiply` (new canonical shortcut keys), so the number row keeps typing.
- Settings schema 35 stores `show_hidden_files` and `mask_selection` only when they
  differ from the defaults; versions 33 and 34 load unchanged; a version-35 file
  must carry one of them; versions above 35 are refused.

## Commands

Docker build image as in the task brief, every Cargo command with `--locked --config
/workspace/_build/tmp/coord/feat-panel.toml`. Test binaries were built once with
`cargo test -p midnight-core -p midnight-platform-common -p midnight-engine -p
midnight-ui-input -p midnight-ui --no-run` and run directly with name filters.

```sh
cargo clippy -p midnight-core -p midnight-platform-common -p midnight-engine \
  -p midnight-ui-input -p midnight-ui --all-targets -- -D warnings
cargo clippy --workspace --features midnight-acceptance/acceptance --test acceptance \
  -- -D warnings
cargo test --workspace --features midnight-acceptance/acceptance --test acceptance --no-run
MIDNIGHT_ACCEPTANCE_RUN_DIR=/workspace/_build/acceptance-feat-panel/<UTC run id> \
  <printed acceptance binary> --scale smoke files_selection files_filter_search \
  tools_main_menu options_general files_hot_paths
```

## Results

New tests, each by name (all passed):

| Binary | Filter | Passed |
| --- | --- | --- |
| `midnight-core` lib | `mask_selection` | 2 |
| `midnight-core` integration | `panel_choice` (keypad key names, defaults, saved assignments win, editors keep keypad text) | 4 |
| `midnight-platform-common` lib | `panel_visibility` (schema 35 writing, 33/34 loading, strict fields, version 36 refused) | 5 |
| `midnight-engine` lib | `hidden_entries`, `mask_selection`, `mask_tests` (native, archive and tree mask selection) | 9 |
| `midnight-engine` integration | `hidden_entries` (listings, re-sort count, archive listing, tree reprojection and maintenance refresh) | 3 |
| `midnight-ui-input` lib | `keypad` (native hook dispatch and release) | 1 |
| `midnight-ui` lib | `panel_choices` (presenter commands, re-reads, cursor, marks, status-line field, startup, Options, dialog, adopted-tree refresh) | 8 |
| `midnight-ui` integration | `panel_choices` (keypad, Ctrl+H, View menu, F10, Options, status line, dialog by keyboard and mouse) | 3 |

Neighbouring groups, same binaries: core `mask_selection shortcuts tree_view` 2 and
integration `panel_choice shortcuts preferences tree_view` 74; platform-common
`settings` 55; engine lib `hidden_entries mask selection tree_ search::` 459 (5
ignored); engine integration `hidden_entries archive_selection tree_filters
tree_controller search navigation` 75 (3 ignored); `midnight-ui-input` lib 49; UI lib
`panel_choices preferences file_keys search selection tree shortcut keyboard` 487; UI
integration `panel_choices main_menu keyboard_options bulk_selection options
panel_view hidden_icons panel_search` 144. All passed. In an earlier run under heavy
parallel build load `watch_native::a_failed_navigation_keeps_real_native_hints_for_the_shown_directory`
timed out waiting for a native watcher; all 12 `watch_native` tests passed when run
alone (`--test-threads=1`). After the fixes below, UI lib `panel_choices preferences
file_keys search selection tree shortcut keyboard details` passed 515 and UI
integration `panel_choices main_menu keyboard_options bulk_selection options
panel_view hidden_icons panel_search details status` passed 170.

Clippy with `-D warnings` reported nothing for the five crates (all targets) and for
the acceptance target; `rustfmt --check --edition 2024` passed for the 58 changed
Rust files.

The earlier acceptance runs and their screenshots showed three problems, all fixed
before the final run:

- Showing hidden files with an expanded inline tree overflowed the stack: the
  tree's maintenance refresh flushes background refreshes, which re-entered the
  hidden-entries re-read before it returned. The re-read flag is now cleared
  first; a presenter test drives that path with an adopted tree and expects one
  tree refresh (it was not run against the unfixed code).
- The "N hidden" count was only in the path field's tooltip; it is now a field of
  the status line, checked by the presenter, window and acceptance tests.
- The scenario pressed Enter on the focused Include folders check box, which
  toggles it, as every check box does; it now clicks OK there and presses Enter in
  the mask field otherwise. The dialog also became wide enough for OK and Cancel to
  share one row.

Final run (smoke scale): `files_selection`, `files_filter_search`,
`tools_main_menu`, `options_general` and `files_hot_paths` passed (5 passed, 0
warned, 0 failed, 127.6 s), with no review flags. `tools_main_menu` checks the
four new F10 entries: Select and Deselect by mask open their dialog, the keypad's
invert marks only files, and Hide hidden files becomes Show hidden files and back.

Not verified here: the Windows hidden-attribute policy (mirrored, not compiled),
real keypad events from a desktop session (the scenarios call the window's
physical-shortcut entry that the native key hook uses), and the full test suites.
