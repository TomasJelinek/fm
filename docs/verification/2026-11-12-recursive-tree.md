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
| Thumbnail RED group | 12 / 2 / 0 |

The tree/CSC UI subset is **69 passed**. Its presenter cases overlap earlier CSC
gates and are not added to those counts. The two thumbnail failures are expected
RED evidence for the separate Perf correction; the overall receipt is not GREEN.

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

CSC authorization remains mocked; successful Windows UAC/CSC access is unproved.
The junction fixture is real Windows filesystem evidence. The Linux native fixture
has not yet executed; new EXE packaging and native app qualification remain pending.
The initial 16-result/50-ms batch bounds are not measured latency guarantees;
no broad performance or thumbnail-fix completion is claimed.
