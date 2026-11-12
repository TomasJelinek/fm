# Selection scopes and live F10 shortcuts — Windows, 11 November 2026

Current defaults supersede older selection shortcut verification records:

| Action | Default | Scope |
| --- | --- | --- |
| Select files only | Ctrl+A | Replace selection with displayed top-level files |
| Select files and folders | Ctrl+Shift+A | Replace selection with displayed top-level files and folders |
| Deselect files only | Ctrl+D | Clear file marks; retain folder and directory-link marks |
| Deselect files and folders | Ctrl+Shift+D | Clear all marks |

Replacement selection clears nested marks. Parent/context rows remain
unselectable. Invert defaults are unchanged. Saved explicit mappings retain
priority over the newly introduced defaults. Editors retain native text shortcuts.

F10 already used the compiled effective shortcut map. Its new regressions verify
visible text after live remapping, removal, restoring defaults, reassignment of
a newly introduced default, primary-alias choice, and the full tooltip alias list.
Established conflicting bindings remain rejected; the tests do not bypass that
validation. The four selection entries dispatch distinct operations to the active
panel and show current keys.

## Verification

Windows MSVC debug configuration, offline SDK registry bundle, shared _build
target, unchanged application/viewer/compare feature union. Behavioral red cases
preceded the backend fix. The final combined results contain **2,041 passing tests**
and **12 existing explicitly ignored tests** across 21 library/integration targets.

Focused integration groups include core shortcut and tree policy, engine archive
selection, UI bulk keys, selection bindings, keyboard Options, panel search,
tree filters/sizes, F10 and native/archive tree selection on both panels. Full
core, engine, UI, input, compare and viewer library suites also ran.

The final repeated targets after resolving failures were:

| Target | Passed | Ignored | Execution time |
| --- | ---: | ---: | ---: |
| Engine library | 750 | 7 | 30.22 s |
| UI library | 809 | 1 | 7.49 s |
| F10 menu | 8 | 0 | 1.66 s |
| Native/archive tree selection | 8 | 0 | 5.27 s |

The streaming regression exercises the actual listing-completion adapter and its
Rebase request. Copy cannot use cursor fallback while bulk selection awaits final
reconciliation; after acknowledgement it receives the surviving folder selection.
Real-key tree tests verify root-file removal and preservation of root/nested
folder marks. Production expanded branches contain folders only; generalized
descendant-file clearing is separately checked by the core row-policy test.

Initial failures exposed two invalid fixture assumptions (a helper omitted the
listing-completion adapter; a folder-only branch was expected to show a file)
and a menu test that attempted a prohibited existing-key conflict. These fixtures
were corrected without weakening production behavior. Thirteen diagnostic tests
also reused a protected temporary directory owned by a different execution
account. A fresh process-local TEMP/TMP resolved those failures with the same
engine binary; no ACL or security policy was weakened.

Software-rendered F10 Selection was visually inspected: all six labels and
shortcut columns are aligned and visible. Local logs and optional renders are
under _build/qa/selection-menu-20261111/. This is Windows qualification; no
Linux desktop or native keyboard-driver claim is made.

Final checks: workspace rustfmt, strict Clippy on the selected libraries/tests,
diff whitespace and the 100,000-byte source-file limit passed (1,830 source files).
The vendored FemtoVG dependency still emits its two existing unused-code warnings.

A fresh debug application build passed, followed by backend, native UI and
software UI startup smoke tests. Its protected runtime contains the unchanged
independently built viewer/filesystem/thumbnail plugins and is registered for
this exact executable hash. The ordinary build-path launcher can use it.

- Executable: D:/experiments/dev/FM/_build/debug/me.exe
- SHA-256: C70619DCEC1911D6A081BF36D14DC1DF757C9FA11EF0E224C9069EB991DC2F98
- Protected runtime: C:/Users/tjelinek/MidnightExplorer-20261111-selection-c70619dc
