# Options and panel polish verification — 7 November 2026

This records focused checks of work in progress, not a deployed release or the
completion of the broader dialog/viewer tasks.

- Archive overlay: actual Slint FileEntry rendered in Blue, Dark, Light and
  Dracula, including selected rows, associated icons and thumbnails. The marker
  is a smaller muted chevron without an opaque tile. Commit `21dde1b` is pushed.
- Color picker: actual software-window pointer/keyboard test passes. Swatch
  changes require explicit acceptance, Cancel preserves the prior value, and
  reopening resets a cancelled draft. No persistence is invoked by the picker.
- Middle path elision: actual widget test passes for a long Unicode path.
  Clicking/copying the pinned root preserves its backend identity and clicking
  the ellipsis exposes the complete editor text. Covered text cannot receive
  clicks through gaps in the leading overlay.
- Wheel motion: 8 focused tests pass, including reversal, dense input, delayed
  frames, bounded residuals and exact total distance. The new single-notch
  test failed on the old first-frame jump of 14.93 logical pixels. With the
  200 ms impulse it passes the less-than-11-pixel presented-step bound at 60 Hz.
  The actual MainWindow native WGPU probe subsequently recorded 79 frames,
  39 changing offsets and a final row-aligned offset of -242. Its first presented
  offsets were -8.84, -17.32, -25.03 and -31.98 logical pixels, rather than full
  row jumps. This was an isolated synthetic-list probe at 225% DPI, not final
  deployed-release qualification or a guarantee about every display/input device.
- Native Browse backend: 3 admission/cancellation/validation tests pass on both
  Windows and Linux; Linux additionally passes 2 URI/cancellation-result tests.
  Both platform/engine strict checks passed before the final Windows close fix.
  Actual Windows Common Item Dialog cancellation initially failed despite
  `IFileDialog::Close` returning success. The exact-window normal close-message
  fix was rebuilt and the isolated native test passed in 1.317 seconds.
- Dependency inventory: regenerated and checked for both target graphs:
  490 package/version records, 458 names, 60 direct dependency records.

Full Options Save/form integration, dialog migration to independent windows,
high-DPI interaction, final strict checks and release deployment remain part of
the ongoing joint verification. Build logs and captures stay under
`_build/goal/options-polish/`.
