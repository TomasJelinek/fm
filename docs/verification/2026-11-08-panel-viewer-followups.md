# Panel and viewer follow-up checks — 8 November 2026

These are working-tree Windows debug/test results. They do not establish release
deployment, native image-pan latency or completion of the broader feature backlog.
All generated test output remains under `_build/`.

## Panel sorting

Both instances of the shared file panel already offer **Extension** in the sort
dropdown in detailed, brief and thumbnail modes. The configurable `sort.extension`
action defaults to Ctrl+F4. Choosing the current sort again reverses its direction.
Backend sorting uses captured native locale keys and retains directories first;
missing extensions remain last in either direction.

The installed-command-bridge regression invokes the same sort callback as the
dropdown for each panel, verifies the active pane and ascending backend request,
then renders the real window and sends Ctrl+F4 to its focused file list. It verifies
a descending request for that panel. This tests callback/keyboard routing, not a
native pointer click on the dropdown or physical storage performance. Existing
`midnight-core/tests/locale_sorting.rs` covers actual extension order and reversal.

## Combined main UI

The Windows `midnight-ui` library suite passed **494 tests**. Selected integration
targets passed **28 tests**:

| Target | Passed | Covered behavior |
| --- | ---: | --- |
| `archive_badges` | 2 | Archive overlays/fallbacks in panel modes and all 14 themes |
| `archive_options` | 1 | Built-in choices, decoder-specific custom extensions and conflicts |
| `command_bindings` | 2 | Extension sorting in both panels; literal command insertion |
| `commander_tools` | 3 | Configured drive information, panel zoom and command-history navigation |
| `dialog_resize` | 1 | Resize edges/corners preserve the draft |
| `error_notice` | 1 | Close collapses presentation; changed occurrence/text reappears |
| `find_windows` | 4 | Pointer sorting headers, current-directory line, selection, themes and 320×200 |
| `metadata_editor` | 2 | No obsolete preview list; prepared review/confirmation and readable sections |
| `parent_focus` | 1 | Parent navigation focuses and scrolls to the departed child |
| `path_elision` | 1 | Shortened path keeps the root clickable and edits the complete path |
| `path_paste_ui` | 9 | Clipboard input, undo, native identities and stale reply rejection |
| `volume_information` | 1 | Measured details/copy/resize; late reply cannot reopen a dismissed notice |

Commands use `scripts/cargo.ps1 test --locked --offline -p midnight-ui --lib` and
the listed `--test` targets, with development/test debug info disabled and
incremental compilation disabled. Logs:
`_build/goal/archive-hotpaths/combined-ui-green.log` (library/archive targets) and
`combined-ui-integrations.log` (remaining targets). Initial integration compilation
identified a missing import and an invalid private-field access in Find; both were
corrected before the passing run. The new keyboard regression needed the real
window layout rendered before dispatch, as in the existing UI input tests.

## Viewer preference input

The viewer's `keyboard`, `preferences` and `wrap_control` targets passed **7 tests**.
The new preference test opens two actual Slint viewer windows, changes separate
fields, verifies synchronous acceptance, immediately closes through Escape, and
reopens through a new window manager. It checks that both choices survive, unchanged
choices do not create redundant saves and programmatic restoration is not a user
edit. Content capture deliberately returns an error; no user document is read and
the assertion does not depend on decoding or frame delivery.

This complements presenter merge/backpressure tests and the strict settings codec
tests. It does not substitute for native plugin reopening, atomic file-write tests,
or the final application restart workflow. Log:
`_build/goal/viewer-performance/viewer-persistence-ui-windows.log`.

## Remaining qualification

Image tile reuse and continuous scrollbar content progress now have
[native Windows measurements and a deployed debug build](2026-11-08-viewer-scrollbar.md).
Debug tail-latency profiling and optimized PDF panning remain follow-up work;
uniformly smooth rendering across all viewer modes has not been established.
Compact panel error rows still need a close control, and repeated identical host
errors need complete occurrence qualification. Archive Hot Paths still need the
real plugin/menu/restart workflow. Independent native dialog migration and release
deployment are tracked in the [implementation backlog](../IMPLEMENTATION_BACKLOG.md).
