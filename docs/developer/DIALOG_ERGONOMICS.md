# Dialog ergonomics and viewer responsiveness

This document describes how dialogs, settings forms and their keyboard handling
behave, and the review rules they follow. Items stated as requirements (such as the
performance requirements below) are rules for every change, not claims about a
measurement.

## Settings and dialogs

- Initial form size follows its content. Short Rename/Create summaries, the Delete
  confirmations and informational notices use compact heights; long diagnostics grow to a
  bounded height and then scroll. User resizing is retained while the form is
  open. Viewers continue to initially cover the main window, as requested.
- Settings uses a smaller initial footprint and responsive fields rather than a
  forced wide canvas. Labels and long check-box captions wrap; narrow forms
  stack labels above inputs. Resizing must preserve field instances, selection,
  caret and unsaved draft. Lists reserve space for actual rows, with a modest
  empty state instead of a large blank list.
- A complete UI review includes 320 × 200 and normal windows, mouse/touch
  density, light/dark themes and large configured fonts. Bounds must be checked
  against ancestor scrolling clips, not just against the application window.
  Keep dismissal reachable; scroll content and overflowing secondary actions.
  Typography and button metrics must scale together, and toolbar actions must
  not expand merely to fill unused width.
- Shared hinted controls create a tooltip only when their hint has content;
  an empty hint must not produce a blank bubble. Scrolling content reserves
  space for overlaid scrollbars so rightmost buttons remain clickable.
- Attributes, Times and Case share one `Include subfolders` checkbox above the
  topic tabs, inside the scrolling body. It remains common to all topics without
  pushing the footer out of a small window with large fonts. Windows attributes
  always use one column; changing topics preserves the complete draft.
- Every application dialog and auxiliary window is resizable. Lists, preview
  areas and editors use additional space; action buttons remain accessible.
  Narrow layouts stack controls and scroll their content instead of overlapping.
  Embedded dialogs have a top-right close button with the same behavior as Cancel;
  its tooltip names Escape. Help, Jobs and the job and decision windows are framed
  by their own title bar and have none.
  Only modeless working windows move freely outside the main window; Attributes /
  Times / Case and the other modal forms stay embedded
  ([dialog policy](../superpowers/plans/2026-11-11-dialog-polish-followup.md)).
- Ordinary confirmations and settings are modal popups embedded in the main
  window: Options, Rename/Create/Delete, Copy/Move drafts, notices, Make File
  List, Panel Filter, Hot Paths, Compare directories, About and Attributes /
  Times / Case. They add no taskbar buttons or system frames. No native host is
  owner-modal. Native
  modeless hosts (Help, Jobs, job progress and operation decisions) set ownership
  before first show and retain normal title bars and resizing. Independent working
  windows (Find, Checksums, viewers and comparators) keep taskbar and Alt+Tab
  access, with the same application identity for system grouping. Help remains an
  ordinary dialog even when modeless.
- Options follows OK/Cancel: OK validates and saves the complete current form,
  then closes only after successful persistence. Cancel discards the draft and
  restores live previews, including the theme. Validation remains cancellable;
  dismissal is disabled during an admitted save. Failed saves retain the draft
  and explain the failure. Enter presses OK unless the focused control used it:
  Enter on a combo box only opens its list.
- Controls always show the staged value. An editor that stays open while its
  subject changes (the selected color rule, viewer, Hot Path, archive format, or
  values replaced by Reset to defaults) binds two-way to that value or follows
  it explicitly: native input otherwise detaches a one-way Slint binding, so the
  control would keep the previous value and write it into the next item.
- General (formerly Panel) contains Panels and Command line sections, including
  remembering and clearing command history; there is no separate Command page.
  These controls participate in the same OK/Cancel transaction, including
  restoration of cleared history.
- Copy, Move and Delete use the operation as the primary/default action. A named
  destination receives initial editing focus; a deletion confirmation focuses
  its action button. Enter confirms from the input, option or primary button;
  focused Cancel and the header close button retain their cancel behavior. Tab and
  Shift+Tab reach the header close button too, which Enter or Space presses.
  Escape also cancels. An empty named destination remains invalid; opening or
  focusing a dialog never submits it. Enter with an empty name (Create dir, New file,
  Rename, Copy, Move) keeps the dialog open, moves the keyboard to the name field and
  says "Type the <field label> first." under it, as the field's accessible description
  too, until a name is typed. The FocusScopes that only route keys (Escape, a
  dialog's shortcuts) in dialogs, the colour picker, shortcut replacement, Select by
  mask and the main menu are not Tab stops.
- The Copy/Move dialog (`OperationDialog` in its Copy/Move form, which
  `EmbeddedOperation` selects with `kind < 0`) reads top to bottom: the title;
  one line that names what is copied and ends where the destination field takes over
  ("Copy “report.pdf” (1.2 MiB) to:", "Copy 3 files (12.4 MiB) to:", "Move 2 files and
  1 folder to:", "Move 3 items from the clipboard to:"; from the counts and sizes the
  draft captured with its selection, nothing is read; a size counts files only and is
  left out where folders are counted; where the items come from is the line's tooltip,
  "From: <folder>"); the destination field, full width, with the keyboard and the
  accessible name "Destination folder"; and an **Options** group: one
  column under its heading, every control at the form's left edge. The Links box (as
  wide as its longest entry, at least 160 px) has no visible caption, since both its
  entries name links; a screen reader reads it as "Links". The check boxes follow
  ("Preserve exact modified time", offered only where it can apply, at most 64 native
  regular files, and left out rather than disabled elsewhere; "Wait for earlier
  transfers"). Spacing: 16 px padding, 6 px inside a block (the line and the field,
  a control and its note, the options' rows), 16 px between the destination and the
  options; the compact layout (under 280 px high) uses 8, 4
  and 8 px, keeps the header and the actions fixed and scrolls the rest. At most one
  note shows, a glyph and its text under the control it concerns, first of: Enter's
  request for a destination; a refusal (a destination that is not an existing folder, a
  Move from another drive to a provider's location that refuses one;
  error colours) under the destination; and what keeping links does where none are
  created (`keep_warning`, the first Enter's hold) under the Links box (a warning).
  The compact layout shows every note under the destination, near the top. Why the
  default link handling was chosen starts the Links box's tooltip (and is its
  accessible description), and what the job verifies (the draft's policy) is the Copy or
  Move button's tooltip; neither is a paragraph of the dialog. Tab goes through the
  destination, the options top to bottom, Cancel, the primary action and the header's
  close button, and wraps; Enter in the destination field, an option or the Links box
  confirms. `tests/copy_dialog_polish.rs` (software-window tests) checks every state at
  320 × 200 and 1340 × 820 in both densities, with the density's default font and
  32 px, against the UI polish gate's layout checks, plus the options' edges, the Tab
  cycle, Enter in the destination, one note at a time, the first line, and a long first
  line with a note at every width from 320 to 600 px. These tests run on Windows; the
  application window on Linux is not covered.
- The Delete confirmations (`OperationDialog` with a `question`: Move to Trash, Delete
  or F8; Permanently delete, Shift+Delete or Shift+F8) put their question first, as the
  headline under the title, in the strong text colour at 1.15 × the font, semibold:
  "Permanently delete “report.pdf”?", "Permanently delete the folder “Photos” and
  everything in it?", "Permanently delete the link “x”?", "Move 12 items to the
  Trash?". Right under it is one short line: "Permanent deletion cannot be undone." in
  the error colour, semibold, after a round "!" glyph on its first line; for the Trash
  "If the Trash is unavailable, it stays in place." ("they stay") in the secondary text
  colour, without a glyph. The rest are details, after a wider gap, in a subdued block on
  the secondary surface: a label column in the muted colour, as wide as its widest label
  (at most two fifths of the block), and the values in the secondary colour, both in the
  small font. One item lists Name, Type ("File", "Folder", "Link (what it points to is
  kept)", "Folder link (…)", "Special file"), Size, Modified and Location. Several items
  list Items ("3 files and 2 folders, including everything in the folders"), Links (when
  the sample holds one: "Only the links go; what they point to is kept"), Names (the
  first five, or all six, then "and N more"), Size and Location (the folder all sampled
  items share, when the sample is the whole selection or that folder is the panel's;
  otherwise "Several folders" or nothing). A size is the panels' readable size with the
  exact bytes, "24.1 MiB (25,320,511 bytes)" in the system's number format (bytes alone
  below 1 KiB). Several items count the files' bytes; folder contents count only when
  every folder is in the sample and Folder sizes has measured it ("At least … (folder
  sizes not final)" while one is unfinished); otherwise the size says "in the files;
  folder contents not counted", and folders alone show none. A folder has a size only
  when it was measured. Everything comes from the worker's preview (at most 32 items)
  and the sizes the panels hold; opening the confirmation reads nothing from disk.
  Names wrap, so a long name is never cut. A folder's path (Location) stays on one line
  and, when it does not fit, is shortened as the path bar shortens a panel path (see
  below): the same components (the presenter's `path_labels`, which also splits the
  bar's native paths: a drive, `/` or a UNC share is the first component, an archive
  inside a folder is a component like a folder) and the same rule (`PathCollapse`,
  which the path bar uses too), drawn read-only by `PathTrailText`. Up to two leading
  components, one "..." for the dropped middle ones, then the longest run of whole
  trailing components that fits. Only when not even the marker and the last component
  fit is the last component cut, at its start, so that its end (a name's extension)
  stays; the path bar cuts it at its end. The whole path is the tooltip and the
  accessible label, and it is shortened again at every width, font, density and scale.
  Find results, lists and checksum rows keep `PathText` (start, "...", last component).
  The dialog asks for its content's height in the regular layout (padding,
  header, gaps, the summary measured at the regular width, and the actions), between
  180 and 640 px, so it shows no empty band, and it turns compact only when the window
  cannot give it that height. The primary button keeps its caption's 12 px padding in
  either layout, and in the compact layout the actions keep 16 px from the right edge,
  clear of the resize grip. Focus starts on the primary action as before; Enter
  confirms, Escape and Cancel cancel. `operation-body` holds the same content as text,
  the question first ("Label: value" lines after a blank line), for tests and
  assistive copies; the warning stays `operation-warning`. The error colour on the
  dialog surface keeps the palette's text minimum in every theme
  (`all_themes_are_selectable_and_keep_text_readable`). `tests/delete_dialog_polish.rs`
  (software-window tests, run on Windows) checks the question first, every detail
  unclipped, the actions' padding and grip clearance and no empty band, at 8, 12, 20
  and 32 px and Touch 15 and 32 px, in a large and a 520 × 360 window, and the dropped
  path components (a UNC share, an archive, one very long component) at widths from 360
  to 1340 px; `delete_summary.rs` tests the texts. The Copy and
  Move dialogs print no path: the first line names items, the source folder is its
  tooltip, and the destination is the editable field, whose text is never shortened
  (`copy_and_move_print_no_path_and_keep_the_typed_destination_whole`).
- Compare directories compares on Enter from an option or its tolerance field, as
  other dialogs confirm from their options; Space ticks an option, and a held Enter or
  one during a comparison does nothing. In Checksums, Enter in the results or on the
  page starts the shown page's primary action: Calculate, or Verify on the Verify page.
  Enter never discards a verification's results: in them it verifies the same checksum
  file again, and Calculate, which would replace them, is then a button press only,
  without the default look. Dialog tabs and the Checksums and Jobs lists
  show keyboard focus with a 2 px frame. A check box answers the pointer on its box or
  its caption with a 2 px active frame around the box, and an Options › Colors swatch
  with a 2 px active-frame ring around it; only the frame changes, so the caption, the
  mark and the colour keep their contrast, and in every theme the active frame stands
  out from the surface at least as much as the resting one.
- A job window's footer leaves out Copy details, then Jobs, when they would not fit
  whole beside its dismiss button (Jobs offers Copy details too). It has no Details
  button: the job's whole details are what Copy details copies. Where the actions that
  stay (Pause or Resume, Resolve, or the question's Cancel operation) still cannot
  show their captions there, they show their glyphs only, as the Options footer's
  Export and Import do: each keeps its caption as its accessible label and on its
  tooltip's first line, and Resolve has a glyph of its own, apart from Pause's.
  The footer is not stacked. The job window is never narrower than its widest footer,
  Pause and Resolve as glyphs beside Cancel operation, so they stay whole at
  its minimum size with any font. A notice's Details and Copy details go glyph-only
  beside Close the same way. The failure decision window is never narrower than its
  Retry, Skip and Skip all buttons.
- A dialog can hand `DialogFooter` its actions as data (`footer-actions`, a list of
  `FooterAction`: caption, glyph, hint, shortcut, enabled, primary, a `drop` rank and
  `hidden`) instead of buttons. The footer measures each caption as `ActionButton` lays
  it out and shows, in their order and 8 px apart, as many actions as fit whole beside
  the dismiss button; the others move, lowest `drop` rank first (rank 0 only in the last
  step below), into a More… button at the end of the row, whose menu (opening upward,
  Up/Down, Enter or Space, Escape) lists them with the same captions, glyphs, shortcuts
  and enabled states. Where that is not enough, the footer takes these steps in this
  order, each only while the row still does not fit, decided from the measured captions
  alone: More… shows its glyph alone (Find at 320×200 with a 32 px font); then every
  action of rank 1 and up is in More… and the rank 0 actions show their glyphs alone
  (Find, or Back, at 320×200 with a 32 px font where text is wider, as on Linux); then
  the dismiss button shows its glyph alone too; and last the rank 0 actions move into
  More… as well, leaving only More… and the dismiss button, as glyphs, which fit a
  footer about 100 px wide. A button that shows its glyph alone keeps its caption as its
  accessible name and on its tooltip's first line, as the job windows' glyph-only
  actions do; an action without a glyph keeps its caption. A footer whose actions fit
  before these steps looks as it did. Such a footer never scrolls, never cuts a button
  and never puts one outside the window; it exposes `shown-actions` and
  `overflow-actions`. `tests/fitted_footers.rs` checks each step, and that it was needed,
  from the buttons' real widths in Find's and Checksums' footers at widths from 2400
  down to 120 px, fonts from 12 to 48 px and both densities. Up to eight actions. Find
  and Checksums use it for their footers; footers that pass buttons as children are
  unchanged. In the
  `footer-actions` literal, read no glyph (or string property) twice where one read is a
  branch of `?:`: Slint 1.18.1's generator then fails with E0382 in the generated Rust.
  Bind each glyph to its own property and read it once; `dialog-footer.slint` shows the
  pattern and the workaround.
- Dialog dismissal stays reachable at 320 by 200, including large touch fonts.
  A short About (under 300 px) keeps a 40 px title strip and tighter spacing; its
  Licenses tab and a library's licence scroll as a whole, keeping at least three list
  rows and four lines of licence, Down in the filter scrolls the list into view before
  it takes the keyboard (Slint 1.18.1 drops the focus of a control scrolled out of
  view at the next key, which then reaches nothing, Escape included), and Back sits
  beside Close. Hot Paths keeps its current-folder line at its height and scrolls. The
  Options pages' one-line captions and empty-state notes wrap, so 20–24 px fonts at
  320 px cut none of them.
  Compact forms (Compare folders, Hot Paths, Panel Filter, Attributes) move focus
  to dismissal once when they enter their compact layout, which can clip the
  focused field; resizing within that layout keeps the field being edited.
  Options reuses the shared anchored footer: Reset and OK occupy a scrollable
  action region, while Cancel stays fixed. The shared dismiss-enabled property
  preserves the Options save lock without changing other dialogs' default.
  [Compact-footer verification](../verification/2026-11-10-tree-prefix-search.md#options-footer-follow-up)
  records the Windows/Linux scope separately from native-window migration.
- At 320 × 200 these surfaces show nothing over anything else, nothing cut by a
  container that does not scroll and nothing beyond what a scroll view reaches, in Dark
  Touch and Light Touch with 15, 24 and 32 px fonts and in Light Desktop with a 32 px
  font. `tests/compact_surfaces.rs` checks this with the UI polish gate's layout checks
  (`tests/support/minimum_layout.rs`).
  - A picker's menu (`CompactPicker`) has rows as tall as the control, or as their text
    or icon where that is taller (large desktop fonts), with 4 px above it in a menu with
    separators. The menu is at most its window's height less 16 px: Slint moves a popup
    back into its window but never shrinks it. A menu too tall for that lowers its rows,
    down to their text or icon and 4 px, when every row then shows (the panels' Sort
    menu with a 15 px touch font shows its five rows whole); otherwise its rows keep
    their height and it scrolls.
  - Select files and Deselect files keep their header and footer, and the explanation,
    the mask field, Include folders and an error scroll between them, as in Compare
    directories. A body too short for the field opens scrolled to it: the field has the
    keyboard, and Slint 1.18.1 sends no key to a focused control scrolled out of view.
  - Checksums: an empty list's note sits below the column headings. While its lines do
    not fit there, the headings are left out (there is no row to head); lines that do
    not fit even the whole frame end in an ellipsis, and the note's accessible label
    keeps the whole text.
  - Attributes › Times: a row too narrow for the date, the time and the UTC offset on one
    line puts the offset below the time. The fields stay the same instances, so a resize
    keeps their text, caret and selection; Now and both pickers then show whole.
- About's version pill is its text's line with 2 px above and below, at least 20 px,
  so the 32 px fonts Options allows do not clip it; the sky grows by what the pill
  adds, and a sky too short for the icon, title and pill shows only the title. When
  the system asks for less motion (the main window's `reduce-motion`: Windows'
  animation effects off) the sky holds still in the middle of its loop, where
  Cassiopeia, the satellite and a shooting star show, and it follows a change of
  the setting while About is open. The `about_dialog` tests
  `the_version_pill_holds_its_text_at_every_font_size` and
  `the_sky_holds_still_while_the_system_asks_for_less_motion` cover both with
  software windows; they do not change the Windows setting itself.
- About's Made with Slint pill (Slint's `AboutSlint` widget cropped to its logo: its
  padding and the version and address lines are left out) sits right under the sky at
  the right end of the tab row, aligned with the content edge like Close, on both tabs.
  It is as tall as a tab and at least 32 px (48 px at most, the widget's logo box), at
  the logo's proportions (70 × 32 px with the default font), so the row is at most 4 px
  taller than the tabs, which stand on the line below. The tabs stay in the middle while
  the row has room for as much space left of them as the pill takes (a dialog about
  376 px wide); a narrower dialog moves them left of the pill. A dialog too narrow for
  the tabs and the pill side by side (narrower than about 298 px) shows the pill at the
  top right of the About tab instead, beside the introduction, and scrolls it with the
  tab; the Licenses tab then has none. A covering touch area keeps a click from opening
  Slint's link. The `about_dialog` test
  `slints_pill_sits_under_the_sky_beside_the_tabs_or_at_the_top_right_of_the_about_tab`
  covers these places from 1260 × 660 down to the smallest dialog with software windows.
  The cut-off checks (`minimum_dialogs` and the acceptance UI gate) leave the cropped
  version and address lines out by their text.
- Filename colors offer a reusable visual color picker: hue, saturation/value,
  numeric RGB values, sample colors, and reset to the theme; each color's hex value
  is edited in its field on the Colors page. Existing automatic
  focused/selected color derivation and contrast adjustment remain in force.
  Its footer stays fixed and the palette scrolls above it. A short picker keeps
  at least one whole row of sample colors visible: it first narrows its gaps
  from 8 px to 4 px and drops the divider, and below 240 px it also reduces its
  padding, spacing and title size. Known limit: at 320 × 200 with touch density
  and a 32 px font, the title, the two stacked footer rows and one 46 px sample
  row need 9 px more than the window has, so the palette shows a 37 px viewport
  and must be scrolled.
- File-valued settings have Browse actions. Choosing a file does not execute it;
  cancellation does not overwrite the draft. Native paths stay lossless through
  the backend and late results cannot overwrite a newer form or another item.
- Rows, tabs, the path bar, dialog bodies and errors, status text and viewer and
comparator captions and status lines show file and folder names and paths through
`shown_text` (`midnight_core::visible_name`), which displays the explicit
bidirectional controls as `\u202E`-style escapes. Name and destination fields,
operations and copy actions (Copy details,
  error copies, right-clicking a breadcrumb) keep the raw text. The path editor
  starts from the escaped location and copies what it shows. Enter on that text
  opens the raw location, and that text followed by a separator and more path
  opens a path below it; any other edit of an escaped name is taken as typed.
- Editors and Custom Commands share an ordered argument list. Compact +, minus,
  up and down actions sit beside its heading; selecting a row shows its value in
  one editor below. Insert file/value appends a new argument. Users do not edit
  JSON. Spaces, quotes, empty arguments and Unicode remain literal argv data;
  there is no shell splitting or interpretation.
- Custom Commands is reworked from Salamander's User Menu implementation:
  command list plus selected command form, executable/command, arguments,
  initial directory, icon and insertion helpers. Advanced shell options are
  disclosed separately. Drag reordering and the existing backend validation,
  confirmation and execution boundaries are retained.
- Attributes, times and case share a readable form, useful icons and compact
  operation summary. An obsolete preview table must not dominate the dialog;
  backend preflight and necessary confirmation remain intact. The header shows
  the single item's name or the item count, plus plain sentences only for taken
  names, unsupported items or hard links. Timestamps use separate Date, Time and
  UTC offset fields (`metadata-time-field.slint`): typing always works, Up/Down
  step the part under the cursor, and calendar/time picker popups are aids. Their
  starting values come from the preparation worker (`LocaleService::local_calendar`);
  the UI never infers a timezone. Values start as year-month-day and 24-hour times;
  the fields also accept the system's own forms from the worker-captured
  `DateTimeInputFormat`: its numeric date order (on Linux, glibc padding flags such
  as cs_CZ's `%-d` are read) and its clock, including a 12-hour clock with the
  locale's AM/PM markers in any letter case. Arrow steps and picker choices keep the
  form a value was typed in; on a 12-hour clock Up/Down on the marker and the time
  picker's AM/PM column switch AM and PM. Hints and errors show both forms with an
  example. A ticked row still showing its starting value keeps that exact instant
  (sub-seconds and seconds of the offset included). The worker also captures the
  local calendar of each set time (`MetadataPreview::patch_calendars`), and the
  review notes a typed offset that is not the local one there.
  Every field shows its row's current value from the start, ticked or not, in the
  text colour, and takes input at once; a field still showing the starting value it
  was given follows a newer one from the worker (the first UTC placeholder becomes
  the local time), an edited field keeps its text. Only ticked rows are applied.
  A pointer press into a field, on its text or its padding, or a click on its
  calendar or clock button ticks the row, and so does any change of a value
  (typing, Up/Down, a picker step or choice, Now). Moving the keyboard into a
  field with Tab does not tick it, because Tab only navigates: the field tells a
  press from Tab by its caret, which Tab selects the whole
  text with and a press sets down with no key held. Unticking keeps the shown
  values, so ticking again restores them. A row whose time the file system cannot
  set stays disabled: it shows its value greyed, and clicks neither tick it nor
  open its pickers. Clicks inside a picker keep it open. The calendar applies a
  date only when one is chosen (a day, Today, Enter or Space); Escape or an outside
  click close it unchanged. The time picker changes the field at once: Enter, Space
  or an outside click keep the time, and Escape restores the time and the row's tick
  as they were when the picker opened. Its ▲/▼ step when pressed and, held on the
  arrow, again after 500 ms and then every 50 ms; off the arrow the repeat pauses.
  The wheel over a column steps it, one step per 60 px notch, which a touchpad's or
  a fine wheel's small deltas add up to; a press anywhere on a column chooses it for
  the keyboard, which shows the chosen column with a 2 px active frame. Values wrap
  within their column without a carry (23 → 00 keeps the date, 59 → 00 keeps the
  hour), as the field's own Up/Down do; on a 12-hour clock the hour runs through the
  day, so 11 AM ▲ gives 12 PM. Keys are listed in
  [keyboard shortcuts](../KEYBOARD_SHORTCUTS.md#attributes-times-and-case-editor).
  The arrows and the calendar's month chevrons are drawn shapes, not font glyphs,
  so they sit centred at one size in every column with any font; the hovered one
  also gets the theme's active frame, which stands out from the surface in every
  theme where the hover colour alone does not. Where four rows or the column names
  do not fit the window (320 × 200 with a 32 px font), the time picker leaves out
  the names and narrows its columns to their values. The calendar's cells shrink
  to the window, down to what a day's number needs, and a calendar that still does
  not fit scrolls, keeping the keyboard's day in view; it turns months with the
  wheel while it fits. Both pickers place their frames on whole device pixels.
  The dialog tells the presenter that the draft changed (`dirty-metadata`) on the
  first change after a review and on any change while a review is requested or
  being prepared; later changes of the same draft wait for Review, which sends the
  whole draft. Each such word retires hints with a synthetic pointer exit, which
  would end a held arrow's grab.
  Now fills its row with the current moment at the worker's latest local offset
  (UTC without one), computed without a timezone lookup, and ticks the row; the
  presenter keeps that exact instant, so an unedited row applies it, sub-seconds
  included, and nothing is re-read at review or Apply. A clean Apply closes
  without a result notice.

## Command-line insertion

Ctrl+Enter appends the focused filename and focuses the command line.
Ctrl+P and the left/right path shortcuts insert the path at the caret or replace
the selection. Directory paths end with the native path separator (`\` on Windows,
`/` on Linux/macOS), preserving an existing separator without doubling it. This
allows Ctrl+Enter to append a filename directly after an inserted directory.
These actions add **no quoting, escaping or separator spaces**.
Spaces and quotes that belong to the actual name are preserved. Insertion can
occur inside text or quotes the user has already typed, and never executes it.
The resulting editor text is intentionally shell syntax when explicitly run.
Names that cannot be represented exactly or contain control characters are
rejected without modifying the draft; no lossy filename substitution is allowed.
The command prompt's `>` is followed by only 4 logical pixels of padding in both
mouse and touch layouts. Long working paths retain their horizontal viewport. A
scrolled one shows the folder's start, at least two characters, and "..." over the
strip's start, where the strip is cut, so the two never overlap; that start takes the
strip's wheel, drag and tooltip. Without room for it and a character of the end, the
strip shows its end alone.

## Performance requirements

Text/source viewers read and render the visible region with a small bounded
prefetch/cache budget. They must not wait for full-file parsing before showing
or scrolling text. Background indexing, syntax state and search must be bounded,
cancellable and independent of UI input. Measure end-to-end frame latency and
bytes read, including large/long-line files and rapid reversal of scroll input.
Image opening, fit/zoom and panning need separate measurements; repeated decoding
or resampling of unchanged image data must not occur on the UI thread.
Ctrl+F opens and focuses viewer search; typing and confirming a query must work
with the actual viewer window, including text, source and binary representations.

Panel file/folder icons fill the unchanged row height, leaving one logical pixel
at each edge. Filename spacing stays fixed instead of becoming negative as
icons grow. Wheel scrolling presents intermediate pixel positions and settles
on the nearest row only after input/inertia ends; touchpad and touchscreen
gestures retain their continuous native motion.
A wheel notch glides for 200 ms up to 120 px, falling to 100 ms from 480 px, times
the Scrolling glide setting, and shorter when input arrives quickly
([wheel glide](SCROLLING.md#wheel-glide)); rendering is requested every 8 ms and
motion advances only on actual redraws. A delayed redraw consumes at most 50 ms
of the curve, avoiding an invisible fast-forward. Idle snapping starts after a whole
100 ms check interval without input or motion and glides to the row for
200 ms times the glide setting (at once with reduced motion or glide Off), and real
gestures or explicit cursor navigation cancel it.

Long panel paths preserve their beginning and ending: whole middle components
collapse into one ellipsis marker. `PathCollapse` decides which components show; the
path bar and the Delete confirmation's Location (`PathTrailText`) draw what it decides. Up to two leading components stay (while they
take at most about a third of the bar), then the marker, then the longest suffix of
whole components that fits; nothing overlaps and no component is cut, except the
last one, which is shortened at its end only when it alone is wider than the bar.
At least 32 logical pixels after the path stay free, inside the bar: each shown
component is drawn as its own caption, rounded up to a whole pixel, so the fitting
keeps a pixel per component in reserve. Clicking there or on the marker edits the
complete path, and the marker's tooltip shows it. A Network location's editor holds
only its own caption, while its parts join the captions of its ancestors: there the
marker's tooltip names what it stands for, the chain from the outermost location
through the last ancestor it omits. Every shown
component keeps its backend path identity; shortening never changes a copied or
opened path. Component captions, like file names in rows, tabs and the details
bar, keep a left-to-right base direction, so a right-to-left name keeps its
extension at the end.

## Native settings file choosers

UI code sends `PickPath` with a purpose, native initial-directory suggestion and
owner handle. A single bounded backend worker owns the actual chooser until it
exits. `PathPicked` returns an optional raw `PathBuf`; ordinary Cancel is `None`.
The requesting form rejects results after a different page, selected command,
field or draft revision becomes current. No selected program is launched.

Windows uses the [Common Item Dialog](https://learn.microsoft.com/en-us/windows/win32/shell/common-file-dialog)
on its own STA, with filesystem-only results, no current-directory changes,
no recent-file recording and no `MAX_PATH` buffers. Cancellation posts the
ordinary close message to the exact dialog window obtained through `IOleWindow`
on that same STA. `IFileDialog::Close` was experimentally a no-op from a timer,
matching [Mozilla's implementation investigation](https://bugzilla.mozilla.org/show_bug.cgi?id=1870660).
Shell extension modal UI can defer closure; the worker retains its permit until
the real chooser returns. It never destroys Shell windows or terminates threads.

The Custom Command icon chooser is the Shell's `PickIconDlg`, not a Common Item
Dialog, on its own STA. Its result is the dialog's path text: one pair of
surrounding quotes is removed and `%NAME%` references are expanded from the
process environment (an undefined reference stays literal) before handle-based
validation and pixel capture; the selected module is never executed. Cancellation
posts the ordinary close message to the chooser whose owner chain reaches the
worker's hidden owner window.

Linux uses the [FileChooser portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.FileChooser.html)
over the session bus, subscribes before opening, validates the returned request
ownership, decodes local file URIs without losing native bytes, and closes its
request when cancelled. Native service absence is an actionable error; manual
path entry remains available. Chooser output has a 4 MiB transport resource
budget, distinct from platform/path-length policy.

## Panel Filter, Archives and the command toolbar

Dialogs keep an explicit primary action and a separate dismissal.
Panel Filter has OK (validate and apply), Cancel (discard draft) and Show all
(disable the applied filter, preserving its expression). It has no separate
enable checkbox; panel indicators provide a direct × dismissal. Pending filter
validation cannot re-enable a cleared filter. Its expression is the shared edit line,
as in Find, with that line's inline clear and focus underline. Recent filters open
from the line itself (`HistoryLineEdit`, Down or Alt+Down; Escape closes the list
before the dialog), and any other edit line can use the same list. Compare and Jobs
share the bounded scrolling footer; Jobs says Cancel job to distinguish it from Close.

Archives retains built-in association checkboxes, but user-added extensions are
removable values with a leading ×. A compact + opens their inline editor instead
of permanently displaying a second form. All edits are staged until Options OK.

The custom-command toolbar announces icon visibility when each button is created
as well as when geometry or configuration changes. This also covers recreating
the row after leaving the compact layout. The callback updates a bounded
presentation set; the backend still loads the icons asynchronously.

References: [Fluent 2 dialogs](https://fluent2.microsoft.design/components/web/react/core/dialog/usage)
and [tags](https://fluent2.microsoft.design/components/web/react/core/tag/usage).
These inform action grouping and dismissal, while existing keyboard behavior,
native paths, cancellation and settings transactions remain authoritative.

## Common edit

Every single-line text field in a dialog or tool window is the common edit: a framed
line that, while it has the keyboard and text and is enabled and editable, shows an
inline clear × at its right end; a click on the × empties the field as an edit and
leaves the keyboard in it. A password field shows no × (as Fluent's own field), so the
archive password clears only while Show password reveals it. There are two
implementations with that behaviour:

- `HintedLineEdit` (Slint's Fluent `LineEdit` with an optional hint) and
  `HistoryLineEdit` (the same line with its recent entries), used by Options, Find,
  Panel filter and About;
- `ThemedLineEdit` with `clear-button: true`, for fields that need their caret
  offsets (Make File List inserts variables at the caret) or the theme's selection
  colours, which the stock `LineEdit` does not let a host set (contrast in Light).

Recent entries open from the line itself (`HistoryList`, Down or Alt+Down), never from
a picker beside it: Panel filter's expression and Make File List's line template. The
`common_edits` test scans every `.slint` file of the application, the viewer and the
comparator and fails for any other text field that is not listed with its reason:

| Field | Why it is not the common edit |
| --- | --- |
| Make File List preview | multi-line and read-only |
| Command output window's command line | read-only: selectable, nothing to clear |
| Custom Command script | multi-line |
| Command line | its own history, completion and path insertion keys |
| Path bar editor | its own Go button and native path paste |
| Panel quick search | Escape and Backspace end it; it is not a form field |
| Date and time segments (Attributes › Times) | each segment steps with the arrows |
| Number boxes (`ThemedSpinBox`, the colour picker's channels, viewer font size) | they step with − and + or arrows |
| Viewer zoom entry | a transient editor that leaving cancels, so an × would only cancel |
| Viewer find bar | its frame already holds its own inline clear |
| Compare directories' time tolerance | a short number field with its unit |

Pickers: a dialog uses one kind of dropdown for its fields. Make File List's Presets,
Insert variable (action menus, which Slint's `ComboBox` cannot be) and Output are all
`CompactPicker`s, as tall as its fields. Pages that use Slint's `ComboBox` throughout
(Options pages, Find, Panel filter) keep it.

## Alignment of neighbouring controls

Every window, dialog and popup lines up its controls with each other, not only with the
window. The UI polish gate checks these rules on every surface it renders (U-K32–U-K39
in [acceptance/ui-polish.md](acceptance/ui-polish.md)), and the `midnight-ui` layout
tests that use the gate's layout findings (`tests/support/window_polish.rs`) check them
too.

- **One row, one height.** Controls side by side (a field and its button, a label and
  its control, pickers, the buttons of one row) share their vertical centre within 1 px,
  and framed controls (fields, buttons, pickers, number boxes) their height within 1 px.
  A multi-line control (a list, a multi-line field) is exempt.
- **A field and its button.** The small button that belongs to a field (Browse beside a
  path) is `FieldButton`: square, as tall as the field, showing a centred glyph (for
  Browse a folder with an ellipsis, for Find's filename mask history a down chevron)
  rather than a font's "…", whose dots sit on the baseline, near the button's bottom
  edge. Its caption chooses the glyph and heads its tooltip; its accessible label names
  its action.
- **Every character drawn.** A caption, label or message uses only characters that the
  application font, or the fallback the system finds for it, can draw. The font differs
  per platform (Segoe UI on Windows, the fontconfig default on Linux), and a symbol of
  the Common script (arrows, triangles, check marks) gets no script fallback, so a
  character the font lacks is drawn as an empty box. A field's list button and a
  section's disclosure show the bundled chevrons (`FallbackIcons.chevron-down`,
  `FallbackIcons.chevron-right`), not "▾" or "▸". The `midnight-ui` test
  `tests/caption_glyphs.rs` draws every non-ASCII character of the `.slint` string
  literals and of the non-test Rust sources whose text the windows show, with the
  application's font on the platform that runs it, and fails on any drawn like the
  missing-glyph box. The alignment checks judge a symbol caption only by where its
  pixels sit, so they pass a box.
- **Captions in the middle.** A button's, picker's or tab's caption is centred
  vertically (its line box within 1.5 px, the glyphs of a caption without letters within
  2 px or an eighth of the font) and is never cut at its top or bottom.
- **One content edge.** Controls and labels stacked in a column share their left edge,
  and right edge where both stretch; edges 1–6 px apart are a defect, a larger indent is
  deliberate.
- **Even rhythm.** Within a column the gaps between rows are even; a gap three times the
  usual one (and 12 px more) is a defect, except above a dialog's footer row, which a
  window taller than its content pushes down.
- **Room to breathe.** Controls side by side, and fields stacked, keep at least 2 px
  apart; no control comes within 2 px of its dialog's edge. Segments of one control
  (tabs, page buttons, a list's rows) touch by design.
- **No needless "…".** A label is not shortened while the free room beside it would
  show it whole.
- **One size per bar.** The function bar's keys show their captions in one size and
  all with their icons or none: the widest shown caption decides both, the size never
  below three quarters of the font, and a caption that would need less shows its key
  alone ("F9"). Find's query rows share one height (the stock field's), which its Help,
  Add, Remove and Browse buttons and the folders picker take; a form note's text starts
  where a check box's caption does.
- **One rhythm for options.** Check boxes, radio buttons and switches stacked one after
  the other keep one gap (U-K39). On the Options pages that gap is `OptionRows.row-gap`
  (`options-rows.slint`: 2 px, 4 px with touch density); every check box row keeps the
  whole control height as its hit target and the room of its focus ring, a page's groups
  are set apart by `OptionsRule` (`OptionRows.group-gap` above and below its line), and
  dependent options keep their indent. `tests/options_rhythm.rs` checks every page in
  both densities and prints each page's height.
- **Panel toolbar.** The View and Sort pickers stay whole ("Detailed", "Name ↑", each as
  wide as its longest caption) while the drive and folder buttons fit beside them. When
  the buttons need that room, the pickers turn compact: the view's own glyph, and the
  sort key's initial with its direction ("N ↑"), with the same accessible names,
  tooltips, menus and keys. The choice compares the buttons' measured width with the
  room beside the whole pickers, measured from their captions, never with the pickers'
  current size, so the row cannot flip between the two; the buttons are never narrowed,
  and scroll behind the strip's arrows only when even the compact pickers leave them too
  little room (`tests/panel_toolbar_fit.rs`).

## Make File List layout

The dialog has one content column. The header's close button, the line template, the
pickers, Output, the destination row, the preview and the footer's Cancel end at the
same right edge, as far from the dialog's edge as the column starts from the left
(16 px, 20 px with touch density); the form's scroll bar lies in that right margin,
left of the resize edge, instead of taking a column from the fields. Fields and pickers
share one height (the control height, at least the font size and 12 px). Presets and
Insert variable stand side by side while both captions fit and one above the other
otherwise; a short dialog scrolls the form and keeps the footer whole. The
`file_list_layout` tests check this at 8, 12 and 32 px fonts, desktop and touch
density, Dark and Light, a wide and a narrow window and the 328 × 208 minimum.

## Review checklist

Use isolated preferences and owned fixtures. Exercise mouse and keyboard,
tab order, Escape, resize, scrolling, validation failures, save/reopen, picker
cancellation and stale replies. Check wide and narrow windows in light/dark
themes and increased font/density. Run a harmless explicit custom command to
verify that displayed argument choices match actual argv. Record measured viewer
latencies; reduced CPU cost alone is not evidence of instant scrolling.


## Settings and forms

Options initially requests more vertical space and remains bounded by the current
window/work area. The pages that fit their lists to the shown height (Comparators,
Plugins, Keyboard) keep at least six control heights and 32 px; in a smaller window
the page scrolls instead of squeezing their lists. Plugin catalogs scroll
independently of the single pinned Rescan all plugins action. The full package path replaces the redundant plugin ID
line. Previews shows the cache location resolved by the backend, above Clear cache;
new settings use a thumbnail size of 96 logical pixels (the edge of tile icons; previews
fill their tiles' preview box).

All confirmation/form bodies retain a themed icon, title and divider, including
native hosts. Rename derives initial height after applying its final text
and font at the intended width. Cancelled operations retain existing lifetime and
job admission rules. Header close, Escape and Cancel use the same form dismissal.
Active Copy/Move/Delete progress instead asks for cancellation confirmation;
Continue is the default and dismissing that question keeps work running.
Operation decision windows (name conflicts, Copy/Move errors and metadata conflicts)
never cancel on dismissal either: their header close, the window's X and Escape
close only the window, like Decide later, and the footer Cancel job is their explicit
cancellation.
Small list controls share compact padding and centered glyphs; remove/clear actions
use the same red cross in history, cache and custom archive extensions.

The theme picker sorts labels alphabetically but translates to stable saved IDs.
Archive preset filenames use a yellow base, with existing contrast derivation for
every theme and selection state. Only an exact previous preset migrates; custom
or reordered color rules are retained. Find places Whole words beside Containing
text. Native mount-path labels sort after named volumes within their section;
paths themselves are never rewritten by sorting.

Ctrl+Shift+P appends the focused native item's full path to the command draft,
without quoting or added spacing. Virtual providers never expose backing temp
files through this action.

Folder tab body tooltips show only the complete displayed location path, including
archive members and nested archive paths. They omit the repeated tab caption and
gesture instructions. The tab caption remains compact, and the separate close
button retains its action/shortcut hint.

Tab path tooltips allow up to 1,040 logical pixels, constrained by the space
available beside the cursor. Ordinary action tooltips retain their compact width.

Viewer file navigation and comparison differences use distinct shortcut contexts.
Shared controls such as Find and Close apply to both windows, including their search
fields; file cycling and difference navigation apply only to their respective content.
This permits Alt+Left/Right to cycle files in viewers while retaining difference
navigation in comparators, with separate user overrides and conflict checks.

## Options page order

Options lists its pages in the order users look for settings:
general behavior and appearance first, then how the panels show files, file
handling, the programs that open and compare files, tools and keys, and the rarely
changed plugin catalog last. Related pages sit next to each other, and the most used
pages are at the top.

| Position | Page | Why there |
| --- | --- | --- |
| 1 | General | Everyday behavior: filename search, hidden files, sizes, copy verification, disk I/O, command history and the settings file. The most used page, and the page Options first opens on. |
| 2 | Appearance | Theme, spacing, fonts and text rendering: the other most used settings. |
| 3 | Colors | Filename colors in the panels: the look of the file list, beside Appearance. |
| 4 | Previews | Image and video thumbnails in the panels' Thumbnails view, their size and cache: panel display again. Its switch for previews inside archives leads on to the next page. |
| 5 | Archives | Which file types open as folders, and custom archive extensions: how files are handled. |
| 6 | Editors | The program that F4 opens for each file name. |
| 7 | Comparators | The program that compares two files, with the same rule list and program fields as Editors. |
| 8 | Custom Commands | Your own programs and scripts on the toolbar and in F9, again with a program, arguments and a working folder. |
| 9 | Keyboard | Keys for every command, Custom Commands included: a new command gets its key on the next page. |
| 10 | Plugins | Installed viewer, filesystem, archive and thumbnailer packages: viewer priority and extensions, diagnostics and Rescan. Changed rarely, so last. |

Every page keeps its content and its keyboard access: Tab into the list, the arrow
keys, Enter or Space, and the topic picker that replaces the list in narrow windows.
Every page also keeps its topic identity (`OptionsTopics` in
`crates/midnight-ui/ui/options-navigation.slint`, whose `ids` and `positions` map list
positions to identities and back; the removed Command topic 2 shows General). The
Custom Commands toolbar, F9 and shortcut, the settings pickers, page validation errors
and the page shown again when Options reopens in the same session all use these
identities, so they open the same page in any order. No setting stores a page, so
nothing needs migrating. Options has no page for toolbar buttons or Hot Paths (Hot
Paths has its own manager), and the internal viewers have none of their own: their
extensions and priority are on Plugins › Viewers.

## Options feedback and Tab order

**Colour picker swatches.** Each of the picker's eight preset swatches keeps its
`RGB r, g, b` accessible label and carries its colour's name and hex value as its
accessible description and in its tooltip ("Red #EF4444: click or press Enter to use
it."). Hovering draws the 2 px active-frame ring that the Colors page's own swatches
use, and keyboard focus draws the swatch's frame 2 px wide in the active colour.
Closing the picker with Escape or Use color while a swatch's tooltip shows closes only
the picker and leaves no tooltip behind: Slint closes a popup's sub-menus with it but
not its tooltips, so the picker retires them first (`ColorOptionsView.picker-closing`,
`tooltip::dismiss`).

**Reset to defaults.** The button is enabled exactly when a click applies: not while a
page's file chooser (an editor or comparator program, a Custom Command program or
folder), an Export or an Import is outstanding, and the presenter refuses in the same
cases (an Export takes its values when its file is chosen). Every applied click says
what it did in one line above the footer, which screen readers find as **Options
status**:

- "Reset to defaults: every page now shows its defaults. OK saves them; Cancel keeps
  your settings." when anything changed, unfinished page edits included;
- "Reset to defaults changed nothing: every page already showed its defaults."
  otherwise;
- "Reset to defaults did not apply: …" with the reason, when the keyboard map could
  not be restored.

The line wraps instead of eliding behind a tooltip (Slint 1.18.1 cannot take a tooltip
destroyed while it shows inside a popup). It goes with the next change of a setting,
an Export or Import, OK, Cancel or the next Options session, and a settings file
message takes its place while that shows.

**Archives Tab order.** From the Archives entry Tab visits the page entries below it,
the format, the association search, the built-in association check boxes, the custom
extension field's Add, then Reset to defaults, Export configuration…, Import
configuration…, OK, Cancel, the header's close and the page list from General, every
control in visual order, and wraps to Archives. Slint 1.18.1 moves the keyboard by Tab
only into a line edit that is at least partly in view (the field clips its text, and
an item clipped away counts as hidden unless only a Flickable hides it). The Archives
page (`ArchiveOptions`, a `FocusScope` that never takes the focus itself) therefore
passes Shift+Tab from the first file type, and Tab from the format, to the search: it
focuses the field, which scrolls it into view, and selects its text as Tab does. Any
other line edit in a scrolled page has the same limitation; at the default size no
other Options page scrolls one out of view on the way (the every-page test below).

**Keyboard page.** The command list is one Tab stop (the arrows, Home and End move its
selection); its frame takes the active colour while it has the keyboard, as the Colors
rule list does, and the list has an accessible name and item count. **Press shortcut
keys** reached with Tab or Shift+Tab lets those keys pass without recording them;
clicked, or once it has recorded another key, it records Tab and Escape and leaves. A
modifier pressed on its own records nothing and does not count as that other key, so
Shift on its way to Shift+Tab leaves the field without recording Shift+Tab.

**Page list keys.** The arrows (Up and Down, Left and Right) move from the focused
entry, as in any list, Home and End go to the first and the last page, and the focus
and the page shown move together, so they agree after each of these keys; the ends
clamp and the order stays the list's. Tab moves the focus from entry to entry without
changing the page. A key that comes before the focus has moved, as with key repeat,
goes on from where the last one went (`current` in `options-navigation.slint`).

**Tab order on every page.** `crates/midnight-ui/tests/options.rs` (`tab_walk`) walks
each page's Tab cycle at the default 1340×820 window with default settings. Every
stop stays inside Options on something that was there before the walk, never on a
hidden (`visible: false`, or clipped away where no scrolling brings it back) or
disabled control; it scrolls into view and changes by at least 3:1, the UI polish
gate's measure. (A check box drawn by its own indicator keeps its native control
transparent on top; the 3:1 change is what tells that its focus shows.) Every control
a user can scroll to is reached, in visual order: the header's close, the page list
from the top, the page row by row (or column by column) and the footer from the left.
The cycle wraps, Shift+Tab retraces it, and the walk changes no control and records no
shortcut.

Tests: `colour_picker_swatches_name_their_colour_and_show_hover_and_focus`
(`tests/form_polish.rs`); `reset_always_reports_what_it_did_until_the_next_edit_or_session`
(`src/presenter/preferences/options_tests.rs`), `reset_waits_for_a_pending_export`
(`src/presenter/preferences/settings_file_tests.rs`) and
`reset_defaults_says_what_it_did_even_when_nothing_changes`
(`tests/options_transaction.rs`); `archives_tab_cycle_reaches_the_footer_and_wraps_in_visual_order`,
`keyboard_tab_cycle_shows_the_command_list_and_records_no_shortcut`,
`page_list_keys_move_from_the_focused_entry_and_keep_focus_and_page_together`,
`every_options_page_tab_cycle_keeps_to_shown_enabled_controls` and, for the page order
and the clamps at both ends, `options_pages_list_in_reading_order_and_keep_their_topic_identities`
(`tests/options.rs`).
