# Theme and Commander tools verification

This extends the [panel, shell and shortcut verification](2026-11-06-panel-shell-and-shortcuts.md).
The checkpoints describe intermediate source states. The final integration checks
below supersede their outstanding validation items only where explicitly recorded.

## Completed checks

- Windows core/common theme settings and compatibility: **105 passed, 1 ignored**;
  strict Clippy passed. All eight stable storage names round-trip; old indices stay fixed.
- Windows UI theme/command-row suite: **278 passed**, including 270 library tests,
  real Options selection for all eight themes in both densities, semantic contrast,
  footer/dialog/fonts/native-controls/Options regressions, and command-row geometry
  across 128 combinations. The height regression first failed at 34px versus22px.
- New theme text contrast is at least 4.5:1 and selection/active border contrast at
  least 3:1 against tested adjacent surfaces. Dracula, Nord, Gruvbox Dark,
  Catppuccin Mocha and Solarized Light panel renders were visually inspected.
- Native Fluent controls animate light/dark color changes for 150–200 ms. Theme
  screenshots now wait for that animation before capture; settled Options renders
  and final Linux verification remain pending at this checkpoint.
- Commander keyboard baseline: **three expected failures** reproduced absent
  Ctrl-toolbar captions, absent panel zoom, and absent Alt+Up command history.
- Metadata form baseline: **five expected failures** reproduced missing tri-state
  conversion, octal permission masks, explicit-offset timestamp validation,
  case opt-in and invalid-form rejection.
- Native metadata baseline returned Unsupported; after implementation, the
  Windows Unicode helper suite (**4 tests**) and native edits (**2 tests**) pass.
  Further native and engine regressions are part of integration verification.

QA logs and rendered artifacts are ignored under `_build/qa/panel-shell/`.
This document is a checkpoint, not a claim that the complete Commander feature
set has passed its final cross-platform review.

## Review checkpoint before the requested clean rebuild

The final Linux backend run passed 408 tests with 5 intentional skips across 39
suites, followed by strict Clippy. Windows UI all-target tests passed 360 and
ignored one; two stale test captions failed and were corrected (idle Compare
dialog uses Close, and the theme tooltip describes eight choices). These fixes
and the subsequent layout/contrast refinements require the next UI run.

Independent review found missing late metadata errors in Jobs. A focused regression
failed before the formatter included the separate bounded error list, then passed.
The fix received an independent review. A native Windows regression also reproduced
three failures for Read-only plus compression. The handle-based fix passed all 11
native metadata tests, including collisions and partial failure, and was reviewed.

At the user's explicit request, the entire `_build` directory, including these
local logs/renders and both OS build caches, was removed before a fresh release
build. The earlier results above are historical evidence; new results are recorded
after rebuilding. No build products were moved elsewhere or committed.

## Clean release and requirements checkpoint

After that deletion, `cargo build --workspace --release --locked --offline` on
Windows completed successfully in 9m57s. This baseline predates final drag-and-drop
integration. Executable: 34,736,640 bytes; SHA-256
`646B828ABF64DC159AC249FC7C13D771ADD93F8DB836260A4C0D96EFDEB98D89`.
Fresh evidence is under `_build/qa/clean-rebuild/` and `_build/qa/panel-shell/`.

- Filesystem smoke exited successfully.
- Native software-renderer UI smoke passed, produced a screenshot and left both
  current and legacy preference-file hashes unchanged.
- Default GPU and explicit `WGPU_BACKEND=vulkan` failed during device creation
  with `Parent device is lost`. The identical binary with `WGPU_BACKEND=dx12`
  passed the native UI/screenshot/preferences checks. The Direct3D screenshot
  was visually inspected, including compact controls and Unicode filenames.
- Windows startup policy was changed to prefer Direct3D while retaining explicit
  renderer/backend overrides. Final rebuilt-default validation remains pending.
- PE import inspection confirmed `VCRUNTIME140.dll` plus Windows UCRT imports.
  Runtime documentation now includes the Microsoft x64 redistributable.
- Short Direct3D UI startup memory sample: peak working set **418,058,240 bytes**
  (398.69 MiB); maximum sampled private memory **640,675,840 bytes** (611.00 MiB).
  69 samples at 25 ms, repository-root left pane and a three-entry Unicode fixture
  on the right; no large-directory or sustained-copy workload. This is not a
  constrained-RAM minimum, GPU-memory measurement or cross-platform benchmark.

[System requirements](../SYSTEM_REQUIREMENTS.md) distinguish compatibility targets,
actually checked environments, provisional runtime headroom and build requirements.
Windows host: Windows 11 Enterprise 25H2, build 26200.9711. Linux host: Ubuntu
24.04.4 WSL2, kernel 6.6.87.2, glibc 2.39. Mint, Windows 10 and macOS have not been
qualified by those checks; macOS has no adapter yet.

## Final integration regression checks

The post-rebuild broad runs each exercised 104 suites: Windows passed 880 tests
with 10 intentional skips, and Linux passed 840 with 16 skips. Each run had one
failure: the Solarized Light accent against its header measured 4.495974:1, below
the required 4.5:1. The accent was changed from `#00666d` to `#00585e`, raising that
pair to 5.487197:1. These broad logs are retained as failed-then-fixed evidence,
not relabeled as fully green runs.

The subsequent focused Windows palette/footer checks passed all five tests across
four suites. The current-source Linux library and affected integration run passed
606 tests across 16 suites, with 15 intentional skips and no failures; this includes
the actual eight-theme Options selection/render test, footer geometry, native
metadata edits and the drag/drop cancellation, ordering and operation-ID limits.
Explicit native drag/drop runs are documented separately in
[file drag/drop verification](file-drag-drop.md); their opt-in checks are not silently
counted as having run in the ordinary workspace invocation.

Windows release-profile checks passed 151 engine tests (one intentional skip) and
136 native platform tests across six suites (seven intentional skips), including
all 11 metadata tests and independent verified-copy readback. Strict workspace
Clippy with `-D warnings`, Rust formatting and the 100,000-byte handwritten-source
limit also passed. The source-size check covers 477 files.

The footer test's earlier Touch/32px failure was a capture-mask defect: two pixels
from the command row contaminated the footer-label bounds. Its mask now uses the
actual surrounding gaps; all 21 font/density/native-aspect cases pass. No product
geometry was changed to accommodate the test. The Linux copy fixture was moved to
the test runner's native POSIX temporary directory because WSL DrvFS does not
provide the required anonymous-file semantics; the product's integrity checks
were retained.

Logs are under `_build/qa/clean-rebuild/`: `windows-workspace-tests.log`,
`linux-final-tests.log`, `windows-final-palette-tests.log`,
`linux-final-affected-tests.log`, `windows-release-engine-tests.log`,
`windows-release-native-tests.log` and `windows-workspace-clippy.log.stderr`.
Footer RED/GREEN evidence is under `_build/qa/panel-shell/footer-mask-*`.

The final Windows release UI run then passed all **310 tests** across five suites,
with no failures or skips: 301 library tests, four installed drag tests, three
queue/order/history tests, footer rendering and the eight-theme Options test.
This covers the final native-handle decode refactor and exhausted-operation-ID
regression. Log: `windows-final-ui-tests.log` and its `.stderr` companion.
Linux strict workspace/all-target Clippy also completed successfully with
`-D warnings`; log: `linux-workspace-clippy.log`.

Linux release-profile checks passed **151 engine tests** (one intentional skip)
and **103 native platform tests** across three suites (14 opt-in skips). All eight
native transfer integrity tests and all three metadata tests passed. These runs
use native POSIX temporary storage and retain the normal anonymous-file,
no-overwrite, identity and checksum checks. Logs:
`linux-release-engine-tests.log` and `linux-release-native-tests.log`.

### Integrated Windows release and application icon

The final Windows workspace release build completed successfully in 8m54s. The
resulting `me.exe` is 35,555,328 bytes, SHA-256
`7FB9A3CF593F00DDF2AEEE3254340A4CEAAA8BF18DA0ACFCB3101342FE7877F4`.
Its filesystem smoke passed on the native desktop: three volumes, 24 entries in
the repository and 62 in Home. The initial sandbox invocation correctly refused
access to the user's Home directory; the same binary passed outside that boundary.
No product permission or integrity check was relaxed.

The exact release binary passed the native UI smoke with both `SLINT_BACKEND` and
`WGPU_BACKEND` unset: its Windows Direct3D default works. The screenshot was visually
inspected, including panel paths, compact bars and Unicode names. Current and
legacy preference-file hashes remained unchanged. Evidence:
`_build/qa/panel-shell/integrated-default-native-smoke-result.json`,
`integrated-default-native-main.png` and
`_build/qa/clean-rebuild/windows-final-filesystem-desktop.log`.

All nine embedded PE icon payloads were extracted as resources and SHA-256 compared
with the corresponding committed ICO frames; every frame matched. The transparent
moon artwork and PNG/ICO exports are intentional source assets. The Linux desktop
entry, icon name and XDG application ID match; actual Wayland launcher/compositor
qualification remains separate from these packaging and Windows resource checks.

### Integrated Linux build and desktop smoke

The final Linux workspace **debug** build completed in 4m16s. This is not a full
Linux release-GUI qualification; the critical engine/native suites above were
tested in release mode separately. The debug executable is 526,787,296 bytes,
SHA-256 `512a39fc7e1a49a4f45195233cc08ba1503e1425e062bbdeeb05e7f3c2a2989d`.
Its filesystem smoke passed with 15 volumes, 24 repository entries and 37 Home
entries. UI startup on isolated Xvfb `:97` passed and left current and legacy
application preferences unchanged. The renderer diagnosis below is necessary:
successful startup alone does not prove which renderer Slint actually selected.

The first GUI attempt lacked `libxkbcommon-x11.so.0` in the minimal WSL environment.
The already-documented Ubuntu runtime packages `libxkbcommon-x11-0` and
`libxcb-xkb1` were unpacked only under `_build/qa`; the unchanged executable then
started successfully. The desktop also lacked CJK/emoji fonts. QA-only official
Noto packages and a private fontconfig configuration restored CJK rendering.
The combined emoji initially remained blank even when its font was explicitly
selected. Independent diagnosis found that Xvfb had no accepted GPU adapter:
Slint fell back to its software renderer, whose vector-font path requests outline
glyphs only. The fixture was intact and a standalone cached-dependency Swash
probe correctly rasterized its exact joined bitmap glyph.

With the QA-only `SLINT_WGPU_CPU=1` override, the **unchanged** executable used
FemtoVG-WGPU with a CPU adapter and visibly rendered Czech, CJK and the joined
color emoji. The final smoke passed again with identical executable hash and
unchanged current/legacy preferences. This qualifies the tested WGPU glyph path,
not a physical GPU, performance on CPU rendering, every Unicode font technology
or a native Wayland compositor. No application/font-layout workaround was added.

Logs and screenshots: `_build/qa/clean-rebuild/linux-final-debug-build.log`,
`linux-debug-filesystem-smoke.log`, `linux-native-default*` and
`linux-native-unicode-gpu*`, `linux-emoji-gpu.png` and `emoji-swash-probe.log`.
The earlier `linux-native-unicode-full*` screenshots retain the software-fallback
diagnosis. Fedora, openSUSE Leap/Tumbleweed and SUSE Linux
Enterprise were not executed in this environment. Their support status remains
as specified in [system requirements](../SYSTEM_REQUIREMENTS.md).
