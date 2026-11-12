# Retro replacement and light theme verification, 9 November 2026

DOS Commander is removed from the picker. Amber Terminal retains its ID 13 and
accepts the old saved name as a one-way migration. Verdant Light (14), Catppuccin
Latte (15) and Rosé Pine Dawn (16) bring the catalog to 17 entries. The preceding
Tokyo Night, One Dark, Rosé Pine and Norton Commander additions are retained.
The [design](../superpowers/specs/2026-11-06-theme-expansion-design.md) and
[palette provenance](../THEME_PROVENANCE.md) describe the choices and licenses.

## Checks performed

- The new core tests first failed on the old implementation: the replacement
  name was absent and the migration could not resolve it. After implementation,
  all 5 core theme tests passed on Windows and Linux (WSL Ubuntu 24.04).
- All 4 settings integration tests passed on both platforms, including exact
  saved-name round trips, rejection of unknown names, legacy schemas and the
  old-name migration. Migrating the theme leaves other settings unchanged.
- The actual Slint Options picker/render test passed for every theme in both
  desktop and touch density: 34 selections and 68 panel/Options snapshots.
  It checks unique panel backgrounds, native light/dark widget schemes, 4.5:1
  semantic text contrast and 3:1 focus/selection/control boundary contrast.
- Two actual archive-badge render tests passed with the expanded catalog.
- All 33 presenter preference tests passed, including theme mapping, delayed
  settings load, preview cancellation, save retry and stale acknowledgements.
- Release packaging tests passed: 9 tests, 2 platform-specific skips on Windows.
  The notice test compares all four included palette licenses and the provenance
  record byte for byte with their repository sources.
- Focused Rustfmt checks passed. The repository source-size check passed for
  1,365 handwritten source files, each no larger than 100,000 bytes.
- Independent source review found no actionable defect in migration, indices,
  widget scheme selection, branding or distribution notices.

Windows software-rendered Options and panel images were inspected for all four
new choices. Amber Terminal uses amber focus with cyan folder names; Verdant
Light uses pale green surfaces and dark green controls; Latte uses cool paper
with mauve focus; Dawn uses warm paper with rose focus. The measured minimum
text contrasts across the tested row/control combinations are 5.85:1, 5.34:1,
4.91:1 and 4.96:1 respectively. Existing geometry and native icons are unchanged.

Logs and render artifacts are under `_build/qa/theme-refresh/`. Verification is
against the current working application, which also contains separately tracked
in-progress features; it is not evidence that those unrelated features are
complete. Linux coverage here is core/settings persistence, not a new native
Linux screenshot run.

## Windows build and deployment

`cargo build -p midnight-explorer --bin me --offline --locked` passed in the dev
profile. The build still reports 52 pre-existing unused/dead-code warnings in
the separately in-progress engine tree implementation; this is not a claim of
a warning-free whole-project build.

The binary SHA-256 is
`5B8714ACB08B06E380B8CA05C77B8F92FE2EDC375D11DF8DE7B7874316883F7E`.
It was installed with the existing viewer/filesystem/thumbnail packages into
`C:\Users\tjelinek\MidnightExplorer-20261109-080643-b1143a4b`, and the redirected
Desktop shortcut was updated and read back. The first shortcut publication hit
a temporary file-sharing error; retrying the same validated installer succeeded.
Both headless enumeration and native UI startup smoke exited 0. The startup
snapshot was inspected, and the saved preferences remained byte-identical.
