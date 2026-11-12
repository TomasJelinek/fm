# Recursive tree and CSC UI verification — 12 November 2026

Recursive expansion is implemented and tested on Windows with software UI coverage
and a real junction fixture. The integrated build exited 0 with zero source drift. The
`recursive-ui-viewer-thumbnail-runtime-result.json` receipt under
`_build/verification/build-20261112/` records:

| Scope | Passed / failed / ignored |
| --- | --- |
| Presenter tree 44 + folder presence 9 + elevation 9 | 62 / 0 / 0 |
| `tree_disclosure` | 5 / 0 / 0 |
| `tree_panel` | 2 / 0 / 0 |
| `copy_dialog` | 8 / 0 / 0 |
| Selected `native_job_windows` notice cases | 2 / 0 / 0 |
| `operations_native_ui` | 4 / 0 / 0 |
| Viewer image/pixel preparation | 26 / 0 / 0 |
| Initial thumbnail RED group | 12 / 2 / 0 |

The tree/CSC UI subset is **69 passed**. Its presenter cases overlap earlier CSC
gates and are not added to those counts. The two initial thumbnail failures are
preserved RED evidence; the later correction is qualified separately below.

The broad Backend `tree-recursive-green-runtime-result.json` selected 159:
158 passed, one legacy no-links assertion failed, none ignored, zero source drift.
The new recursive cases and real Windows junction fixture passed. The corrected
Delete fixture permits link browsing while preparing only the logical link,
excluding browsed descendants. The later `tree-recursive-refined-runtime-result.json`
passes **11 / 0 / 0**: ten Delete-capture cases plus the real Windows junction
fixture, with unchanged production and zero drift. This is an overlapping narrow
rerun, not another 159-test execution or 11 additional unique tests.

Per-target summaries and matching `*-source-and-command.json` captures retain
artifact/source identities and commands. [Behavior and guard limits](../developer/TREE_PANEL.md#retry-diagnostics-and-recursive-expansion)
remain in the tree contract.

## Thumbnail correction and debug build

Perf's correction is committed at `8e205c0`. The later
`thumbnail-green-runtime-result.json` records **14 passed / 0 failed / 0 ignored**,
zero source drift, completed at 23:42:42 CET on 12 November. This supersedes the
earlier thumbnail RED result within that scope.

`final-debug-build-result.json` records a normal debug build with exit 0 and zero
source drift. `_build/debug/me.exe` is **101,768,704 bytes**, SHA-256
`d2643b49917c29b8f5f3b0171dc075bb30058817de6f0b6b1212d8361c0f658d`.
The registered private entrypoint is
`C:/Users/tjelinek/MidnightExplorer-20261112-224859-d2643b49/me.exe`, with the same
SHA-256. `final-private-entrypoint-smoke-results.json` records engine
`--smoke-test` and native `--ui-smoke-test`, both exit 0 with unchanged fixtures.
`final-private-qualified-debug-result.json` records source `3619070`; Root visually
inspected `final-private-entrypoint-native.png` and found normal two-panel/toolbar
startup without an error. The previous runtime, preferences and desktop shortcut
are unchanged; candidate native Rename plugins were not deployed.

This qualifies startup, not full viewer workflows, manual CSC/UAC access or Linux.
The Linux native fixture remains unexecuted. Batch bounds and thumbnail tests
establish no broad latency or SSD-performance claim.
