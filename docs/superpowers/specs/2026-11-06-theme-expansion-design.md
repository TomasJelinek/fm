# Theme catalog and a compact command row

The user requests five additional readable color combinations alongside Blue,
Dark and Light, inspired by themes used in other software. The command editor row
must have the same height as the function-button toolbar below it.

## Approach

Extend the existing semantic color tokens, using one reusable palette definition
per theme. This keeps native file icons unchanged and avoids copying controls for
each skin. External theme-file loading and independent widget styles add no value
to this request and remain outside this change. The five palette choices below
are implementation choices made from the user's request to select suitable themes.

Keep existing theme identifiers and the Blue default. Append Dracula, Nord,
Gruvbox Dark, Catppuccin Mocha and Solarized Light to Options. Store stable names
through the existing backend settings service; do not perform persistence in UI
callbacks. Existing settings remain readable, and unknown theme names remain an
explicit settings error. New variants need no additional storage fields.

## Palette references

- [Dracula specification](https://draculatheme.com/spec): charcoal, purple and pink.
- [Nord palette](https://www.nordtheme.com/docs/colors-and-palettes/): cool gray and frost.
- [Gruvbox source](https://github.com/morhetz/gruvbox): warm dark surfaces and amber.
- [Catppuccin palette](https://catppuccin.com/palette/): Mocha surfaces and pastel mauve.
- [Solarized](https://ethanschoonover.com/solarized/): warm light surfaces and teal.

These are adapted palettes, not copied editor UI implementations. Main, secondary,
directory, selected and search text must remain legible on the backgrounds where
the application actually draws them. For all eight themes, test at least 4.5:1 text
contrast and 3:1 for active/selection edge cues against their adjacent surfaces.
The contrast refinement additionally requires 3:1 between ordinary control frames
and their surface, with more distinct header, selection and active-panel colors.
Selection also uses borders and existing markers, not only a color change.
Render and inspect the actual file panels and Options at both densities.

Light and Dark also distinguish folder names from regular files with blue text:
deep blue (`#174d7a`) on Light and pale blue (`#a8d5ff`) on Dark. Their neutral
surfaces stay unchanged. Folder text retains at least 4.5:1 contrast on every row
background; selected text and matched search prefixes retain their existing
priority, and compression continues to use its separate marker.

## Command geometry

The command bar and its editor share `Theme.bottom-height` with the footer.
Remove the extra vertical padding and override the shell/history trigger heights
within this row. Reuse the compact function-button behavior for Run where native
button minimum heights would otherwise enlarge the row. Keep the full path and
`>` prefix, single-line input, history, shell selection, command execution and
keyboard focus behavior. Respect requested font sizes and touch spacing.

The top action row uses the same vertical padding as the path row, so its gap to
the drive row matches the drive-to-path gap. An overflowing breadcrumb strip
initially shows its end with a leading ellipsis. Resizing or changing paths/fonts
repositions it to that suffix. Clicking the ellipsis opens the full editable path;
panning reveals ancestors, and visible breadcrumb actions retain their original
native tokens for navigation and copying.

## Validation

Test all eight settings values and legacy IDs, presenter mapping and delayed-load
reconciliation. Verify command row/editor height equality, control containment and
actual keyboard/history behavior at desktop/touch density and font sizes 8–32.
Verify new palettes' contrast and light/dark widget scheme; render every theme.
Run focused Windows/Linux tests, Clippy, source-size/format checks, independent
review, an optimized Windows build and native smoke before commit and push.

## Additional modern and retro palettes, 8 November 2026

The nine existing IDs, including Carbon Green at 8, remain unchanged. Append
Tokyo Night (9, `tokyo-night`), One Dark (10, `one-dark`), Rosé Pine (11,
`rose-pine`), Norton Commander (12, `norton-commander`) and DOS Commander (13,
`dos-commander`). Blue remains the default. The backend persists their stable
names through the same versioned settings service; there is no new theme field.

Tokyo Night uses cool ink-blue surfaces and cyan/violet accents; One Dark uses
charcoal surfaces and blue; Rosé Pine uses muted purple surfaces, foam folders
and rose focus. Norton Commander uses classic blue/cyan/yellow. DOS Commander
is a separate black/cyan/amber interpretation. All are dark schemes for native
widgets, and all keep the existing geometry and host file icons. Ordinary file
text differs from folders; selected entries, focus, search and errors retain
separate semantic colors and existing markers/borders.

The three modern palettes are adapted from established editor themes rather than
chosen from a claimed global ranking. [Pinned palette references and retained
licenses](../../THEME_PROVENANCE.md) document the Apache-2.0/MIT terms and changes.
The retro themes use original color-token mappings with no copied code or artwork.
No new dependency, royalty or subscription is introduced.

The existing Options picker consumes the shared palette-name model. Extend real
selection/render tests to all fourteen themes in both densities and retain all
prior text/frame contrast requirements. Settings round-trip and presenter tests
must cover every enum value; unknown IDs and names remain rejected. Verify the
release copies the palette attribution and license texts. Record rendered checks
and platform results in a focused verification note under `docs/verification/`.

## Retro replacement and light palettes, 9 November 2026

Replace DOS Commander with **Amber Terminal** at ID 13 (`amber-terminal`). Its
charcoal/teal surfaces, warm amber focus and selection, soft cyan folders and warm
white text retain the terminal character with a calmer hierarchy. The removed
`dos-commander` storage name migrates to this replacement; new saves use only the
new name. It is not a second picker entry. IDs 0–12 remain unchanged.

Append **Verdant Light** (14, `verdant-light`), an original white/graphite and
forest-green palette with restrained lime accents, without NVIDIA naming or
branding. Append the established light community palettes **Catppuccin Latte**
(15, `catppuccin-latte`) and **Rosé Pine Dawn** (16, `rose-pine-dawn`), adapted for
file-manager readability rather than copied literally. Latte uses cool porcelain,
mauve focus and blue folders; Dawn uses warm paper, rose focus and pine folders.
All three use the native light widget scheme. Blue remains the application default.

The picker now contains 17 themes. The settings service, live appearance preview
and Cancel rollback use the existing shared theme catalog. Geometry, fonts, icon
sizes and host icons do not change. Check normal/selected/focused row and control
text at 4.5:1 contrast, and focus/selection/control boundaries at 3:1. Exercise the
real Options picker and render panels at both desktop and touch densities, cover
legacy-name migration and unknown-name rejection, and retain upstream MIT notices
in release packages. No new runtime or Cargo dependency is needed.
