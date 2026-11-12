# Panel controls and shared notices — 2026-11-12

## Behavior

- The view and sort dropdowns obtain shortcut labels from the live action registry,
  including user overrides and disabled bindings. Reverse order uses the active
  column's toggle binding. Labels and action identities stay unchanged.
- The drive toolbar measures its controls and shows left/right arrows only when
  they overflow the panel. Either arrow activates its owning panel, scrolls within
  the valid extent, and disables at its boundary. Resizing clamps the offset;
  fitting strips do not reserve arrow space.
- Tab body tooltips show only the full displayed path and can grow to 1,040
  logical pixels within the available cursor-side space. Active and parked archives
  retain their member path; network locations use their displayed address. Native
  navigation paths stay separate from display strings.
- Error notices have one themed icon, heading and separator, regardless of native
  or embedded hosting. Copy details and Close remain separate from scrollable text.
- Operation progress explicitly measures both its details and its cancellation
  question inside the scrolling viewport. A compact window can scroll the graph
  and diagnostics while its actions remain accessible.

## Verification

Targeted integration checks: `main_menu`, `drive_toolbar_scroll`, `compact_controls`,
`panel_view_modes`, `dialog_theme`, `tool_reflow`, and `native_operation_windows`.
The previous GitHub run failed on Windows and Linux because the compact progress
content did not scroll. The existing `tool_reflow` assertion is retained unchanged.
Windows coherent affected gate: **1,844 passed, 44 explicitly ignored, 30 suites**.
This includes all 934 UI library cases, the actual timed tooltip popup and the
active/parked archive path producer regression. The corrected command-list drag
fixture preserves the assertion that a drop over the clipped footer does not
reorder staged commands. Strict affected-package Clippy passed; its two existing
`femtovg` dependency warnings are outside `--no-deps` scope. Formatting, whitespace
and the 100,000-byte source-file limit passed.

Evidence: `_build/qa/find-metadata-scroll-20261112/windows-affected.log`,
`windows-clippy.log`, `format.log`, and `ui-render/`. Compact command settings,
separator settings and the single-heading themed notice screenshots were inspected.
Linux tooltip and compact-window integration cases also passed; see the feature
reports for their targeted scope.

The fresh debug build passed backend, native-rendered and software-rendered startup
checks with exit zero, no stderr and unchanged user preferences. Both main-window
screenshots show populated panels and the bottom command/button rows. Evidence:
`final-smoke-*.log`, `final-native.png` and `final-software.png` in the same folder.
This is startup/rendering evidence, not a frame-pacing benchmark.

Follow-up: the three actual-hover tooltip tests also pass with the wider tab
layout, including a long native path rendered on one line. Evidence: `tab-width-tests.log`.

## Final deployed debug qualification

After the navigation and HTML-input follow-ups, the final debug executable is
`C:/Users/tjelinek/MidnightExplorer-20261112-user-menu-d5cf41d7/me.exe`, SHA-256
`d5cf41d7f8a7529e08a0e4a94d3fbfd5229888fea5a81c488df49bc37386ba17`.
Backend, native-rendered and software-rendered startup checks each exit 0 with no
stderr and unchanged preferences. The final screenshots and logs are in the
`final-d5cf41d7/` subdirectory of the QA folder above. Both rendered main windows
show populated panels and the bottom controls. The first repeat refused an
already-existing snapshot path; it was rerun with fresh output filenames without
changing application behavior.

The affected all-target Windows Clippy gate and fresh debug application/helper
build passed after the last source edits. Formatting, whitespace and all 2,117
handwritten source files meet the 100,000-byte limit. Follow-up shortcut and
viewer results are recorded separately in the dated navigation/HTML reports.
Actual foreground input and remaining frame-pacing limits are documented in
[the scroll qualification](2026-11-12-foreground-scroll-resume.md). Parallel
headless test execution is documented in [its report](2026-11-12-parallel-tests.md).
