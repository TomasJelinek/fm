# Native controls and full command path verification

Historical baseline `2de9d14`: the later
[compact-command follow-up](compact-command-and-size-feedback.md) restores the
single command row, removes file-row tooltips and refines partial-size feedback.

Scope: shared drive/special-location choices, working hover tooltips, native
action/location icons with semantic fallbacks, preserved input gestures and
the full wrapped command directory. The starting point is commit `d4ef58a`.
See the [design](../superpowers/specs/2026-11-04-native-controls-and-full-path-design.md).

## Meaningful regressions

Core tests cover semantic keys, distinct aliases and original native drive
roots. Backend tests reject malformed/transparent native art, preserve RGBA,
negative-cache unavailable action/location roles and keep ordinary file/folder
fallbacks. The hotplug test holds native icon loading while two inventories
reuse the same label/ID with different Unicode roots. Discovery continues and
the latest root receives its own icon key. A full-channel test proves the watch
forwarder keeps the latest snapshot and stops when its receiver closes.

Typed desktop discovery has one coalescing worker. A gated old lookup cannot
publish over a newer volume inventory, including after output backpressure.
Volume publication and context invalidation share an exclusive generation guard.
Idle actors await messages; no idle blocking worker is introduced. Windows and
Linux implementations remain separate. Windows known-folder PIDLs and HICON/GDI
objects have scoped ownership; the native test exercises a real Home icon.

UI tests cover canonical quick-strip/dropdown parity, aliases sharing a path,
synthetic Home versus genuine volumes, remembered directories, malformed/stale
tokens and authoritative hotplug removal/replacement. Separate Windows/Unix
fixtures exercise distinct raw paths with identical lossy displayed text.

Actual pointer tests deliver volume metadata and IconLoaded between button-down
and button-up. Unchanged opaque keys now update existing VecModel rows in place;
structural changes replace the model. This fixes the lost-click regression.
Timed hover tests prove drive/dropdown/header/file-row tooltips appear and
dismiss, with existing mouse activation tests retaining pass-through behavior.
Tooltip observers receive hover before custom controls accept events;
declarations hidden behind TouchArea previously never scheduled the timer for
enabled buttons.

Native RGB is rendered without tinting; unavailable art leaves meaningful
application vectors. Windows returned an empty Settings stock bitmap on this
host, so the Options vector is expected. Linux SVG-only host themes also use
application vectors because the bounded host raster loader accepts PNG.

The path matrix covers long components, combining Unicode and emoji, both
densities, ordinary/32 px fonts, active-pane changes, full-text scrolling/reset,
pane errors and important feedback. Tests assert a readable directory line,
usable command input and one complete file row at the effective minimum.
The ordinary height floor remains 480 px; large fonts derive a larger minimum
from fixed bands rather than truncating path data. A measured resize viewport
avoids a preferred/minimum layout binding loop. Important feedback is one line
with complete tooltip/accessibility text.

Independent read-only review found no unresolved P1/P2 after the gesture,
tooltip, typed-context ordering and layout fixes. Focused Windows UI checks
passed 148 unit tests and 16 interaction binaries; the final raw-native popup
fixture also passed separately. Full-workspace results follow below.

## Final workspace and native checks

Windows x64 workspace: **390 tests passed**, zero failures. Two subprocess-only
fixtures are directly ignored and exercised by their passing parent tests.
Workspace formatting, strict all-targets Clippy, application build, the
154-file/100000-byte source guard and backend smoke all passed.

Native Windows FemtoVG-WGPU smoke exited successfully. The inspected
[screenshot](../screenshots/native-controls-windows.png) uses the Unicode preview
as the active pane, proving its complete working-directory prompt exceeds the
former fixed-width field without elision. It also shows the inline drive/location
strip, colored host icons, semantic action fallbacks and compact function keys.
The captured profile is Desktop; Touch and large-font geometry are exercised by
the actual interaction/render matrix. The normal application was then opened
in the actual desktop session with the visible, responding title
**FM 0.1 — File Manager**. PID/title evidence remains local in `native-window.json`.

Linux workspace: **366 tests passed**, zero failures, followed by strict
all-targets Clippy, application build, the 154-file/100000-byte source guard and
backend smoke. Source hashes remained unchanged across the final Windows/Linux
checks. Linux UI tests use the headless software-window harness; native GPU
rendering was inspected on Windows.

Local execution logs remain in ignored `.tools/native-controls-*`,
`.tools/native-icons-*` and `.superpowers/sdd/native-controls/`. Source hashes
are compared after verification. Generated output remains under ignored
`target/` and `.tools/linux-target/`; no executable, dependency cache or temporary
evidence is part of the source commit.
