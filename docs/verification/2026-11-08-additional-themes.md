# Additional modern and retro themes

Scope: Tokyo Night, One Dark, Rosé Pine, Norton Commander and DOS Commander,
appended at stable IDs 9–13. Existing IDs 0–8 and Blue as the default are retained.
The shared Slint palette/name model feeds the Options picker; the backend enum
provides saved names. The viewer receives resolved appearance colors, so it does
not require a separate ID mapping. No geometry, native file icon or dependency
change is part of this theme work.

## Regression evidence

- The new core regression failed first with `missing saved theme tokyo-night`.
  After the enum/catalog change, all four core theme tests passed on Windows.
  The initial red/green check compiled the actual extracted enum with `rustc`
  while Cargo was in use; the green result was then repeated through Cargo.
- The platform settings test round-tripped every theme in both densities and
  retained the minimum required schema version. The independent fixture contains
  all fourteen literal saved names, plus prior-schema and unknown-name coverage.
- The actual Options selection/render test passed on Windows for all fourteen
  themes in both densities. It checks distinct pane colors, light/dark widget
  choice and the complete semantic contrast matrix. Debug compilation took
  3m 42s; the test itself took 10.24s. Log: `_build/qa/themes/windows-ui.log`.
- The packaging regression failed first with `Missing palette notice: folke`.
  After adding notice staging it passed. The complete Python release-script suite
  passed 34 tests with one platform skip: the Windows private-deployment hook is
  not run by Linux. Log: `_build/qa/themes/packaging-tests.log`.
- Rustfmt passed for the changed Rust test files. The repository source-size
  check passed for 1,105 files, all at or below 100,000 bytes. Focused
  `git diff --check` passed.

Windows checks use `scripts/cargo.ps1` with `CARGO_PROFILE_DEV_DEBUG=0`,
`CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_INCREMENTAL=0` and two build jobs. This work
uses debug iteration; no theme-specific optimized build was requested or run.

## Readability and rendering

The existing semantic contrast matrix covers ordinary, secondary, folder,
selected, search, pending-size and compression text against every row state;
control text against six control surfaces; error/editor-selection text; and
frame/focus/selection edges. Thresholds remain 4.5:1 for text and 3:1 for edges.
Both native light themes remain IDs 2 and 7; all five additions use dark widgets.

Preliminary source-color calculation found the following minimum row-text ratios:

| Theme | Minimum row-text contrast |
| --- | ---: |
| Tokyo Night | 4.67:1 |
| One Dark | 4.58:1 |
| Rosé Pine | 5.40:1 |
| Norton Commander | 5.97:1 |
| DOS Commander | 6.10:1 |

The Options selection test produced 56 BMPs under `_build/qa/themes/windows/`:
Options and actual file panels for fourteen palettes in both densities. All
twenty new-theme captures were visually inspected in five unscaled composite
PNGs, `theme-9-overview.png` through `theme-13-overview.png`, in the same directory.
The check confirmed the intended distinct palettes, readable selected/search
text and folder names, visible focus/frame cues, and contained Options controls.
These fixtures exercise a software-rendered application UI; they are not a claim
of a new native Windows/Linux full-application smoke run. Error-token contrast is
verified numerically rather than by a rendered error-dialog fixture in this run.

## Attribution and distribution

[Exact palette references and retained licenses](../THEME_PROVENANCE.md) document
the Apache-2.0/MIT adaptations and the original retro color mappings. Release
packaging copies the provenance record and all three exact license files to
`licenses/themes/`; the regression checks their bytes against the retained files.
No editor plugin code, product icon, font, logo or screenshot pixel is bundled.

The broader dialog, command-line, filename-color, find, checksum and user-command
render matrices now enumerate `PreferenceTheme::ALL` rather than a hard-coded
nine-theme range. These matrices are covered by the coordinated final integration
gate; a theme-only run does not claim full-workspace verification.

The named presenter `theme_tests` (three tests) and independent
`midnight-platform-common --test theme_preferences` (three tests) remain queued
for that combined gate, as does the Linux theme integration run. The Windows
theme-only Cargo slot was released to the coordinated Move API update after
the core, JSON round-trip and real Options/render checks passed.
