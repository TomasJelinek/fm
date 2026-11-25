# Theme palette provenance

The themes added on 8–11 November 2026 extend Midnight Explorer's semantic
palette catalog without adding a library, editor plugin, font or icon package.
The referenced community editor palettes were selected for variety;
this is not a claim about a global popularity ranking.

| Theme | Stable ID / saved name | Palette reference and license |
| --- | --- | --- |
| Tokyo Night | `9` / `tokyo-night` | [folke/tokyonight.nvim, commit cdc07ac](https://github.com/folke/tokyonight.nvim/tree/cdc07ac78467a233fd62c493de29a17e0cf2b2b6), Night and inherited Storm colors; Apache-2.0. [Retained license](licenses/themes/folke/LICENSE). |
| One Dark | `10` / `one-dark` | [joshdick/onedark.vim, commit 47bec7a](https://github.com/joshdick/onedark.vim/tree/47bec7a6196a843dad195d2666c3ac84c6e80c78), `autoload/onedark.vim`; MIT, Copyright (c) 2015 Joshua Dick. [Retained license](licenses/themes/joshdick/LICENSE). |
| Rosé Pine | `11` / `rose-pine` | [rose-pine/neovim, commit ff48305](https://github.com/rose-pine/neovim/tree/ff483051a47e27d84bdef47703538df1ed9f4a47), main variant in `lua/rose-pine/palette.lua`; MIT, Copyright (c) 2023 Rosé Pine. [Retained license](licenses/themes/rose-pine/LICENSE). |
| Norton Commander | `12` / `norton-commander` | Original semantic palette inspired by classic blue, cyan and yellow DOS file-manager screens. No upstream code, screenshot pixels, logo, font or icon is included. |
| Amber Terminal | `13` / `amber-terminal` | Original charcoal/teal terminal interpretation, warm amber focus and selection, soft cyan folders and warm white text. No external code or artwork is included. |
| Verdant Light | `14` / `verdant-light` | Original pale green/white surfaces, graphite text, forest-green controls and restrained lime accents. No brand name, logo, code or artwork is included. |
| Catppuccin Latte | `15` / `catppuccin-latte` | [catppuccin/catppuccin, commit d09787d](https://github.com/catppuccin/catppuccin/tree/d09787dd98ca6fba08af5ef2ae94a7e09f17daca), Latte reference palette; MIT, Copyright (c) 2021 Catppuccin. [Retained license](licenses/themes/catppuccin/LICENSE). |
| Rosé Pine Dawn | `16` / `rose-pine-dawn` | Dawn variant from the same pinned Rosé Pine source and MIT license above. Warm paper surfaces, rose focus, pine folders and violet secondary accents. |
| Paper Contrast | `17` / `paper-contrast` | Original cool paper, ink-blue text and blue-gray selection. |
| Sandstone Contrast | `18` / `sandstone-contrast` | Original warm ivory, dark umber text and sand selection. |
| Deep Ocean | `19` / `deep-ocean` | Original deep navy, pale cyan accents and warm selection markers. |
| Graphite Contrast | `20` / `graphite-contrast` | Original graphite and plum surfaces, warm white text and peach accents. |
| High Contrast Dark | `21` / `high-contrast-dark` | Original black/white palette with pale cyan folders and pale yellow selection markers. |
| High Contrast Light | `22` / `high-contrast-light` | Original white/black palette with very dark semantic accents. |
| Slate Blue | `23` / `slate-blue` | Original neutral charcoal surfaces, near-white text and bright blue accents, informed by the user's visual reference. |
| Slate Teal | `24` / `slate-teal` | The same original neutral Slate surfaces with teal accents. |
| Slate Violet | `25` / `slate-violet` | The same original neutral Slate surfaces with violet accents. |

| Red Lacquer | `26` / `red-lacquer` | Original oxblood panes, cinnabar active header and gold cursor edge, inspired by a [carved lacquer scroll box](https://www.clevelandart.org/art/1969.32.b). |
| Overprint | `27` / `overprint` | Original apricot paper and blue ink, with violet focus and raspberry file marks, inspired by [Risograph overprinting](https://sites.saic.edu/servicebureau/home/services/risography-printing/). |
| Faience | `28` / `faience` | Original turquoise surfaces, lapis folder names and copper focus edge, inspired by [Egyptian faience](https://www.metmuseum.org/essays/egyptian-faience-technology-and-production). |
| Afterimage | `29` / `afterimage` | Original deep violet panes and apricot folders, inspired by [Webb's color mapping of the Pillars of Creation](https://science.nasa.gov/asset/webb/pillars-of-creation-nircam-compass-image/). |
| Last Train | `30` / `last-train` | Original amber panes with dark plum focus, inspired by [low-pressure sodium street lighting](https://www.si.edu/object/low-pressure-sodium-lamp-type-na-10%3Anmah_751238). |
| Drawing Office | `31` / `drawing-office` | Original palette. Blueprint-blue panes, chalk focus and red-pencil file marks. |
| Letterhead | `32` / `letterhead` | Original palette. Mauve letter paper with claret focus and a sage selection. |
| Field Survey | `33` / `field-survey` | Original palette. Olive chart paper, sienna focus and slate-blue marked files. |
| Mastering Room | `34` / `mastering-room` | Original palette. Walnut surfaces, mint focus and violet selection markers. |
| Titanium | `35` / `titanium` | Original palette. Silver-grey working surfaces, cobalt focus and copper markers. |
| Banker’s Lamp | `36` / `bankers-lamp` | Original palette. Emerald panes, a brass cursor and midnight-blue marked files. |
| Bindery | `37` / `bindery` | Original palette. Mulberry panes with vellum folder names, rose focus and green selection. |
| Chronometer | `38` / `chronometer` | Original palette. Olive enamel with nickel focus and copper-toned file marks. |
| Iris | `39` / `iris` | Original palette. Lilac panes with indigo folder names and a butter-yellow cursor. |
| Canary Copy | `40` / `canary-copy` | Original palette. Canary paper, indigo folders, sage focus and berry-pink marked files. |
| Pencil Lead | `41` / `pencil-lead` | Original palette. Neutral graphite with a raised silver cursor and darker marked rows. |
| Carbon Black | `42` / `carbon-black` | Original palette. Near-black surfaces with a white cursor edge and charcoal marked rows. |
| Warm Ash | `43` / `warm-ash` | Original palette. Warm charcoal with chalk lettering and a darker cursor. |
| Pewter | `44` / `pewter` | Original palette. Cool mid-grey with an ink-dark cursor and a silver edge. |
| Plum Smoke | `45` / `plum-smoke` | Original palette. Violet-tinted charcoal with a lilac cursor edge and neutral graphite file marks. |

Options groups the catalog by appearance and sorts the names within each group.
The initial choices are Blue for a dark desktop and Letterhead for a light
desktop. The saved pair follows operating-system appearance changes.

In a dark-theme choice, `dos-commander` is an alias for Amber Terminal. Saving
that choice writes `amber-terminal`. IDs 0–12 retain their original saved names.

The upstream license files and palette references were inspected at the exact
commits above. All four retained licenses permit use and distribution without mandatory fees,
subscriptions or royalties. Their license terms and attribution still apply.

The application maps selected reference colors to its own UI roles and adds or
adjusts surface, selection, border, error and foreground shades for file-manager
contrast. In particular One Dark body text is brighter than the reference editor
foreground, and Tokyo Night and Rosé Pine use stronger frames. Latte and Dawn use
darker semantic text and borders than some upstream accent colors so file names,
folders, pending sizes and controls remain readable on selected and hovered rows.
The adaptation is
in `crates/midnight-ui/ui/theme-palettes.slint`; no upstream editor implementation
is embedded. Existing native file icons keep their operating-system colors.

Release packaging includes this record and all four retained licenses under
`licenses/themes/`. The Cargo dependency inventory is unchanged by these palette
additions because no dependency or feature selection changed.

## Contrast additions, 9 November 2026

The six contrast palettes are original semantic color choices. They include no
third-party theme code, art, font or copied palette, so no new dependency, license
or distribution notice is needed. Existing palette licenses remain unchanged.
IDs 0–16 and their saved names retain their existing meanings.

`all_themes_are_selectable_and_keep_text_readable` (`crates/midnight-ui/tests/themes.rs`)
checks every palette: row text on every row background, and text, muted helper text
and the accent on window, pane, surface, header and hover backgrounds, reach the
palette's text minimum. That is 4.5:1, 7:1 for the four contrast palettes, 12:1 for
the two High Contrast palettes, and 7:1 for row text in the Slate variants. Cursor and
selection edges reach 3:1.

Paper Contrast, Sandstone Contrast, Deep Ocean and Graphite Contrast target at
least 7:1 for semantic row and control text. Both High Contrast palettes target
at least 12:1, including helper text, while their primary black/white text reaches
21:1. Selection and focus edges retain at least 3:1 against adjacent row fills.
The selected row has a wider side stripe and stronger text weight, also while a
filename search highlights its matching prefix; the cursor on a selected row adds
a one-pixel outline. The Colors page preview draws the same marks. Those shapes
preserve the distinction without requiring recognition of cyan/yellow or another
color pair.

The targets follow the W3C explanations of
[enhanced text contrast](https://www.w3.org/WAI/WCAG22/Understanding/contrast-enhanced.html),
[non-text contrast](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html),
and [use of color](https://www.w3.org/WAI/WCAG22/Understanding/use-of-color.html).
This is a semantic palette contract, not a whole-application WCAG conformance
claim. Native file artwork and Slint's built-in widget styling retain their
native light/dark behavior; user-defined filename colors have their separate
contrast policy. Font size and touch density remain independent choices.

## Slate variants, 11 November 2026

Slate Blue, Slate Teal and Slate Violet share a neutral charcoal body, darker
chrome, raised gray controls, near-white main text and readable secondary text.
Their blue, teal and violet accents distinguish focus, selection and active
controls while the base surfaces remain neutral. All three use dark native
widgets. IDs 0–22, their saved names and the Blue default remain unchanged.

The user-supplied screenshot informed the surface hierarchy and blue accent of
Slate Blue. These are original semantic color choices; no screenshot pixels,
external palette, theme code, artwork, font or icon package is copied. The teal
and violet variants use that same original base. No dependency, feature selection
or third-party license notice changes.

Filename categories retain their existing semantic hues rather than adopting the
theme accent. The shared contrast adjustment derives their displayed foreground
from the actual ordinary, alternate, selected and active/inactive focused row
background. Its 4.5:1 text target and existing selection/focus markers also apply
to the Slate variants; stored base colors and custom rules are unchanged. See the
[filename-color contract](developer/FILENAME_COLOR_OPTIONS.md#default-category-palette).

Measured contrast, rendered filename checks and build evidence are recorded in
[the Slate verification report](verification/2026-11-11-slate-themes.md).

## Text selection

Every palette defines a calm fill for selected text, an explicit color for the text
on it and a dimmer pair for an inactive window.

| Token | Use |
| --- | --- |
| `selection-background` | Fill behind selected text in fields and content views. |
| `selection-text` | Color of all selected text on that fill. |
| `inactive-selection-background` | Fill of a selection in an inactive viewer or comparator window. |
| `inactive-selection-text` | Color of selected text on the inactive fill. |

Rules, checked for every palette by
`text_selection_reads_shows_calmly_and_dims_while_inactive_in_every_palette`
(`crates/midnight-ui/tests/themes.rs`):

- Both text pairs reach the palette's text minimum: 4.5:1 (WCAG AA), 7:1 for the
  four contrast palettes and 12:1 for the two High Contrast palettes.
- The fill stands out from the page (`pane-background`) by at least 1.4:1 and, so
  that it stays calm, by at most 3:1. Dark palettes use a muted tint of their accent,
  light palettes a light one, in the manner of common editor selections. The High
  Contrast palettes instead use strong fills of at least 7:1 against the page: High
  Contrast Dark black on cyan, High Contrast Light white on deep violet, the usual
  Windows high-contrast pairs.
- The inactive fill stays visible (at least 1.15:1 against the page) but stands out
  less than the active one: a less saturated tint, gray in High Contrast.

Themed text fields, the path, command-line and panel-search inputs, the text, source
and Markdown viewers, the hex view and the text comparator's Find highlight paint
selected text in `selection-text` (`ViewerAppearance::selection_text` for the views). Every selected glyph and its underline is
painted in that color, syntax and link colors included, as in native text controls;
they return when the selection ends. The renderer still moves the color in lightness
if a fill would leave it below 4.5:1, so no selected text is ever unreadable.

| Theme | Fill / text | Text on fill | Fill on page | Inactive fill / text | Text on inactive | Inactive on page |
| --- | --- | --- | --- | --- | --- | --- |
| Afterimage | `#734674` / `#fff3ff` | 6.83:1 | 1.87:1 | `#543161` / `#faeffc` | 9.46:1 | 1.30:1 |
| Amber Terminal | `#5f4b2a` / `#fff5df` | 7.67:1 | 1.96:1 | `#403b32` / `#fff5df` | 10.26:1 | 1.46:1 |
| Banker’s Lamp | `#527e53` / `#ffffff` | 4.70:1 | 1.91:1 | `#276545` / `#fef4db` | 6.32:1 | 1.30:1 |
| Bindery | `#9c5d79` / `#fff7f1` | 4.66:1 | 1.89:1 | `#804462` / `#fff1e5` | 6.49:1 | 1.30:1 |
| Blue | `#315b86` / `#ffffff` | 7.07:1 | 2.42:1 | `#323d48` / `#ffffff` | 11.07:1 | 1.55:1 |
| Canary Copy | `#a6b170` / `#282a27` | 6.30:1 | 1.90:1 | `#d2d384` / `#282a27` | 9.23:1 | 1.30:1 |
| Carbon Black | `#424242` / `#d6d6d6` | 6.91:1 | 1.89:1 | `#292929` / `#d6d6d6` | 10.01:1 | 1.31:1 |
| Carbon Green | `#426127` / `#f5ffe9` | 6.85:1 | 2.47:1 | `#31392a` / `#f5ffe9` | 11.64:1 | 1.45:1 |
| Catppuccin Latte | `#d0beef` / `#30334a` | 7.24:1 | 1.51:1 | `#e0daea` / `#30334a` | 9.06:1 | 1.21:1 |
| Catppuccin Mocha | `#524264` / `#f5e0dc` | 7.14:1 | 1.81:1 | `#3d3942` / `#f5e0dc` | 8.90:1 | 1.45:1 |
| Chronometer | `#838765` / `#000000` | 5.62:1 | 1.89:1 | `#6a6c47` / `#f4f8ea` | 5.06:1 | 1.30:1 |
| Dark | `#264f78` / `#ffffff` | 8.50:1 | 1.92:1 | `#3a3d41` / `#ffffff` | 10.92:1 | 1.49:1 |
| Deep Ocean | `#185b60` / `#ffffff` | 7.77:1 | 2.30:1 | `#293739` / `#ffffff` | 12.35:1 | 1.45:1 |
| Dracula | `#534265` / `#ffffff` | 8.99:1 | 1.58:1 | `#46424c` / `#ffffff` | 9.79:1 | 1.45:1 |
| Drawing Office | `#51768c` / `#f8f9f2` | 4.59:1 | 1.90:1 | `#2e5c7f` / `#f2f4e7` | 6.39:1 | 1.30:1 |
| Faience | `#9ca492` / `#0c363d` | 5.05:1 | 1.90:1 | `#aacab8` / `#0c363d` | 7.36:1 | 1.30:1 |
| Field Survey | `#ad8b56` / `#1c211b` | 5.15:1 | 1.91:1 | `#c0af7a` / `#262e24` | 6.44:1 | 1.31:1 |
| Graphite Contrast | `#65485b` / `#ffffff` | 7.98:1 | 2.21:1 | `#3c3439` / `#ffffff` | 12.05:1 | 1.46:1 |
| Gruvbox Dark | `#504945` / `#fbf1c7` | 7.78:1 | 1.67:1 | `#46403d` / `#fbf1c7` | 8.98:1 | 1.45:1 |
| High Contrast Dark | `#1aebff` / `#000000` | 14.37:1 | 14.37:1 | `#c8c8c8` / `#000000` | 12.55:1 | 12.55:1 |
| High Contrast Light | `#37006e` / `#ffffff` | 15.13:1 | 15.13:1 | `#c8c8c8` / `#000000` | 12.55:1 | 1.67:1 |
| Iris | `#9f86bb` / `#211e2a` | 5.14:1 | 1.89:1 | `#bca6d9` / `#2f2940` | 6.35:1 | 1.30:1 |
| Last Train | `#bb8e55` / `#2f231d` | 5.16:1 | 1.90:1 | `#ddaf5d` / `#2f231d` | 7.52:1 | 1.30:1 |
| Letterhead | `#b89baa` / `#2c242c` | 5.95:1 | 1.90:1 | `#d3c0cb` / `#2c242c` | 8.72:1 | 1.29:1 |
| Light | `#a5d0ff` / `#202020` | 10.14:1 | 1.61:1 | `#e5ebf1` / `#202020` | 13.57:1 | 1.20:1 |
| Mastering Room | `#5e6e50` / `#edebe1` | 4.60:1 | 1.89:1 | `#56513a` / `#edebe1` | 6.67:1 | 1.30:1 |
| Nord | `#465a70` / `#eceff4` | 6.16:1 | 1.92:1 | `#484d54` / `#eceff4` | 7.39:1 | 1.60:1 |
| Norton Commander | `#00737d` / `#ffffff` | 5.60:1 | 2.37:1 | `#0a4a8a` / `#ffffff` | 8.90:1 | 1.49:1 |
| One Dark | `#36546e` / `#edf1f7` | 6.99:1 | 1.77:1 | `#3d464d` / `#edf1f7` | 8.49:1 | 1.45:1 |
| Overprint | `#d09ca7` / `#28214c` | 6.37:1 | 1.91:1 | `#ebc4bf` / `#28214c` | 9.33:1 | 1.30:1 |
| Paper Contrast | `#a6c9f2` / `#0c1727` | 10.50:1 | 1.70:1 | `#dce6f3` / `#0c1727` | 14.27:1 | 1.25:1 |
| Pencil Lead | `#5b5b5b` / `#e4e4e4` | 5.34:1 | 1.89:1 | `#434343` / `#e4e4e4` | 7.78:1 | 1.30:1 |
| Pewter | `#7c8285` / `#000000` | 5.39:1 | 1.91:1 | `#62676a` / `#f1f3f4` | 5.14:1 | 1.30:1 |
| Plum Smoke | `#4d4752` / `#e3e1e4` | 6.91:1 | 1.90:1 | `#332f37` / `#e3e1e4` | 10.08:1 | 1.30:1 |
| Red Lacquer | `#864a3a` / `#fff8eb` | 6.51:1 | 2.18:1 | `#5d2a27` / `#ffece1` | 10.04:1 | 1.30:1 |
| Rosé Pine | `#52425e` / `#f4efff` | 8.10:1 | 1.93:1 | `#38343c` / `#f4efff` | 10.81:1 | 1.45:1 |
| Rosé Pine Dawn | `#e9c0cb` / `#302c48` | 8.15:1 | 1.49:1 | `#eadde1` / `#302c48` | 10.08:1 | 1.21:1 |
| Sandstone Contrast | `#e4c09f` / `#1c160e` | 10.54:1 | 1.65:1 | `#ece0d7` / `#1c160e` | 13.85:1 | 1.25:1 |
| Slate Blue | `#255575` / `#ffffff` | 7.97:1 | 2.00:1 | `#323d47` / `#ffffff` | 11.09:1 | 1.44:1 |
| Slate Teal | `#1c5b51` / `#ffffff` | 7.88:1 | 2.02:1 | `#31403c` / `#ffffff` | 10.89:1 | 1.46:1 |
| Slate Violet | `#5c4874` / `#ffffff` | 7.97:1 | 2.00:1 | `#3f3a47` / `#ffffff` | 11.01:1 | 1.45:1 |
| Solarized Light | `#9ecdd2` / `#002b36` | 8.66:1 | 1.61:1 | `#d7e4e6` / `#002b36` | 11.53:1 | 1.21:1 |
| Titanium | `#7b8db0` / `#16222c` | 4.83:1 | 1.90:1 | `#9dadc1` / `#16222c` | 7.07:1 | 1.30:1 |
| Tokyo Night | `#394b70` / `#e0e6ff` | 7.01:1 | 1.97:1 | `#323842` / `#e0e6ff` | 9.51:1 | 1.45:1 |
| Verdant Light | `#badaa9` / `#172213` | 10.74:1 | 1.50:1 | `#e1ebdc` / `#172213` | 13.43:1 | 1.20:1 |
| Warm Ash | `#6c655f` / `#efeae4` | 4.79:1 | 1.90:1 | `#524d48` / `#efeae4` | 6.99:1 | 1.30:1 |

Decisions and limits:

- Midnight Explorer has no user-editable theme. The Colors page edits filename
  rules, whose selected and focused row colors are derived from each row's
  background (see the
  [filename-color contract](developer/FILENAME_COLOR_OPTIONS.md#default-category-palette));
  the selection tokens are palette constants, nothing is stored in settings.
- Viewer and comparator windows switch to the inactive pair when the window is
  deactivated and back when it is activated. The view is painted again with the
  other pair; the page is not laid out again. Focus moving inside an active window,
  for example to the Find bar, keeps the active pair, so a Find match stays
  prominent.
- Fields keep the active pair while their window is inactive. Slint keeps a field's
  selection only across window deactivation, and fields are small; dimming them is
  not implemented.
- Standard Slint inputs that are not themed (`HintedLineEdit`, also inside
  `HistoryLineEdit`, and the plain `LineEdit`, `TextEdit` and `SpinBox` in Options
  pages, Find masks and content, the panel filter's expression, About, keyboard,
  plugin, archive and custom command forms) keep Slint Fluent's own
  selection: the system accent fill with Fluent's text color, about 4.5:1. Slint
  1.18 exposes no selection colors for them; reaching the theme's pair, and 12:1 in
  High Contrast, requires replacing them with `ThemedLineEdit`.
- The HTML viewer shows author pages with their own styles and Chromium's selection.
  The host injects no appearance into pages, so it sets no `::selection` rule;
  one would change author styling and the web viewer API.
- PDF pages are rendered images without a text layer: there is no PDF text
  selection to color.
- A selection in a wrapped Markdown paragraph is one band per line with no gaps
  between lines (`a_selected_wrapped_paragraph_has_no_gaps_between_its_lines` in
  `crates/midnight-viewer-render/tests/selection_contrast.rs`). Table cells and
  separate blocks (paragraphs, list items) keep their own bands, as in browsers.

## Syntax colors

Viewer syntax and link colors are not palette tokens: the host derives them from
fixed role tints and the theme accent, moved toward black or white only as far as
needed against the page. They reach 5:1, a margin over WCAG AA, and 7:1 in the
Contrast and High Contrast themes, recognized by body text of 15:1 or more
(`midnight_core::content_minimum`). Keyword colors use the theme accent. Every color
keeps its hue, so each theme keeps its character. See
[source highlighting](developer/VIEWER_PRESENTATION.md#source-highlighting-and-dense-pages).
