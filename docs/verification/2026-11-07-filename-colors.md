# Filename colors and Carbon Green verification — 2026-11-07

Implemented ordered, enabled Masks/Regex color rules with case and file/folder
targets, backend validation/matching, strict version-14 persistence and late-load
merge protection. Results require the original listing `Arc`, request ID and rules
identity; sorting rematches, while selection/caret changes reuse valid styles.
Undecodable native filenames remain visible and retain their original identities,
but do not accidentally match every color rule. No dependency was added.

Rules are bounded to 32, expressions to 4,096 UTF-8 bytes, masks to 64 and each
compiled regex to the existing 2 MiB limit. The same presentation functions drive
rows and the four settings previews, with foreground contrast of at least 4.5:1
on the actual background. Carbon Green appends theme index 8 without changing old
saved identities. See the [implementation notes](../superpowers/plans/2026-11-07-filename-colors-verification.md).

## Results

All logs below are under `_build/qa/filename-colors/`. Cargo used the locked,
offline worker-SDK registry configuration. Build/debug output stayed under
`_build`; test suites used `RUST_TEST_THREADS=1`.

| Scope | Result | Log |
|---|---|---|
| Windows core, platform-common, engine | 406 passed, 2 ignored; 48 suites | `windows-backend-tests.log` |
| Linux core, platform-common, engine | 403 passed, 1 ignored; 48 suites | `linux-backend-serial-tests.log` |
| Linux core/platform-common after compact preference storage | 153 passed; 33 suites | `linux-preferences-final.log` |
| Windows complete main UI, with corrected targets rerun | 483 unique tests passed, 1 ignored; 55 suites | `windows-ui-tests.log` and reruns below |
| Linux complete main UI, with corrected target rerun | 480 unique tests passed, 1 ignored; 55 suites | `linux-ui-tests.log`, `linux-panel-refinements-final.log` |
| Windows strict Clippy: core/common/engine/UI, all targets | Passed | `windows-clippy-final.log` |
| Linux strict Clippy: core/common/engine, all targets | Passed | `linux-backend-clippy.log` |
| Linux strict Clippy: main UI, all targets | Passed | `linux-ui-clippy-final.log` |

The Windows full run recorded 479 passes and four obsolete test assertions. The
four corrected targets passed all 13 tests in `windows-visible-controls-final.log`,
`windows-navigation-bindings-final.log` and `windows-panel-refinements-final.log`.
The assertions now inspect visible caption items, keep Hot Paths in numbered menus,
and distinguish the unchanged menu glyph from enlarged drive/location artwork.
These corrections changed tests only. Targeted strict Clippy also passed
(`windows-interaction-clippy-final.log`, `windows-navigation-clippy-final.log`).
The Linux full run captured the corrected caption and navigation tests, but its
older panel-refinements binary retained the menu-icon assertion. That one corrected
target passed on rerun; no other Linux UI target failed.

Workspace Rust formatting and scoped `git diff --check` passed. The source-size
check passed for 766 handwritten files, all at most 100,000 bytes. Generated main UI Rust was
23,815,772 bytes, approximately 3.1% above the pre-feature 23,100,350-byte baseline.

## Render and interaction evidence

Real Slint rendering covers nine themes × detailed/brief/thumbnails × normal,
focused, selected and combined states. Additional contrast checks cover inactive
focus, light/dark/gray inputs, custom backgrounds and search-prefix highlighting.
The tests verify left-aligned names, combined-state top/edge cues, untouched pending
size colors, and the parent/default rows. Editor interactions exercise add, remove,
reorder, backend validation, rejection without saving and discard.

Windows artifacts are in `_build/qa/filename-colors/windows/`:
`filename-colors-theme-{0..8}-mode-{0..2}.bmp` and
`filename-colors-options.bmp`. Fresh Carbon Green and options renders were visually
inspected: preview markers align to the left, and explanatory prose is an Apply
tooltip. Linux artifacts use the corresponding `linux/` directory.

## Diagnosed test issues

At an 11px font, no glyph pixel necessarily has full ink coverage. The original
prefix assertion incorrectly required an exact opaque RGB pixel; a measured pixel
was approximately 88% coverage of the correct foreground. The corrected assertion
recognizes foreground/background alpha compositing with at least 65% coverage and
bounded rounding error. The old unreadable highlight follows a different RGB line
and still fails. Earlier failing logs are retained as
`windows-ui-before-antialias.log` and `linux-ui-before-antialias.log`.

An initial parallel Linux backend run hit existing five-second folder-size test
timeouts under build contention. The isolated test and all 20 folder-size tests
passed serially; the final full backend run also passed. No production workaround
was introduced (`linux-backend-tests.log` retains the initial failure).
