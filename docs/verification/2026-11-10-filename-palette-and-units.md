# Filename palette, binary units and sort menu, 2026-11-10

The eleven default file categories now follow the requested blue/purple/slate/
pink/orange/green/teal/amber/red/brown palette. Plain text and documents share
slate; folders retain their theme appearance. The shared contrast adjustment
targets at least 4.5:1 against the actual rendered row background, including
selection/focus and alternating rows. See the
[category and migration contract](../developer/FILENAME_COLOR_OPTIONS.md).

Only an exact match for the previous five-rule default list is migrated. User
changes, disabled rules, reordering, background choices and an intentionally
empty list are preserved. No dependency, plugin API or preference schema changes.

## Verification

- Platform-common tests exercise the real mask matcher, every category,
  extensionless README, README suffixes and customization preservation.
  `_build/qa/filename-palette/linux-green.log` and the later README regression
  logs record Linux checks; `_build/qa/size-warnings/windows-color-presets.log`
  passes all eight Windows cases.
- `midnight-ui/tests/filename_colors.rs` exercises the rendered glyph colors:
  eleven categories across 23 themes, four selection/focus states and active/
  inactive panels (2,024 combinations), including alternating backgrounds.
  All three tests in the target pass in
  `_build/qa/size-warnings/windows-ui-regressions.log`.
- Snapshots under `_build/qa/size-warnings/render/default-palette-*` include
  light, dark and accessibility palettes; the light and blue captures were
  visually inspected. These are software-rendered component tests, not physical
  monitor color calibration or all-platform desktop qualification.

## Size labels and sorting

Readable sizes already divided by powers of 1,024. Their previous KB/MB/GB/TB
labels were inaccurate and now read KiB/MiB/GiB/TiB. A MiB is 1,048,576 bytes;
an MB is 1,000,000 bytes. Exact byte mode and locale formatting are unchanged.
The shared formatter covers panel sizes, free space and operation summaries.
Existing size, volume, model and native-control assertions qualify these labels
in the Windows UI library/integration gates.

The sort picker draws a separator before Reverse order. Its selectable indices
and sort actions remain unchanged; the separator is not a selectable fake row.
The real native-controls integration target passes with the changed picker.
