# Contrast theme changes, 9 November 2026

Six original palettes append stable IDs 17–22: Paper Contrast, Sandstone Contrast,
Deep Ocean, Graphite Contrast, High Contrast Dark and High Contrast Light. The
catalog now contains 23 entries. Existing IDs and the DOS-to-Amber migration are
preserved. [Palette provenance](../THEME_PROVENANCE.md) records the contrast
targets and the limits of the palette-level accessibility checks.

The top toolbar uses the semantic header background. Muted helper text in Blue,
Dark, Light, Solarized Light, Carbon Green, Tokyo Night, One Dark, Catppuccin Latte
and Rosé Pine Dawn is strengthened after checking it against the surfaces where
it appears. No dependency or feature selection changes are part of this work.

## Completed checks

- Core theme regression first failed for the absent `paper-contrast` choice and
  the old 17-entry catalog. All 6 core theme tests then passed.
- All 4 platform-common theme persistence tests passed, including all 23 saved
  names, unknown-name rejection, old schemas and DOS Commander migration.
- These Windows checks used `--offline --locked -j 2` with the isolated target
  directory `_build/theme-contrast-core`.

## Combined UI verification

The `midnight-ui` `themes` integration test now selects all 23 themes through the
actual Options picker in both desktop and touch density. It checks ordinary
text at 4.5:1, the four new contrast palettes at 7:1, and the two accessibility
palettes at 12:1. Muted helper text and selected-plus-focused edges are included.
The test samples the rendered toolbar background and the accessibility themes'
side-stripe/top-line selection markers. With `MIDNIGHT_UI_RENDER_DIR` set, it
produces 92 Options and panel snapshots for visual inspection.

The updated UI test passed through the real Options picker for all 23 themes and
both densities. The three presenter theme tests and two archive-badge regressions
also passed. The render gate produced all 92 snapshots under
`_build/qa/contrast-themes/render`; representative Paper Contrast, Deep Ocean and
both High Contrast variants were visually inspected. The toolbar uses the theme
header surface in those renders. Logs are
`_build/delete-followups/dialog-themes-green.log` and
`_build/qa/contrast-themes/preferences-green.log`.

These checks cover the specified semantic palette contrasts and selected rendered
states, not certification of every native widget or complete WCAG compliance.
