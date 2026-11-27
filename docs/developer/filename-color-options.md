# Filename-color priority editor

Options → Colors shows the rule priority as a visible ordered list. The first
enabled matching rule wins; moving a rule up gives it precedence over later
matches. Disabled rules keep their position and display `[off]`. Rule expressions
retain their complete text and Unicode; the list elides only when its actual
width requires it. The selected editor preserves the complete expression.

The editor controls bind two-way to the selected rule's draft: selecting, adding
or removing a rule shows that rule, and the next edit changes only it. An edit
whose expression exceeds 4096 bytes (or whose color text exceeds 64 bytes) is not
applied: the editor shows the rule's last accepted values again and the page
reports "Edit not applied".

The list supports mouse and accessibility selection, Up/Down and Home/End. Keyboard
selection reveals the selected row in the bounded list viewport. Add, Remove and
Move up/down retain the existing bounded draft editor and its 32-rule limit. The
foreground/background controls and previews are compact. Previews use two columns
when width or font size needs more room, and each card fits its wrapped text. The
color chooser remains a separate popup. The surrounding Options page scrolls
independently of the priority list; OK and Cancel stay in the dialog footer.
The chooser's eight preset swatches keep their `RGB r, g, b` accessible labels and
name their color and hex value in their accessible description and tooltip (for
example "Red #EF4444"). Hovering one draws the 2 px active-frame ring of the page's
own swatches; keyboard focus draws its frame 2 px wide in the active color.

Selection and reordering do not persist preferences. Apply colors runs backend
validation and updates the current Options preview. OK validates pending changes
and saves the displayed order; Cancel restores the session baseline. Options reset
uses the existing color-draft cancellation and refresh path so that the displayed
list and selected editor agree with the restored preferences.

The behaviour follows the reference file manager's color settings: the first matching
rule in list order wins, and moving a rule repositions it. No source or visual assets
are copied; Midnight Explorer keeps its own style and backend validation.

`crates/midnight-ui/tests/color_priority_list.rs` exercises the real accessible
list and rendered row geometry, pointer/accessibility/keyboard selection,
reorder/toggle/add/remove, Cancel and validated OK, the last of 32 rules, and compact
page scrolling, including actual preview-text layout containment. Existing
filename-color and chooser tests cover matching/rendering and picker cancellation.
Software-rendered checks establish layout and interaction, not native font shaping.

## Default category palette

Defaults apply to files, preserving the theme's folder appearance. Matching is
case-insensitive and uses the existing backend mask matcher. These are the stored
base colors:

| Category | Base color |
| --- | --- |
| Source code and structured configuration | Sky blue `#23AAF2` |
| Videos | Purple `#8B5CF6` |
| PDF documents | Coral red `#EF5A5A` |
| Word documents and ebooks | Blue `#3B82F6` |
| Images | Pink `#EC4899` |
| Executables and installers | Orange `#F97316` |
| Spreadsheets and tabular data, including CSV | Green `#22C55E` |
| Audio | Teal `#14B8A6` |
| Archives | Yellow `#EAB308` |
| Presentations | Red `#EF4444` |
| Plain text, Markdown and README files | Blue `#3B82F6` |
| Fonts | Brown `#92400E` |

The defaults are defined in `crates/midnight-core/src/filename_colors.rs` (at most 32
rules, `MAX_FILENAME_COLOR_RULES`). Word documents (`*.doc;*.docx;*.odt;*.rtf;*.epub`) and
plain text with Markdown (`*.txt;*.md;*.markdown;*.rst;*.log;*.text;README;README.*`)
share the blue `#3B82F6`. Source code was cyan `#06A4C7` until 23 November 2026, and
blue `#3B82F6` before that.

The shared contrast policy preserves a readable base color or mixes it toward
black or white by the smallest RGB change that reaches 4.5:1 against the actual
row background. This includes alternating rows, selection, focus and inactive
focus in every theme. A focused marked item keeps its selection fill and adds a
visible cursor outline, so moving the cursor never makes its mark look cleared.
The displayed color can therefore differ from the stored base swatch.
The neutral [Slate themes](../theme-provenance.md#slate-variants-11-november-2026)
use the same policy: categories retain their semantic hues, with foregrounds
adjusted for each actual row state rather than recolored to blue, teal or violet
just because that is the active theme's accent.

### Source-code sky blue contrast per theme

`#23AAF2` is readable on the ordinary rows of every dark theme (5.13–8.10:1), which show
it unchanged; only Nord lightens it slightly on its alternate rows (`#27ACF2`). It is
too light for light themes (2.19–2.59:1), which darken it to 4.5:1 (`#1771A2`–`#1A7DB3`).
Nothing beyond the shared policy above is needed. Measured with that policy (WCAG
relative luminance) against each theme's row colors in `theme-palettes.slint`, through
the UI's `FilenameColorPresentation` (the method reproduces the earlier table for
`#06A4C7` exactly):

| Theme | Kind | Row | `#23AAF2` on row | Shown on row | Alternate row | `#23AAF2` on it | Shown on it | Lowest shown, focus and selection |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Amber Terminal | dark | `#162226` | 6.28:1 | unchanged 6.28:1 | `#1B2A2E` | 5.72:1 | unchanged 5.72:1 | 4.53:1 |
| Blue | dark | `#151C26` | 6.61:1 | unchanged 6.61:1 | `#19212D` | 6.25:1 | unchanged 6.25:1 | 4.50:1 |
| Carbon Green | dark | `#171B16` | 6.73:1 | unchanged 6.73:1 | `#1C211A` | 6.32:1 | unchanged 6.32:1 | 4.53:1 |
| Catppuccin Latte | light | `#EFF1F5` | 2.29:1 | #1874A6 4.54:1 | `#E9ECF2` | 2.19:1 | #1771A2 4.53:1 | 4.51:1 |
| Catppuccin Mocha | dark | `#1E1E2E` | 6.33:1 | unchanged 6.33:1 | `#232335` | 5.94:1 | unchanged 5.94:1 | 4.50:1 |
| Dark | dark | `#202020` | 6.29:1 | unchanged 6.29:1 | `#252525` | 5.91:1 | unchanged 5.91:1 | 4.50:1 |
| Deep Ocean | dark | `#091923` | 6.90:1 | unchanged 6.90:1 | `#10232E` | 6.22:1 | unchanged 6.22:1 | 4.51:1 |
| Dracula | dark | `#282A36` | 5.50:1 | unchanged 5.50:1 | `#2C2E3B` | 5.19:1 | unchanged 5.19:1 | 4.50:1 |
| Graphite Contrast | dark | `#1B181B` | 6.79:1 | unchanged 6.79:1 | `#221F22` | 6.30:1 | unchanged 6.30:1 | 4.72:1 |
| Gruvbox Dark | dark | `#282828` | 5.69:1 | unchanged 5.69:1 | `#2D2B29` | 5.44:1 | unchanged 5.44:1 | 4.51:1 |
| High Contrast Dark | dark | `#000000` | 8.10:1 | unchanged 8.10:1 | `#080808` | 7.73:1 | unchanged 7.73:1 | 6.29:1 |
| High Contrast Light | light | `#FFFFFF` | 2.59:1 | #1A7DB3 4.54:1 | `#FAFAFA` | 2.48:1 | #197AAE 4.54:1 | 4.50:1 |
| Light | light | `#FFFFFF` | 2.59:1 | #1A7DB3 4.54:1 | `#F6F6F6` | 2.40:1 | #1978AB 4.50:1 | 4.52:1 |
| Nord | dark | `#282E39` | 5.26:1 | unchanged 5.26:1 | `#323A48` | 4.42:1 | #27ACF2 4.51:1 | 4.50:1 |
| Norton Commander | dark | `#0000AA` | 5.13:1 | unchanged 5.13:1 | `#00009B` | 5.50:1 | unchanged 5.50:1 | 4.50:1 |
| One Dark | dark | `#282C34` | 5.40:1 | unchanged 5.40:1 | `#2C323C` | 4.98:1 | unchanged 4.98:1 | 4.50:1 |
| Paper Contrast | light | `#FDFEFE` | 2.56:1 | #1A7DB1 4.51:1 | `#F0F3F8` | 2.33:1 | #1876A8 4.50:1 | 4.51:1 |
| Rosé Pine | dark | `#191724` | 6.81:1 | unchanged 6.81:1 | `#1F1D2E` | 6.36:1 | unchanged 6.36:1 | 4.51:1 |
| Rosé Pine Dawn | light | `#FAF4ED` | 2.37:1 | #1977AA 4.51:1 | `#F4EDE8` | 2.24:1 | #1873A4 4.50:1 | 4.50:1 |
| Sandstone Contrast | light | `#FFFBF2` | 2.51:1 | #197BB0 4.52:1 | `#F7F0E4` | 2.29:1 | #1874A6 4.54:1 | 4.53:1 |
| Slate Blue | dark | `#1F2228` | 6.15:1 | unchanged 6.15:1 | `#242830` | 5.70:1 | unchanged 5.70:1 | 4.51:1 |
| Slate Teal | dark | `#1F2228` | 6.15:1 | unchanged 6.15:1 | `#242830` | 5.70:1 | unchanged 5.70:1 | 4.51:1 |
| Slate Violet | dark | `#1F2228` | 6.15:1 | unchanged 6.15:1 | `#242830` | 5.70:1 | unchanged 5.70:1 | 4.51:1 |
| Solarized Light | light | `#FDF6E3` | 2.40:1 | #1978AC 4.50:1 | `#F5EEDB` | 2.24:1 | #1873A4 4.50:1 | 4.50:1 |
| Tokyo Night | dark | `#1A1B26` | 6.60:1 | unchanged 6.60:1 | `#202131` | 6.12:1 | unchanged 6.12:1 | 4.54:1 |
| Verdant Light | light | `#FBFDF8` | 2.53:1 | #1A7CB0 4.50:1 | `#F1F6EB` | 2.36:1 | #1876A9 4.55:1 | 4.52:1 |

The last column is the lowest contrast of the shown sky blue on the active and
inactive cursor rows and on selected rows; each is adjusted to 4.5:1 or more as above.
Compared with the earlier cyan `#06A4C7`, the sky blue is lighter: dark themes show it
at a higher contrast (One Dark no longer adjusts it on its alternate rows), and light
themes darken it more. Audio keeps teal `#14B8A6`, a greener hue, and documents and
text keep the deeper blue `#3B82F6`.

### Document blue contrast per theme

`#3B82F6`, the color of Word documents and plain text/Markdown, falls a little short
of 4.5:1 on many rows, and the same shared policy adjusts it. Dark themes show it
unchanged on ordinary rows in Blue, Carbon Green, Deep Ocean, Graphite Contrast, High
Contrast Dark, Rosé Pine and Tokyo Night, and lighten it slightly elsewhere, most on
Nord, Norton Commander, One Dark and Dracula; light themes darken it (3.25–3.68:1
unadjusted). Every row shows it at 4.5:1 or more. Measured as above:

| Theme | Kind | Row | `#3B82F6` on row | Shown on row | Alternate row | `#3B82F6` on it | Shown on it | Lowest shown, focus and selection |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Amber Terminal | dark | `#162226` | 4.42:1 | #3D84F6 4.52:1 | `#1B2A2E` | 4.03:1 | #4A8CF7 4.51:1 | 4.52:1 |
| Blue | dark | `#151C26` | 4.66:1 | unchanged 4.66:1 | `#19212D` | 4.40:1 | #3E84F6 4.50:1 | 4.51:1 |
| Carbon Green | dark | `#171B16` | 4.74:1 | unchanged 4.74:1 | `#1C211A` | 4.45:1 | #3C83F6 4.50:1 | 4.52:1 |
| Catppuccin Latte | light | `#EFF1F5` | 3.25:1 | #316BCB 4.53:1 | `#E9ECF2` | 3.11:1 | #2F68C6 4.53:1 | 4.51:1 |
| Catppuccin Mocha | dark | `#1E1E2E` | 4.46:1 | #3C83F6 4.51:1 | `#232335` | 4.19:1 | #4589F6 4.52:1 | 4.52:1 |
| Dark | dark | `#202020` | 4.43:1 | #3D84F6 4.52:1 | `#252525` | 4.17:1 | #4689F6 4.50:1 | 4.51:1 |
| Deep Ocean | dark | `#091923` | 4.86:1 | unchanged 4.86:1 | `#10232E` | 4.39:1 | #3F85F6 4.53:1 | 4.50:1 |
| Dracula | dark | `#282A36` | 3.87:1 | #5090F7 4.53:1 | `#2C2E3B` | 3.66:1 | #5895F7 4.52:1 | 4.50:1 |
| Graphite Contrast | dark | `#1B181B` | 4.79:1 | unchanged 4.79:1 | `#221F22` | 4.44:1 | #3D84F6 4.53:1 | 4.50:1 |
| Gruvbox Dark | dark | `#282828` | 4.01:1 | #4B8DF7 4.53:1 | `#2D2B29` | 3.83:1 | #5291F7 4.53:1 | 4.50:1 |
| High Contrast Dark | dark | `#000000` | 5.71:1 | unchanged 5.71:1 | `#080808` | 5.45:1 | unchanged 5.45:1 | 4.52:1 |
| High Contrast Light | light | `#FFFFFF` | 3.68:1 | #3473DB 4.54:1 | `#FAFAFA` | 3.52:1 | #3371D5 4.51:1 | 4.50:1 |
| Light | light | `#FFFFFF` | 3.68:1 | #3473DB 4.54:1 | `#F6F6F6` | 3.40:1 | #326ED1 4.53:1 | 4.50:1 |
| Nord | dark | `#282E39` | 3.71:1 | #5694F7 4.53:1 | `#323A48` | 3.11:1 | #6EA3F8 4.50:1 | 4.50:1 |
| Norton Commander | dark | `#0000AA` | 3.61:1 | #5A96F7 4.52:1 | `#00009B` | 3.88:1 | #5090F7 4.53:1 | 4.51:1 |
| One Dark | dark | `#282C34` | 3.81:1 | #5291F7 4.50:1 | `#2C323C` | 3.51:1 | #5E99F8 4.54:1 | 4.50:1 |
| Paper Contrast | light | `#FDFEFE` | 3.64:1 | #3473DA 4.51:1 | `#F0F3F8` | 3.31:1 | #316CCD 4.54:1 | 4.51:1 |
| Rosé Pine | dark | `#191724` | 4.80:1 | unchanged 4.80:1 | `#1F1D2E` | 4.48:1 | #3C83F6 4.53:1 | 4.50:1 |
| Rosé Pine Dawn | light | `#FAF4ED` | 3.37:1 | #326ECF 4.51:1 | `#F4EDE8` | 3.17:1 | #306AC8 4.51:1 | 4.50:1 |
| Sandstone Contrast | light | `#FFFBF2` | 3.56:1 | #3471D7 4.53:1 | `#F7F0E4` | 3.25:1 | #316BCB 4.53:1 | 4.50:1 |
| Slate Blue | dark | `#1F2228` | 4.33:1 | #4086F6 4.52:1 | `#242830` | 4.02:1 | #4B8DF7 4.54:1 | 4.51:1 |
| Slate Teal | dark | `#1F2228` | 4.33:1 | #4086F6 4.52:1 | `#242830` | 4.02:1 | #4B8DF7 4.54:1 | 4.50:1 |
| Slate Violet | dark | `#1F2228` | 4.33:1 | #4086F6 4.52:1 | `#242830` | 4.02:1 | #4B8DF7 4.54:1 | 4.50:1 |
| Solarized Light | light | `#FDF6E3` | 3.41:1 | #326ED1 4.54:1 | `#F5EEDB` | 3.18:1 | #306AC8 4.51:1 | 4.50:1 |
| Tokyo Night | dark | `#1A1B26` | 4.65:1 | unchanged 4.65:1 | `#202131` | 4.31:1 | #4086F6 4.50:1 | 4.50:1 |
| Verdant Light | light | `#FBFDF8` | 3.59:1 | #3472D9 4.50:1 | `#F1F6EB` | 3.35:1 | #326DCF 4.53:1 | 4.51:1 |

### Saved color lists

Loading a saved document upgrades only an exact match for a historical shipped
preset: the original five-rule list, the eleven-category slate/amber list, that
list after archives became yellow, the eleven-rule palette with bright
document/text colors, the previous twelve-category palette with blue source code and
light cool neutral (`#C4CFE0`) documents and text, the short-lived
twelve-category palette in which all three were cyan, or the palette shipped until
23 November 2026 with cyan `#06A4C7` source code and blue documents and text. These
earlier presets are kept as fixed copies in
`crates/midnight-platform-common/src/settings/filename_colors.rs`, so a later change
of the defaults cannot alter what they match. Customizations retain their order,
disabled state, matching options, colors, backgrounds and explicit removal of all
rules, except for two narrow rule-by-rule upgrades: the legacy-document split
described below, and Word document and plain-text rules still exactly as the interim
palette set them (masks, the default expression, case-insensitive, files, no
background, foreground `#06A4C7`), which turn blue `#3B82F6`, disabled ones
included. A saved list that differs from the earlier twelve-category palette in any
way therefore keeps blue source code and light cool neutral documents and text; one
customized on the interim build keeps its source-code color, and its document and
text rules turn blue unless their color, mask, mode, case, target or background was
changed. A saved list that differs from the 23 November palette in any way keeps its
cyan source code. Unrelated customized rules remain unchanged.
New and legacy documents without a stored color list receive the new defaults.
Settings files store the whole color list once any newer setting, such as the
window placement, is saved, so this exact-preset match, not the settings schema,
decides which saved lists follow the new defaults. A list set in Options to exactly
one of these presets cannot be told from an unchanged default list and moves to the
current defaults on its next load, as it did for the earlier presets. No preference
schema or plugin API changes are needed.
`README` and `README.*` match the plain-text preset; earlier source rules retain
precedence, so `README.rs` stays a source-code name.

The platform-common filename-color tests check actual mask matching for every
category and preservation of customized preferences through encode/decode,
including the move of the three unchanged twelve-category palettes to the current
defaults, the preservation of customizations of the earliest and the latest of them
(a cyan source rule included), and the
rule-by-rule move of interim cyan documents and text to blue with its negative
cases. The UI filename-color tests inspect actual rendered glyph pixels and the
contrast of every category, the blue documents and text included, across every
theme, alternating rows and all active/inactive focus/selection combinations, and
that dark themes show the source-code sky blue unchanged on ordinary rows.

PDF uses its own rule before other documents. Complete unchanged legacy presets
still migrate to the twelve-rule palette. In an otherwise customized list, only
the exact legacy `*.pdf;*.doc;*.docx;*.odt;*.rtf;*.epub` mask rule (Files,
case-insensitive, no background, foreground `#64748B` or `#C4CFE0`) splits into
a preceding PDF `#EF5A5A` rule and the remaining document rule. Position and
enabled state are preserved; unrelated rules are not reset. Standalone gray PDF
rules or changed match/target/style options remain intentional customizations.
At the full 32-rule limit no split/truncation occurs. No schema change is needed.
