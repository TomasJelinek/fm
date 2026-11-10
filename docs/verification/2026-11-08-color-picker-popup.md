# Color picker popup

Foreground and background swatches now open one shared chooser above Options.
The HSV/RGB controls and presets occupy a scrollable body. Use color and Cancel
stay in the footer, and the header supplies X and dragging. The common dialog
frame supplies resizing and viewport bounds. The settings form no longer grows
when the chooser opens.

Use color updates the selected rule's draft; it does not save preferences. Cancel,
Escape and X discard only the chooser's tentative color and return to Options.
The outer Options Cancel still discards the complete settings session. Reopening
starts from the field's current value. Page/rule changes invalidate an open chooser.

The implementation reuses Slint and the existing picker math and backend color
validation. No dependency, platform API or plugin API changes were needed.
Salamander's `src/dialogs4.cpp` uses a separate `ChooseColor` dialog with explicit
acceptance; it was consulted as a behavior reference, without copying its code.

Evidence is under `_build/goal/color-picker-popup/`. The initial regression
recorded in `red.log` fails because the old picker expands the settings form
instead of opening a second popup. Normal-size rendered appearance was reviewed
in `snapshots/color-picker-popup.bmp`.

The final focused test gate (`scroll-tests.log`) passed all 11 tests across
`color_picker_popup`, `color_picker`, `filename_colors` and `options_transaction`.
It covers actual swatch clicks, explicit acceptance, Cancel/Escape/X, reopening,
stale choices, and the outer settings save/cancel transaction. Resize coverage
includes 320 x 200, mouse/touch density, configured font sizes 11/15/32, wheel
access to the bottom palette, and growing a chooser opened in the small window.
Normal, small desktop and large-font touch snapshots were visually reviewed.

Two Slint layout details are handled explicitly: popup preferred dimensions track
the main viewport (with the frame anchored at its origin), and a direct layout
inside ScrollView reports the complete reusable controls' content extent. Padding
keeps controls clear of overlay scrollbars.

Strict `cargo clippy --locked --offline -p midnight-ui --all-targets -- -D warnings`
passed (`clippy-final.log`). `cargo fmt --all -- --check`, `git diff --check`, and
the source-size check also passed (1166 handwritten files, each <= 100,000 bytes).

The Windows debug application rebuilt successfully (`build-verified.log`). Native
startup with the registered private viewer/filesystem/thumbnail runtime passed:
exit 0, empty stderr, and identical saved-preferences hashes before/after. This
startup check complements the rendered interaction tests; it does not substitute
for their picker assertions. Evidence: `app-acceptance.txt`, `deploy.log` and
`app-native.{out,err}.log`.

- Executable: `_build/debug/me.exe`
- SHA-256: `DA46082EAA265E2B675DB4D7169ED9D621082F85FCE22E1B12D525819A6BC03C`
- Private runtime: `C:\Users\tjelinek\MidnightExplorer-20261108-040725-abe4e8da`
