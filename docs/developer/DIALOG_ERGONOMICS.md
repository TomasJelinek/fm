# Dialog ergonomics and viewer responsiveness

Accepted follow-up, 7 November 2026. Implementation and verification are tracked
separately; this document is not a claim that every item is already released.

## Settings and dialogs

- Initial form size follows its content. Short Rename/Create/Delete summaries
  and informational notices use compact heights; long diagnostics grow to a
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
  Under the [11 November policy](../superpowers/plans/2026-11-11-dialog-polish-followup.md),
  only modeless working windows require free movement outside the main window.
  Attributes / Times / Case remains embedded. Do not migrate other embedded modal
  forms to satisfy the superseded all-native-window requirement.
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
- Compare directories compares on Enter from an option or its tolerance field, as
  other dialogs confirm from their options; Space ticks an option, and a held Enter or
  one during a comparison does nothing. Dialog tabs and the Checksums and Jobs lists
  show keyboard focus with a 2 px frame.
- A job window's footer leaves out Copy details, then Jobs, when they would not fit
  whole beside its dismiss button (Jobs offers Copy details too), and the failure
  decision window is never narrower than its Retry, Skip and Skip all buttons.
- Dialog dismissal stays reachable at 320 by 200, including large touch fonts.
  Compact forms (Compare folders, Hot Paths, Panel Filter, Attributes) move focus
  to dismissal once when they enter their compact layout, which can clip the
  focused field; resizing within that layout keeps the field being edited.
  Options reuses the shared anchored footer: Reset and OK occupy a scrollable
  action region, while Cancel stays fixed. The shared dismiss-enabled property
  preserves the Options save lock without changing other dialogs' default.
  [Compact-footer verification](../verification/2026-11-10-tree-prefix-search.md#options-footer-follow-up)
  records the Windows/Linux scope separately from native-window migration.
- Filename colors offer a reusable visual color picker: hue, saturation/value,
  numeric RGB/hex, sample colors, and reset to the theme. Existing automatic
  focused/selected color derivation and contrast adjustment remain in force.
  Its footer stays fixed and the palette scrolls above it. A short picker keeps
  at least one whole row of sample colors visible: it first narrows its gaps
  from 8 px to 4 px and drops the divider, and below 240 px it also reduces its
  padding, spacing and title size. Known limit: at 320 × 200 with touch density
  and a 32 px font, the title, the two stacked footer rows and one 46 px sample
  row need 9 px more than the window has, so the palette shows a 37 px viewport
  and must be scrolled. Fixing it needs a product decision: smaller touch
  samples, or a one-row footer at tiny sizes.
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
  review notes a typed offset that is not the local one there. Clicks inside a
  picker keep it open; choosing a day, Enter, Escape or an outside click close it.
  The row's checkbox follows the draft, so typing into an unticked row ticks it.
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
mouse and touch layouts. Long working paths retain their horizontal viewport.

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
The discrete wheel impulse lasts 200 ms; rendering is requested every 8 ms and
motion advances only on actual redraws. A delayed redraw consumes at most 16 ms
of the curve, avoiding an invisible fast-forward. Idle snapping remains a
separate 100 ms transition after 80 ms without motion, and real gestures or
explicit cursor navigation cancel it.

Long panel paths preserve their beginning and ending: whole middle components
collapse into one ellipsis marker. Up to two leading components stay (while they
take at most about a third of the bar), then the marker, then the longest suffix of
whole components that fits; nothing overlaps and no component is cut, except the
last one, which is shortened at its end only when it alone is wider than the bar.
At least 32 logical pixels after the path stay free; clicking there or on the
marker edits the complete path, and the marker's tooltip shows it. Every shown
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

## Verification

The November 8 polish keeps an explicit primary action and a separate dismissal.
Panel Filter has OK (validate and apply), Cancel (discard draft) and Show all
(disable the applied filter, preserving its expression). It has no separate
enable checkbox; panel indicators provide a direct × dismissal. Pending filter
validation cannot re-enable a cleared filter. Compare and Jobs share the bounded
scrolling footer; Jobs says Cancel job to distinguish it from Close.

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

Use isolated preferences and owned fixtures. Exercise mouse and keyboard,
tab order, Escape, resize, scrolling, validation failures, save/reopen, picker
cancellation and stale replies. Check wide and narrow windows in light/dark
themes and increased font/density. Run a harmless explicit custom command to
verify that displayed argument choices match actual argv. Record measured viewer
latencies; reduced CPU cost alone is not evidence of instant scrolling.


## Settings and forms, 11 November follow-up

Options initially requests more vertical space and remains bounded by the current
window/work area. The pages that fit their lists to the shown height (Comparators,
Plugins, Keyboard) keep at least six control heights and 32 px; in a smaller window
the page scrolls instead of squeezing their lists. Plugin catalogs scroll
independently of the single pinned Rescan all plugins action. The full package path replaces the redundant plugin ID
line. Previews shows the cache location resolved by the backend, above Clear cache;
new settings use 96 logical pixels for the thumbnail's longest side.

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
