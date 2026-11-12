# Function-key footer and direct panel views, 11 November 2026

Verified Windows debug checkpoint based on 098d749. The parallel tree-editor,
network-bookmark and transfer-cleanup work is not included in this checkpoint.

## Behavior and cause

Ctrl+F4 already resolved to extension sorting, but FunctionButton dropped the
entire action caption when its icon and text did not fit. It now gives action text
priority over the icon, fits the font within its existing height, and retains a
key-only fallback below 56 logical pixels. Full chord/action descriptions remain
available through tooltips and accessibility.

The old footer also recognized only pure Ctrl; Shift/Alt combinations displayed
normal actions despite invoking different keyboard commands. Every physical
F1–F12 cell now uses the effective shortcut map, including all 16 modifier masks,
filename-search scope, explicit remaps and disabled bindings. Unassigned cells
are disabled. Pointer-down retains the original action and pane through modifier
release, pane switching and scrolling. The current default matrix is maintained
in ../KEYBOARD_SHORTCUTS.md.

Native modifier edges track both physical sides and clear on focus loss. Aggregate
focus snapshots without side information retain authority until updated; this
covers Wayland-style focus entry without inventing the count of held keys.
Windows synthetic focus keys reconstruct exact side ownership. AltGr remains text
input. Tests supply Windows key metadata matching Winit, so genuine Ctrl+Alt is
not accidentally simulated as AltGr.

Ctrl+Shift+B/L/T select Brief/Detailed/Thumbnails in the active file panel,
including filename search. Editor/viewer text input and explicit saved assignments
retain priority. The same effective bindings appear in Keyboard Options and F10.

Salamander references: src/salamand.rc:74 maps Ctrl+F4 to CM_ACTIVEEXT;
src/toolbar.h:293 and toolbar4.cpp:1376 describe the modifier-based function bar.
Only implemented/configured Midnight Explorer actions are exposed.

## Validation

100 distinct focused tests passed (overlapping reruns are not counted twice):

- 74 catalog/integration tests: shortcuts 23, commander_edges 9, function_bar 4,
  keyboard_options 22, main_menu 10, small_window 4, tooltips 2.
- 19 native input unit tests, including eight modifier-state regressions.
- Three logical/native UI modifier-bridge tests and four native dialog geometry
  tests selected by the native test filter.

The new tests verify the independent F1–F12 action matrix, rendered visible text
rather than hidden caption-sizing items, keyboard/mouse parity on both panes,
live remapping, search context, AltGr, both modifier sides and focus snapshots.
Existing tests exercise captured clicks after modifier release and the 320×200
layout. Ctrl+F4 software-rendered snapshots at 1340 and 1600 pixels were produced;
the 1340-pixel result and the native startup image were visually inspected.

Strict Clippy passed for the application dependency graph and selected targets.
The vendored FemtoVG crate retains its two pre-existing warnings. Formatting,
source-size and staged whitespace checks passed. No dependency was added.

Backend startup, software-rendered UI startup and native-rendered UI startup all
exited zero, with empty stderr. Smoke settings isolate the user's saved panels.
The updated application was reopened normally after these checks.

## Artifact and limits

Executable: D:/experiments/dev/FM/_build/debug/me.exe, 96,123,904 bytes.
SHA-256: 47a89e92edc2cbe2740d91ba0539f79d9c7f16b0356b0cf2ddbf67cf1164083a.

Matching protected runtime:
C:/Users/tjelinek/MidnightExplorer-20261111-footer-47a89e92.
The normal deployment helper verified existing plugin packages and registered
the new executable hash; executable/plugin trust checks were not bypassed.

Local logs/screenshots: _build/qa/function-footer-20261111/. Earlier failing logs
record harness/API corrections and are superseded by footer-final.log,
function-bar-lint-final.log, modifier-tests-final.log, native-tests-final.log,
clippy-final.log and startup outputs. Integration suites unaffected by those
final corrections are recorded in tests-final.log.

This is Windows execution evidence. Wayland focus sequences are covered by state
regressions and pinned Winit source inspection, not a native Linux desktop run.
OS-reserved key combinations may be intercepted before application dispatch.
