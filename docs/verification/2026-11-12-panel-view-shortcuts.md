# Salamander panel-view shortcuts — 2026-11-12

The default panel-view shortcuts now match Salamander: **Alt+2** selects Brief,
**Alt+3** selects Detailed, and **Alt+5** selects Thumbnails in the active pane.
The catalog stores physical `Alt+Digit2`, `Alt+Digit3` and `Alt+Digit5` chords;
menus and Options display the shorter labels. The same bindings work during
filename search and do not intercept the command editor or viewer windows.

## Reference and compatibility

The supplied Salamander checkout at `D:/experiments/salamander-main` establishes
these mappings in three places:

- `src/fileswn0.cpp:1191`: Alt without Ctrl/Shift dispatches the physical digit
  to its zero-based view-template index.
- `src/salamdr4.cpp:691`: default template indices 1, 2 and 4 are Brief, Detailed
  and Thumbnails respectively.
- `help/src/hh/salamand/environment_selview.htm:37`: the documented shortcuts are
  Alt+2, Alt+3 and Alt+5.

Only the three existing view actions change default bindings. The former
Ctrl+Shift+B/L/T defaults are removed; no other built-in action owned the new
chords, so no unrelated action needs relocation. Existing explicit overrides
remain authoritative, including a view explicitly assigned its former key or
another action explicitly assigned Alt+2/3/5. The existing added-default policy
suppresses the conflicting view default and its hint; two explicit conflicting
assignments still fail validation. Empty overrides still disable an action.
No preference-schema migration or key-dispatch algorithm change is introduced.

## Verification

The production shortcut module and actual current catalog were exercised in an
isolated rustc harness using cached production dependency types. The only module
adaptation is an absolute catalog path for the standalone source layout; test
bodies are loaded directly from the repository. No Cargo target was modified.

Before changing the catalog, all three panel-view regression tests failed for the
expected old defaults: Alt+2 was unbound, a saved Alt chord did not suppress the old
view default, and the old Ctrl+Shift+B remained active without an override. After
the three key replacements, all 24 tests in the core shortcut test file passed.
The final harness also includes the coordinator's two comparison-context and two
file-comparison tests: **28 passed, 0 failed, 0.02 seconds**. Logs are
`_build/qa/panel-view-keys-20261112/{red,green,final}.log`.

The UI tests now assert Alt+2/3/5 dispatch changes only the active pane, leaves
command editing intact, updates menu/dropdown hints, and respects live remapping
and removal. A new preferences regression covers six saved configurations across
all three chords, preserving explicit former view bindings and other actions'
existing Alt-digit assignments through encode/decode. Their coordinated Cargo
results will be recorded with the final application gate.

## Coordinated Cargo follow-up

The final Windows Cargo artifacts pass all 24 core shortcut tests, 11 main-menu UI
tests and 5 saved-shortcut preference tests, including the six-configuration view-key
roundtrip. The command-editor regression compares a text-bearing Alt-digit event with
the same event when view bindings are disabled, preserving native text handling while
asserting unchanged panel mode and retained editor focus.

The bounded headless run records each executable separately in
`_build/qa/viewer-physical-keys-20261112/parallel-test-results.json`. Foreground native
view switching is recorded separately by the scroll qualification.
