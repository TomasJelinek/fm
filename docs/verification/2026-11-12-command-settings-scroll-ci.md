# Custom Command settings scrollbar regression

The Ubuntu CI failure was an incorrect test assumption about which scrollbar
covers the form. No production layout or command behavior changed.

## Reproduction and diagnosis

Run `43155735332`, Ubuntu job `130976865531`, failed
`user_command_settings_ui::drag_cases::long_form_wheel_scroll_reveals_apply_for_a_real_pointer_click`
with 13 other tests passing. The failed assertion required 14 logical pixels
below the final form action for a horizontal scrollbar.

The local Linux reproduction retained the original 320 by 500 window, wheel
input and assertion. It failed in 0.39 s after a 6 min 08 s build. The measured
outer viewport was `(22, 124)..(298, 397)` and the final Preview action was
`(22, 353)..(284, 385)`: 12 pixels below and 14 pixels to the right. Content height
was 1248 and content offset was -975, exactly the end of a 273-pixel viewport.
The single wheel gesture had therefore already reached the end. Linux
fontconfig mapped the requested Segoe UI family to DejaVu Sans.

The rendered image shows the right vertical scrollbar and no bottom horizontal
scrollbar. `options-dialog.slint` explicitly disables horizontal scrolling and
binds content width to visible width. The pinned Slint Fluent 1.18.1 ScrollView
makes an `always-off` scrollbar invisible; its visible vertical bar covers the
rightmost 14 pixels. Full vertical visibility with a 12-pixel bottom gap is
valid here. Adding padding to satisfy the obsolete bottom exclusion would not
fix a product defect.

## Correction and preserved coverage

Only `crates/midnight-ui/tests/user_command_settings_ui/drag_cases.rs` changed.
The test now requires the entire action to clear the actual vertical scrollbar
on the right, with left containment as well. It retains the original vertical
visibility assertion, actual wheel gesture, real pointer click on Options OK,
exactly one SavePreferences command, and the saved command name. Diagnostics
on failure include the measured action and viewport rectangles. Temporary
measurement logging was removed.

## Evidence

All local evidence is under `_build/qa/settings-scroll-ci-20261112/`:

- `red.sh`, `red.log`, `red-status.txt`: original assertion reproduced (1 failed,
  13 filtered out).
- `font-match.log`: Linux font fallback observation.
- `red-renders/command-form-wheel.bmp`: rendered geometry at the scroll end.
- `green.sh`, `green.log`, `green-status.txt`: Linux final whole target, 14 passed,
  0 failed, 0 ignored and 0 filtered out in 10.84 s; build 2 min 00 s.
- `windows-green.log`: Windows final whole target, 14 passed, 0 failed and
  0 ignored in 2.61 s.

The downloaded CI log is
`_build/qa/report-arena-20261112/ci-linux-failure.log`.

Parent source review confirmed that the correction checks the real overlay
without changing wheel input, vertical visibility, pointer interaction or save
semantics. No screenshots, binaries or temporary diagnostics are committed.
