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
  Dialogs have a top-right close button with the same behavior as Cancel.
  Under the [11 November policy](../superpowers/plans/2026-11-11-dialog-polish-followup.md),
  only modeless working windows require free movement outside the main window.
  Attributes / Times / Case remains embedded. Do not migrate other embedded modal
  forms to satisfy the superseded all-native-window requirement; existing native
  modal hosts still need a policy review.
- Ordinary confirmations and settings are modal and do not add separate Windows
  taskbar buttons. Existing native ordinary hosts set ownership before first show
  and retain normal title bars and resizing. Independent working windows (Find,
  Checksums, viewers and comparators) keep taskbar and Alt+Tab access, with the same
  application identity for system grouping. Help/About remain ordinary dialogs
  even when modeless. Make File List uses an owner-modal native host with no
  separate taskbar item.
- Options follows OK/Cancel: OK validates and saves the complete current form,
  then closes only after successful persistence. Cancel discards the draft and
  restores live previews, including the theme. Validation remains cancellable;
  dismissal is disabled during an admitted save. Failed saves retain the draft
  and explain the failure.
- General (formerly Panel) contains Panels and Command line sections, including
  remembering and clearing command history; there is no separate Command page.
  These controls participate in the same OK/Cancel transaction, including
  restoration of cleared history.
- Copy, Move and Delete use the operation as the primary/default action. A named
  destination receives initial editing focus; a deletion confirmation focuses
  its action button. Enter confirms from the input, option or primary button;
  focused Cancel and the header close button retain their cancel behavior.
  Escape also cancels. An empty named destination remains invalid; opening or
  focusing a dialog never submits it.
- Dialog dismissal stays reachable at 320 by 200, including large touch fonts.
  Options reuses the shared anchored footer: Reset and OK occupy a scrollable
  action region, while Cancel stays fixed. The shared dismiss-enabled property
  preserves the Options save lock without changing other dialogs' default.
  [Compact-footer verification](../verification/2026-11-10-tree-prefix-search.md#options-footer-follow-up)
  records the Windows/Linux scope separately from native-window migration.
- Filename colors offer a reusable visual color picker: hue, saturation/value,
  numeric RGB/hex, sample colors, and reset to the theme. Existing automatic
  focused/selected color derivation and contrast adjustment remain in force.
- File-valued settings have Browse actions. Choosing a file does not execute it;
  cancellation does not overwrite the draft. Native paths stay lossless through
  the backend and late results cannot overwrite a newer form or another item.
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
  backend preflight and necessary confirmation remain intact.

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

Long panel paths preserve their beginning and ending with a middle ellipsis.
The pinned leading components and the trailing breadcrumb share the same token
component and backend path identities; shortening never changes a copied or
opened path. Clicking the ellipsis edits the complete path.

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
