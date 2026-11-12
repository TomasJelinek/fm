# Compact PDF footer CI repair

Status: implemented and qualified on Windows and Linux.

The Ubuntu check in [run 43131426269](https://github.com/TomasJelinek/fm/actions/runs/43131426269)
for baseline `6f06ec45d2ca27c9d78bca88d6e2d458c87e97e5` failed the rendered
`midnight-viewer-ui::ui_polish` compact PDF regression. At 320 × 200, the
26-pixel Zoom in button began at x = 296, placing its right edge at 322.
The test retained its real geometry check and pointer-input assertions.

The footer combined six fixed-width buttons with measured page/zoom labels in
one row. Native font metrics made that row wider than the 308 pixels available
inside the window padding. This was a layout defect, not a test-coordinate fix.
`crates/midnight-viewer-ui/ui/viewer.slint` now places PDF page navigation above
zoom controls when compact. Wide PDF windows and ordinary images retain one row.
No button hit target shrinks, and the full page count remains visible.

The extended regression checks both 9999 / 10000 and the maximum supported
signed-32-bit page count, including fractional zoom. It checks both palettes at
320 × 200 and 1000 × 720, full counter accessibility, all control bounds, actual
pointer callbacks, and a useful content viewport. The existing typography
regression remains in the same targeted test executable.

Evidence is under `_build/qa/life01-retirement-20261112/`:

- `ci-ubuntu-43131426269.log`: original failing Ubuntu job log.
- `footer-linux.sh`: focused native Linux command, using the existing locked SDK
  configuration and an isolated test-owned temporary directory.
- `footer-linux-green.log`: 2 passed, 0 failed/ignored/filtered; test body 1.15 seconds.
- `footer-linux-renders/`: saved actual Linux software-rendered frames; both compact palettes
  and the wide PDF layout were visually inspected. All six buttons and the full maximum page
  counter are visible without overlap at 320 × 200.
- `footer-qualified-metadata.json`: hashes of both platform test executables and the two source files.

Linux qualification passed both tests with native Linux font discovery and no fixture font override.
Windows qualification passed both `ui_polish` tests (2 passed, 0 failed/ignored)
with the updated footer and strengthened regression. The parent ran this in the
shared feature union; evidence is
`_build/qa/life01-engine-20261112/footer-windows.log`. Existing vendored femtovg
warnings are unrelated to the footer; no new compiler warning was reported.

No GitHub workflow was restarted or created. The unrelated constructor-migration
CI audit found workspace examples already compiled by
`scripts/ci-workspace-tests.py`, with `--workspace --all-targets` Clippy and an
explicit app/helper release build in `.github/workflows/ci.yml`. No compiled
Engine constructor callers outside workspace crates were found; the old signature
survives only in a historical implementation plan.
