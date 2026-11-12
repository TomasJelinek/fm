# Panel shell integration and configurable shortcuts verification

The implementation covers captured native menu targets, clipboard copying with
SHA-256 readback, selection byte totals, panel controls, dialog layout, and
configurable application shortcuts. Viewer plugins, direct F5/F6 dialogs, replacement
of existing files, directory merging, and Move/Cut remain outside this stage.

## Backend checks

All commands use the repository Cargo wrappers or the isolated WSL verification
environment. Windows output stays under `_build`; Linux targets use `_build/linux`.
Linux native raw-path and `O_TMPFILE` fixtures use `/tmp/fm-interaction/_build/tmp`
on an ext4 filesystem, because DrvFS cannot exercise those guarantees.
The final Linux UI/app run sets `CARGO_INCREMENTAL=0` to avoid retaining another
large incremental cache. Product debug/release profiles are unchanged. An earlier
UI link attempt exhausted disk space; only regenerable `_build` caches and debug
symbols were cleaned, and that incomplete attempt is not counted as a passing run.

| Target and command scope | Passed | Intentionally ignored |
| --- | ---: | ---: |
| Windows: core, common platform, engine and Windows platform, all targets | 393 | 8 |
| Linux: core, common platform, engine and Linux platform, all targets | 370 | 5 |
| Windows: release checksum, queue, cancellation and native mutation/transfer checks | 54 | 0 |
| Linux: release checksum, queue, cancellation and native mutation/transfer checks | 61 | 0 |

Backend strict Clippy (`--all-targets -- -D warnings`) passed for these packages.
Ignored tests include benchmarks and native desktop exercises that require explicit
isolation; they are not counted as passes. The X11 clipboard cases were run separately
on an isolated Xvfb server, and the Windows context-menu smoke displayed a real native
menu. See [Linux clipboard verification](linux-file-clipboard.md).

Local logs are in `_build/qa/panel-shell/`: `locale-backend-windows-green.log`,
`locale-backend-linux-final-green.log`, `locale-backend-clippy-windows.log`,
`locale-backend-clippy-linux.log`, and the `copy-*-release-*` /
`mutation-native-release-linux.log` files.

Native Czech collation checks passed on both platforms. Linux's default run skips
the optional Czech fixture; its explicit isolated `LOCPATH` run passed separately
(`locale-sort-linux-czech-green.log`). A separate process also verified that invalid
number/time locale configuration does not disable collation.

## Regressions covered

- Selection and menu requests retain native paths and exact selection revisions.
  A sole folder's Paste targets that folder; grouped items retain their captured parent.
- Source changes, failed readback, publication races, cancellation, stale conflict
  decisions and unsupported entries cannot silently replace existing files or remove
  sources. Raw Unix bytes and unpaired Windows UTF-16 remain lossless.
- Copy and mutation work share the mutation gate. Clipboard/menu output backpressure
  stays bounded, including native menu panics and clipboard ownership servicing.
- Streamed and filtered indexes include regular-file size metadata. Unknown lengths,
  links and directories cannot become fabricated exact byte totals.
- Capacity is tied to the current navigation rather than guessed from a lexical path
  prefix across a mount or junction.
- Locale keys are prepared once on workers and reused for re-sorts; native case,
  accent and Czech contraction ordering agrees with the platform comparator.
- Mixed modified/plain clicks do not open entries. Press/release modifiers and native
  click timing are checked across Detailed, Brief and Thumbnails in both panels.
- Shortcut overrides replace old aliases, allow disabling, validate overlapping
  contexts and retain native editing/IME behavior. Settings v1–6 migrate to v7.
  Reassigning Rename to Ctrl+C or Delete to Ctrl+V still leaves search-editor
  clipboard handling intact; the action applies outside filename search.

The Linux transfer rerun exposed an invalid timing assumption in a test: two
same-length writes received identical mtime and ctime values. The destination hash
correctly matched the captured source bytes, while the source had subsequently
changed. The regression now explicitly changes mtime and asserts that change before
checking rejection; cancellation has a separate test. The preserved diagnostic is
`transfer-stamp-diagnostic-linux.log`. This does not provide a concurrent-source
snapshot guarantee; [SECURITY.md](../../SECURITY.md#verified-clipboard-copying-and-the-copy-contract)
states the precise limitation.

## UI and final integration

The UI verification additionally exercises real pointer/key events, dialogs at
desktop/touch spacing, all themes, and font sizes through 32 logical pixels.
The Windows all-target UI run passed 321 tests across 35 suites, with one explicit
performance benchmark ignored (`keyboard-ui-all-green.log`).
The rendered dialog test passed with the final body-origin, Jobs-button-height and
footer-width fixes. Visual inspection confirmed the complete copy-conflict heading
and body, normal-height Jobs actions, and footer controls inside the window at 32px.
Images are local QA artifacts under `_build/qa/panel-shell/render-final-fixed`;
`dialog-render-final-fixed.log` records the test invocation. Final Windows scope-cache
and keyboard integration checks passed another 280 tests (`ui-keyboard-final-windows.log`),
and strict UI/app Clippy passed (`ui-app-clippy-final-windows.log`).

Native Windows WGPU startup failed with `Parent device is lost` both inside and
outside the sandbox. Software rendering reached the real application window.
Visual inspection then caught a smoke-mode isolation bug: startup tab persistence
still used the native settings service. The sandbox denied that save and the actual
profile hashes remained unchanged, but the screenshot contained an error dialog;
that run is not counted as a successful visual check. A regression first reproduced
native profile access, then passed with an app-local in-memory SettingsService.
All 11 application tests passed on both platforms (`app-smoke-isolation-*.log`).
The isolated Windows software-rendered screenshot showed both panels and Unicode
filenames without an error dialog; profile hashes remained unchanged. Error-dialog
generations now fail a smoke check instead of silently producing a passing image.

The Linux UI/app all-target run passed 327 tests and ignored one benchmark, but
failed one footer-render regression (`ui-app-all-targets-linux-final.log`). At 32px,
Linux glyph metrics caused the compact Help key to become only an ellipsis.
Compact captions now fit their available width while preserving layout minima,
native icon geometry and smaller requested font sizes. Windows footer/dialog
checks passed (`footer-fit-windows-green.log`); final Linux checks and release
evidence are recorded below. Added 8px fixtures exposed a raster-mask
assumption: an RGB tolerance of 40 retained one antialias pixel, while 80 retained
25 pixels forming the complete glyphs. Only the new below-11px fixtures use that
tolerance; the existing glyph-height and containment assertions remain intact.

Final Windows UI/app strict Clippy passed (`ui-app-clippy-release-final-windows.log`).
The optimized release build passed (`release-footer-final-windows.log`) and its
actual native software-rendered screenshot was inspected without error dialogs
(`verified-final-native-main.png`). The smoke exited successfully and both current
and legacy preference-file hashes stayed unchanged
(`verified-final-native-smoke-result.json`). The executable's SHA-256 is
`CBC4C24DA731E469F6A007A4857FF0CFF81CDFC9E1BDD98ED816249E0CEB71FE`.

Final formatting and whitespace checks pass. The source-size guard checks 394
handwritten files, all at or below 100,000 bytes. No files under `_build`, `target`
or `.tools` are tracked; screenshots and local logs remain ignored QA artifacts.

Before the subsequent theme/command-row extension, final Linux UI/app Clippy and
the native debug build passed (`ui-app-clippy-linux-final.log`,
`app-build-linux-final.log`). Backend smoke passed (`app-smoke-linux-final.log`).
The first native X11 run exposed a missing runtime dependency,
`libxkbcommon-x11.so`. Official Ubuntu `libxkbcommon-x11-0` and `libxcb-xkb1`
packages were unpacked into the isolated QA runtime without system installation.
The repeated native software-rendered smoke passed and its complete two-panel
image was inspected (`native-main-linux.png`, `native-smoke-linux.log`). README
and Linux CI prerequisites now include the runtime package. The new theme and
command changes receive separate follow-up verification.

## Independent review

Review found and prompted fixes for folder-menu Paste targeting, Linux shared-parent
directory creation, X11 event-flood deadlines and delayed ownership loss, unbounded
Wayland writer threads, lossy conflict basenames, stale capacity fallback, modified
double-clicks and output backpressure. The final keyboard review additionally found
search-editor clipboard chords could be stolen by reassigned file actions; a core
regression reproduced this before the contextual binding fix. Subsequent independent
source review of the final keyboard routing, portable sorting policy, engine publication,
and Windows/Linux locale adapters found no additional actionable issues. A separate
Windows FFI review prompted a tested guard against unterminated locale-name slices.
The late smoke-settings isolation and compact-caption fixes also received separate
review with no actionable findings remaining.

The current capability and durability limits are documented in
[SECURITY.md](../../SECURITY.md#verified-clipboard-copying-and-the-copy-contract).
