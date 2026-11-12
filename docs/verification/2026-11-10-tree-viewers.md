# Viewers from expanded panels

The viewer can open an ordinary root file while inline folder branches are
expanded. This applies to F3, Alt+F3 and the existing file order used by Space.
The new `ViewerLaunch.tree` contract retains the exact projection and focused
occurrence. The viewer worker normalizes only identity-checked root aliases before
using existing native/archive content capture; no new parser, dependency or plugin
ABI is introduced.

This is deliberately bounded to files currently present in the root listing.
Expanded descendants contain folders only. Generic descendant content, external
editing and other tree operation consumers remain separately fenced.

## Regression evidence

Before the implementation, the presenter rejected the root file and the backend
used the original listing order, including a filtered-out third file. Foreign root
allocations could reach the capture callback when the tree scope was ignored.
The focused tests failed for those behaviors before normalization was added.

Review also found that an unreadable, unrelated archive member blocked viewing a
readable file. A regression failed before correcting that behavior: unrelated
unreadable members are now omitted, while focusing one still refuses the launch.
Occurrence/root ID collisions, raw Unicode names, exact root allocations,
malformed focus/descendants, parent/loading/pending UI states, Basic preferences
and the existing external-editor fence are covered.

The Engine/Slint bridge exercises F3 and Alt+F3 for native and archive panels in
Details, Brief and Thumbnails. The first right-panel test incorrectly changed only
the window's active-pane property; a real click invokes the activation callback
before selection. The corrected fixture uses that actual callback and asserts the
active panel. Initial failing logs are retained; this correction changed no
production behavior.

## Qualification

- Windows: core 48, panel UI 703 and viewer engine 107 unit tests passed.
- Linux/WSL Ubuntu 24.04: core 48, panel UI 698 and viewer engine 107 unit tests
  passed. Four existing opt-in viewer-engine tests remain ignored on each system.
- Native and archive Engine/Slint viewer bridges pass on both systems (two tests
  each); the 17 existing filter, prefix-search, folder-size and panel bridge tests
  also pass on each system.
- Viewer UI: 41 unit tests and 17 integration tests passed on each system.
- Protected Windows installation: 26 ordinary application tests passed (15
  opt-in tests ignored by that default run). Two opt-in native plugin tests were
  then run explicitly and passed: tree-file viewing/cycling and the existing flat
  viewer/provider-removal regression. A filtered-out missing file is absent from
  the viewer cycle; folder rows are never opened for content.
- Strict scoped Clippy passed for core, panel UI, viewer engine, viewer UI and the application
  with all targets and `--no-deps`. Five existing dead-code warnings in the
  non-target engine dependency remain; this is not a strict workspace claim.
- Source-size check passed for 1,619 handwritten files; focused formatting and
  whitespace checks passed.
- Independent scoped review found no remaining production blocker.

The final debug executable is `_build/debug/me.exe`, SHA-256
`cfb3bed652e5aa91620e4383d322333272cfe05210a4e0d2462e405636579340`.
The identical binary is installed in
`C:\Users\tjelinek\MidnightExplorer-20261110-html\me.exe`, with a matching
hash-bound runtime registration. The actual build-path startup smoke exited zero
and listed five drives with 30 left-panel and 132 right-panel entries.
The HTML provider package was unchanged in this integration.

Raw logs, initial failures and corrected reruns are under `_build/qa/tree-viewers/`.
These tests do not establish Linux Chromium/GPU rendering, macOS support or full
TREE-01 completion. The HTML provider's separate
[Windows native verification](2026-11-10-html-viewer.md) retains its dated scope.

The design follows Salamander `src/viewer3.cpp`, `CM_NEXTFILE` /
`GetNextFileNameForViewer` for source-associated file navigation. Midnight Explorer
retains an immutable captured visible order instead of querying a mutable panel.
