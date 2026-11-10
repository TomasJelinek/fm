# Comparison toolbar and directory dialog

Both panel action toolbars now expose distinct Compare files and Compare
directories icons. The same icons appear in F10; the directory icon also appears
in its dialog. Both buttons activate their owning pane before dispatching the
existing command, including after swapping panels. Tooltips resolve the user's
configured shortcuts rather than embedding default key strings.

Compare files uses Ctrl+Shift+C's existing native-file preparation and text/image
comparison. Compare directories uses Ctrl+F10's existing background metadata
comparison, requiring two native panels. A parent-only selection cannot target
file comparison. Both directories may be compared without a selection.

Reviewed Salamander's `src/lang/lang.rc` (`IDD_COMPAREDIRS`),
`src/dialogs6.cpp` (`CCompareDirsDialog`), `src/mainwnd3.cpp`
(`CM_COMPAREDIRS`) and `src/mainwnd5.cpp` (`CompareDirectories`). The existing
Midnight Explorer subset selects unmatched entries and the larger/newer side
according to independent size/time criteria. The dialog now labels both paths
and explains selection replacement in the Compare tooltip. It explicitly limits
itself to displayed metadata; content, attributes and recursive comparison remain
unimplemented. No file mutations or new backend comparison algorithm were added.

## Validation

Evidence: `_build/goal/comparison-toolbar/`.

- New toolbar integration tests initially failed because both controls were absent
  (`red.log`), then passed using real pointer dispatch on both panels, swapped
  panels, mouse/Touch and widths 1340, 900 and 660. Availability checks cover the
  parent-only target and archive panels.
- UI integration gate: **18 passed** across `comparison_toolbar`,
  `commander_dialogs`, `commander_tools`, `dialog_drag` and `main_menu`.
- Engine comparison/selection checks: **11 passed**, including cancellation,
  stale results, missing metadata, tolerance boundaries and native Unicode names.
- Presenter comparison checks: **5 passed**; file-comparison capture checks:
  **2 passed**.
- Visually reviewed the normal toolbar, narrow Touch toolbar and directory dialog
  from rendered test snapshots. Large-font dialog draft preservation and X/Escape
  behavior remain covered by the integration gate.
- Workspace formatting and handwritten source-size checks passed (1161 files,
  each at most 100000 bytes). No dependencies or plugin APIs changed.
- `cargo clippy --locked --offline -p midnight-ui --all-targets -- -D warnings`
  passed.
- Windows debug `midnight-explorer` build passed. Starting `_build/debug/me.exe`
  with `--ui-smoke-test` and registered viewer/filesystem/thumbnail plugins exited
  with code 0 and empty stderr. The real preferences file hash was unchanged.

The executable SHA256 is
`8BBE191FAC651DB4CFCDC83B20AD83B0B0C64AB4E647BC569C11A8F443384F02`.
Its matching protected runtime is
`C:/Users/tjelinek/MidnightExplorer-20261108-023418-11e8dd13`.
Registration lets the debug build resolve these protected plugin files without
relaxing plugin path trust checks. See `app-build.log`, `deploy.log` and
`app-acceptance.txt` in the evidence directory.

This is Windows debug validation. Linux/macOS native execution and a new release
build are not part of this toolbar change.
