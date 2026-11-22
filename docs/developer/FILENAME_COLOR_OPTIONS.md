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

The behavioral reference is the local Salamander source:
`src/cfgdlg.h`, `CHighlightMasks::AgreeMasks` (line 53), returns the first matching
item in ascending list order. `src/dialogs4.cpp`, `CCfgPageColors::DialogProc` /
`EDTLBN_MOVEITEM2` (line 3930), repositions the selected rule. No source or visual
assets are copied; Midnight Explorer keeps its own style and backend validation.

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
| Source code and structured configuration | Cyan `#06A4C7` |
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

Since 22 November 2026 source code is cyan `#06A4C7`, and Word documents
(`*.doc;*.docx;*.odt;*.rtf;*.epub`) and plain text with Markdown
(`*.txt;*.md;*.markdown;*.rst;*.log;*.text;README;README.*`) have the blue `#3B82F6`
that source code had before; until then the two document rules were light cool
neutral `#C4CFE0`. A build of that day (`ebf11cff`) first gave all three rules cyan;
it was then corrected to these colors. Only these three colors changed: the rules,
their order and their masks are the same.

The shared contrast policy preserves a readable base color or mixes it toward
black or white by the smallest RGB change that reaches 4.5:1 against the actual
row background. This includes alternating rows, selection, focus and inactive
focus in every theme. A focused marked item keeps its selection fill and adds a
visible cursor outline, so moving the cursor never makes its mark look cleared.
The displayed color can therefore differ from the stored base swatch.
The neutral [Slate themes](../THEME_PROVENANCE.md#slate-variants-11-november-2026)
use the same policy: categories retain their semantic hues, with foregrounds
adjusted for each actual row state rather than recolored to blue, teal or violet
just because that is the active theme's accent.

### Source-code cyan contrast per theme

`#06A4C7` is readable on the ordinary rows of every dark theme, which show it
unchanged (Nord and One Dark lighten it slightly on their alternate rows); it is too
light for light themes (2.5–2.9:1), which darken it to 4.5:1. Nothing beyond the
shared policy above is needed. Measured with that policy (WCAG relative luminance)
against each theme's row colors in `theme-palettes.slint`:

| Theme | Kind | Row | `#06A4C7` on row | Shown on row | Alternate row | `#06A4C7` on it | Shown on it | Lowest shown, focus and selection |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Amber Terminal | dark | `#162226` | 5.53:1 | unchanged 5.53:1 | `#1B2A2E` | 5.04:1 | unchanged 5.04:1 | 4.53:1 |
| Blue | dark | `#151C26` | 5.83:1 | unchanged 5.83:1 | `#19212D` | 5.51:1 | unchanged 5.51:1 | 4.50:1 |
| Carbon Green | dark | `#171B16` | 5.93:1 | unchanged 5.93:1 | `#1C211A` | 5.57:1 | unchanged 5.57:1 | 4.50:1 |
| Catppuccin Latte | light | `#EFF1F5` | 2.60:1 | #047892 4.52:1 | `#E9ECF2` | 2.48:1 | #04758E 4.50:1 | 4.50:1 |
| Catppuccin Mocha | dark | `#1E1E2E` | 5.58:1 | unchanged 5.58:1 | `#232335` | 5.24:1 | unchanged 5.24:1 | 4.51:1 |
| Dark | dark | `#202020` | 5.54:1 | unchanged 5.54:1 | `#252525` | 5.21:1 | unchanged 5.21:1 | 4.51:1 |
| Deep Ocean | dark | `#091923` | 6.08:1 | unchanged 6.08:1 | `#10232E` | 5.49:1 | unchanged 5.49:1 | 4.50:1 |
| Dracula | dark | `#282A36` | 4.84:1 | unchanged 4.84:1 | `#2C2E3B` | 4.58:1 | unchanged 4.58:1 | 4.51:1 |
| Graphite Contrast | dark | `#1B181B` | 5.99:1 | unchanged 5.99:1 | `#221F22` | 5.55:1 | unchanged 5.55:1 | 4.50:1 |
| Gruvbox Dark | dark | `#282828` | 5.01:1 | unchanged 5.01:1 | `#2D2B29` | 4.80:1 | unchanged 4.80:1 | 4.51:1 |
| High Contrast Dark | dark | `#000000` | 7.14:1 | unchanged 7.14:1 | `#080808` | 6.81:1 | unchanged 6.81:1 | 5.54:1 |
| High Contrast Light | light | `#FFFFFF` | 2.94:1 | #05819D 4.53:1 | `#FAFAFA` | 2.82:1 | #057E99 4.52:1 | 4.51:1 |
| Light | light | `#FFFFFF` | 2.94:1 | #05819D 4.53:1 | `#F6F6F6` | 2.72:1 | #057B96 4.54:1 | 4.51:1 |
| Nord | dark | `#282E39` | 4.64:1 | unchanged 4.64:1 | `#323A48` | 3.89:1 | #28B1CF 4.51:1 | 4.50:1 |
| Norton Commander | dark | `#0000AA` | 4.52:1 | unchanged 4.52:1 | `#00009B` | 4.85:1 | unchanged 4.85:1 | 4.50:1 |
| One Dark | dark | `#282C34` | 4.76:1 | unchanged 4.76:1 | `#2C323C` | 4.39:1 | #0DA7C9 4.54:1 | 4.50:1 |
| Paper Contrast | light | `#FDFEFE` | 2.91:1 | #05809C 4.54:1 | `#F0F3F8` | 2.64:1 | #047993 4.54:1 | 4.53:1 |
| Rosé Pine | dark | `#191724` | 6.01:1 | unchanged 6.01:1 | `#1F1D2E` | 5.61:1 | unchanged 5.61:1 | 4.50:1 |
| Rosé Pine Dawn | light | `#FAF4ED` | 2.69:1 | #047B95 4.50:1 | `#F4EDE8` | 2.54:1 | #047690 4.53:1 | 4.53:1 |
| Sandstone Contrast | light | `#FFFBF2` | 2.85:1 | #057F9B 4.50:1 | `#F7F0E4` | 2.60:1 | #047892 4.51:1 | 4.51:1 |
| Slate Blue | dark | `#1F2228` | 5.42:1 | unchanged 5.42:1 | `#242830` | 5.03:1 | unchanged 5.03:1 | 4.52:1 |
| Slate Teal | dark | `#1F2228` | 5.42:1 | unchanged 5.42:1 | `#242830` | 5.03:1 | unchanged 5.03:1 | 4.51:1 |
| Slate Violet | dark | `#1F2228` | 5.42:1 | unchanged 5.42:1 | `#242830` | 5.03:1 | unchanged 5.03:1 | 4.51:1 |
| Solarized Light | light | `#FDF6E3` | 2.73:1 | #057B96 4.55:1 | `#F5EEDB` | 2.54:1 | #047690 4.53:1 | 4.50:1 |
| Tokyo Night | dark | `#1A1B26` | 5.81:1 | unchanged 5.81:1 | `#202131` | 5.40:1 | unchanged 5.40:1 | 4.52:1 |
| Verdant Light | light | `#FBFDF8` | 2.87:1 | #057F9B 4.54:1 | `#F1F6EB` | 2.68:1 | #047A95 4.53:1 | 4.50:1 |

The last column is the lowest contrast of the shown cyan on the active and inactive
cursor rows and on selected rows; each is adjusted to 4.5:1 or more as above.
Audio keeps teal `#14B8A6`, a neighboring but greener hue.

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
document/text colors, the twelve-category palette with blue source code and
light cool neutral documents and text (shipped until 22 November 2026), or the
interim twelve-category palette in which all three were cyan. These
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
changed. Unrelated customized rules remain unchanged.
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
including the move of both unchanged twelve-category palettes to the current
defaults, the preservation of customizations of the earlier one, and the
rule-by-rule move of interim cyan documents and text to blue with its negative
cases. The UI filename-color tests inspect actual rendered glyph pixels and the
contrast of every category, the blue documents and text included, across every
theme, alternating rows and all active/inactive focus/selection combinations, and
that dark themes show the source-code cyan unchanged on ordinary rows.

PDF uses its own rule before other documents. Complete unchanged legacy presets
still migrate to the twelve-rule palette. In an otherwise customized list, only
the exact legacy `*.pdf;*.doc;*.docx;*.odt;*.rtf;*.epub` mask rule (Files,
case-insensitive, no background, foreground `#64748B` or `#C4CFE0`) splits into
a preceding PDF `#EF5A5A` rule and the remaining document rule. Position and
enabled state are preserved; unrelated rules are not reset. Standalone gray PDF
rules or changed match/target/style options remain intentional customizations.
At the full32-rule limit no split/truncation occurs. No schema change is needed.

The fresh full filename_colors suite passed 20/0/0 with strict Clippy/format0.
An earlier 12/8 run used the old library because Copy-Item retained its timestamp;
that attempt is not behavior-regression evidence. Fresh compilation of both
library and harness established the result. Descendant-style transport and new
app/native rendering qualification remain separate.
