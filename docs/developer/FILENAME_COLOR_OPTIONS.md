# Filename-color priority editor

Options → Colors shows the rule priority as a visible ordered list. The first
enabled matching rule wins; moving a rule up gives it precedence over later
matches. Disabled rules keep their position and display `[off]`. Rule expressions
retain their complete text and Unicode; the list elides only when its actual
width requires it. The selected editor preserves the complete expression.

The list supports mouse and accessibility selection, Up/Down and Home/End. Keyboard
selection reveals the selected row in the bounded list viewport. Add, Remove and
Move up/down retain the existing bounded draft editor and its 32-rule limit. The
foreground/background controls and previews are compact. Previews use two columns
when width or font size needs more room, and each card fits its wrapped text. The
color chooser remains a separate popup. The surrounding Options page scrolls
independently of the priority list; OK and Cancel stay in the dialog footer.

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
| Source code and structured configuration | Blue `#3B82F6` |
| Videos | Purple `#8B5CF6` |
| Documents, PDF, Word and ebooks | Slate `#64748B` |
| Images | Pink `#EC4899` |
| Executables and installers | Orange `#F97316` |
| Spreadsheets and tabular data, including CSV | Green `#22C55E` |
| Audio | Teal `#14B8A6` |
| Archives | Amber `#D97706` |
| Presentations | Red `#EF4444` |
| Plain text, Markdown and README files | Slate `#64748B` |
| Fonts | Brown `#92400E` |

The shared contrast policy preserves a readable base color or mixes it toward
black or white by the smallest RGB change that reaches 4.5:1 against the actual
row background. This includes alternating rows, selection, focus and inactive
focus in every theme. Existing selection backgrounds and focus edges remain in
place. The displayed color can therefore differ from the stored base swatch.

Loading a saved document upgrades only an exact match for the historical five-rule
preset. Any customization preserves the whole list, including its order, disabled
rules, matching options, background choices and explicit removal of all rules.
New and legacy documents without a stored color list receive the new defaults.
No preference schema or plugin API changes are needed.
`README` and `README.*` match the plain-text preset; earlier source rules retain
precedence, so `README.rs` remains source blue.

The platform-common filename-color tests check actual mask matching for every
category and preservation of customized preferences through encode/decode. The UI
filename-color tests inspect actual rendered glyph pixels and contrast across
every theme, alternating rows and all active/inactive focus/selection combinations.
