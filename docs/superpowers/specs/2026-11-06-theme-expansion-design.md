# Eight themes and a compact command row

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
