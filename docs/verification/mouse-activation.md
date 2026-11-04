# Mouse navigation and file activation verification

Implementation: left click focuses its pane/row; Ctrl+click toggles multiselection;
unmodified left double click and Enter open a folder internally or a regular
file with the host association. Parent `..` remains internal. Native filesystem
links are classified on the backend; no built-in viewer or mutations were added.

Actual Slint pointer tests exposed and fixed a model-reset bug that destroyed
double-click tracking after the first click. Focus/selection updates now retain
delegates and notify only affected rows. Separate navigation-intent/displayed-
snapshot generations guard against refresh, resort, row reuse and recycled IDs.

The explicit opener has independent bounded worker/process capacity. Native
classification, launching and helper reaping never run on the UI thread. Tests
gate a slow opener and verify that navigation still completes. Linux helpers
are reaped and nonzero exit status is surfaced; queue overload fails visibly.

## Windows x64

- Workspace tests: **215 passed**, zero failures. Two existing subprocess-only
  fixtures are directly ignored and exercised by their passing parent tests.
- Strict Clippy for every target, workspace rustfmt, app build and engine smoke:
  passed. Source-size check: **89 files**, each at most 100000 bytes.
- Native FemtoVG-WGPU UI smoke exited successfully and produced the inspected
  [screenshot](../screenshots/mouse-activation-windows.png). Both panels, parent
  rows, active decoration, metadata, host icons and multilingual text render.
- Updated application launched as `NVIDIA.COM\tjelinek`, PID **22584**, title
  `FM — File Manager`; native window visible and responding at verification.

## Linux / Ubuntu WSL

- Workspace tests: **207 passed**, zero failures or ignored tests, including
  the actual Slint pointer integration suite and all Linux native opener tests.
- Strict Clippy for every target and the app build passed. The source-size check
  covered the same **89 files**. Engine smoke passed: 15 volumes, left listing
  17 entries, right listing 36 entries. No native Linux desktop application was
  launched from WSL as part of these checks.

## Test boundaries and references

Native activation tests inspect literal invocation fields/arguments and inject
launchers; they do not execute arbitrary user files. They cover Unicode,
metacharacters, native invalid text, Windows long paths/junctions, Linux
symlinks/FIFOs/sockets, absent files, and contextual association failures. Actual
application choice and association success remain dependent on the host setup.
Linux requires `/usr/bin/xdg-open` or `/usr/bin/gio`; the verified Ubuntu image
has `gio`. Windows `.lnk` uses the host association; this stage adds no separate
shortcut resolver. Path activation is not an immutable-inode guarantee against
concurrent external namespace replacement.

Salamander reference inspected: `src/fileswn9.cpp:877–900` and
`src/fileswn2.cpp:53`. No reference implementation/assets copied. Fresh read-only
review found no confirmed P1/P2 issues after the row-token hardening.

Full check logs use `.tools/mouse-windows-*` and `.tools/mouse-linux-*`;
targeted RED/GREEN evidence, review and native-window facts are under
`.superpowers/sdd/mouse-activation`. No Git repository was initialized.
