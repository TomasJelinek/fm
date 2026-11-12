# Viewer keyboard fixture after the compact PDF footer change

The Ubuntu and Windows CI failures at base `5388a97` were a stale viewport expectation in
`midnight-viewer-ui/tests/keyboard.rs`, not a collapsed document canvas. The approved
compact PDF layout has two footer rows so the page counter and zoom actions remain
visible and clickable at the minimum 320 × 200 window size.

## Reproduction and change

The unchanged `content_height > 100` assertion was first rerun on native Linux with
only a diagnostic assertion message added. It failed with a 292 × 79 canvas at
(6, 10), matching the space reserved by the second footer row. The initial narrow
build took 3m05s; the single test failed in 0.15s. Evidence is in
`_build/qa/viewer-keyboard-20261112/red.log`; the original CI failure is in
`_build/qa/task-services-20261112/ci-linux-failure.log` near line 7600.
Completed baseline run `43139104321` also failed the Windows check at the same
`keyboard.rs:63` assertion; its evidence is
`_build/qa/task-services-20261112/ci-windows-failure.log` near line 7237.

The corrected fixture checks all three compact modes: text and ordinary images
still require more than 100 pixels of content height; PDF requires more than 40,
matching the existing `ui_polish` PDF contract. Every mode retains the native size
and content-width assertions. The search spaces/Unicode, Escape-to-host, content
Space routing and final full-size canvas assertions remain unchanged. No production
Slint, scrolling, rendering or keyboard handling changed.

The separate `ui_polish` test retains its actual widget-bounds and pointer-input
checks for all six PDF footer actions, maximum page counters, fractional zoom,
dark/light palettes and compact/full window sizes.

## Qualification

Commands use Ubuntu 24.04, the existing `_build/linux` target, pinned offline SDK
configuration and the environment from
`_build/qa/interaction/linux-verification-env.sh`. The exact runners are
`_build/qa/viewer-keyboard-20261112/red.sh` and `green.sh`.

- `cargo test -p midnight-viewer-ui --test keyboard --test ui_polish`: 4/4 passed;
  build 1m18s, keyboard 0.16s and UI polish 0.78s. No ignored tests.
- `cargo clippy -p midnight-viewer-ui --test keyboard -- -D warnings`: passed in
  1m48s. Existing dependency warnings from vendored FemtoVG were emitted under
  Cargo's dependency lint cap; no fixture or production source warning was waived.
- `rustfmt --edition 2024 --check crates/midnight-viewer-ui/tests/keyboard.rs` and
  the scoped `git diff --check`: passed.

The fresh Windows build also passed both keyboard tests (0.43s) and both UI polish
tests (0.64s), with no ignored tests; compilation took 10.65s. The recorded command
output is `_build/qa/task-services-20261112/viewer-windows.log`. These are the root
agent's completed Windows checks, reviewed here without another build or test run.

Logs: `green.log`, `clippy.log` and `fmt.log` in the QA directory above. Independent
source review found no lost keyboard or geometry coverage. No broader suite or
native desktop interaction is claimed by this fixture check.
