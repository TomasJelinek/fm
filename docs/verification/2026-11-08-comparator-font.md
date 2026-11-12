# Comparator font verification — 2026-11-08

Implemented the [comparator font contract](../superpowers/specs/2026-11-08-comparator-font.md).
No new dependencies or plugin ABI changes.

- Windows: 2 core preference tests, 6 application settings codec tests,
  20 renderer tests, 24 comparator engine tests and 2 comparator UI tests passed.
- Linux/WSL: all 7 focused font-rendering and settings migration/round-trip tests
  passed. This does not claim a native Linux desktop interaction test.
- Strict Clippy passed for all targets of `midnight-compare-engine`,
  `midnight-compare-ui` and `midnight-viewer-render`.
- Native Windows comparator screenshot inspected: Consolas selected, fixed-pitch
  content in both panes, Unicode fallback, unobstructed font picker. Escape closed
  the window. The software-rendered interaction test also covers the 320 × 200
  layout, invalid indices, rapid choices and preference notifications.
- Debug application build and native startup smoke passed; stderr was empty and
  the user's preferences file was unchanged. Matching protected plugin runtime
  registered for `_build/debug/me.exe`. Release was not rebuilt.
- Source-size check: 1,291 handwritten files, all within 100,000 bytes.

Evidence is under `_build/qa/comparator-font/`: `ui-fonts.log`, `core-tests.log`,
`settings-tests.log`, `render-tests.log`, `linux-tests.log`, `clippy.log`,
`app-build.log`, `native/comparison-0.bmp`, and startup/deployment logs.

An initial concurrent engine run rejected two freshly created fixtures as changed
under the existing LiveValidated checks; subsequent serial and concurrent runs
passed. An initial native probe timed out; after adding source-error diagnostics,
the rerun completed successfully. Those initial observations are not proof of
their cause. No source identity checks were relaxed and neither was reproduced
in the final runs. The application build retains pre-existing unused tree-code
warnings outside this change.
