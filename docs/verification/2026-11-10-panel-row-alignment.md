# Folder-link alignment and partial-size warnings

The reported Windows screenshot showed `Documents and Settings` shifted left of
ordinary folders, with folder-size error glyphs detached from their numbers.
Native read-only inspection identified that entry as the compatibility junction
to `C:\Users`. Automatic size scans intentionally exclude junction targets.

The shared Details/Brief row now reserves a disclosure gutter for every entry.
Rows without an expansion control therefore keep the same icon/name alignment.
The Size cell places flexible space before the compact warning/value pair.
Actual read errors still mark partial totals; running or cancelled work alone
does not acquire an error glyph. No traversal or permission policy changed.

New actual-widget regressions reproduced name positions of 63/43/43 pixels and a
40-pixel gap between the warning and its number. Both pass after the layout fix.
An initial test-fixture lookup was corrected to include Slint `ComplexText`; its
first lookup failures were not product-regression evidence.

Windows software-rendered verification passed all six tests in
`panel_row_layout`, `size_errors` and `tree_disclosure`. They cover Details/Brief
alignment, the warning beside its number in a wide pane, error versus incomplete
states in all three views, disclosure input isolation, stale presses and deep
indentation. Two rendered snapshots were also visually inspected.

The run used an independent generated Cargo harness that imports these repository
tests and the production UI crate, avoiding the concurrently edited engine test
dependency. Its manifest, exact logs, before-images and BMP snapshots are under
`_build/qa/panel-size-alignment/`. No dependency was added to the application.
This is software-renderer layout evidence, not installed GPU visual qualification.

The integrated Windows run also passed all 735 UI library tests and 35 affected
UI/native bridges, including these row tests, sizing and watcher-backed Copy/Move.
Core/UI all-target no-deps Clippy, workspace formatting and source-size checks
passed. Strict engine lint retains the separately documented pre-existing unused
capture fields; this is not a claim of clean workspace lint.

The final debug app was rebuilt after the backend's mechanical lint cleanup:

- Build: `D:\experiments\dev\FM\_build\debug\me.exe`.
- Private installation: `C:\Users\tjelinek\MidnightExplorer-20261110-html\me.exe`.
- Both SHA-256: `AB8628BFF2BB6396F673C754E6712E6A7B7C9C2B9D497A55517C4F4109705B34`.
- SHA-bound private plugin runtime registration refreshed for that exact build.
- Actual build-path `--smoke-test` exited 0; stderr was empty.

No Windows plugin package or filesystem traversal/permission policy changed.
Logs and the build/hash/startup records remain under the same QA directory.
