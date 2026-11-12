# Special locations and Jobs presentation

## Special folders

The drive dropdown currently includes Home, Desktop, Documents and Downloads.
Extend it with the host's Pictures, Videos, Music, user temporary directory and
registered OneDrive location. Windows uses native Known Folder paths, including
redirected folders, instead of constructing English directory names under Home.
OneDrive must be available/configured; do not advertise a fabricated default path.
Salamander `src/drivelst.cpp:1416` and `InitOneDrivePath` demonstrate separate
personal Known Folder and business account discovery. Use this as behavior research,
with independently written Unicode-safe Rust and dynamically sized native buffers.

All discovery and native icon loading runs on backend workers. Keep the Windows
implementation separate. Linux uses XDG user directories and its user temporary
directory; OneDrive is not fabricated on platforms without a registered provider.
Maintain the existing drives/special locations/bookmarks separators, current-volume
selection, native paths and stable location IDs. Tests cover redirects, absence,
duplicate paths, Unicode and long paths without assuming a live OneDrive account.

Native references: [SHGetKnownFolderPath](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shgetknownfolderpath),
[Known Folder IDs](https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid)
and [RegGetValueW](https://learn.microsoft.com/en-us/windows/win32/api/winreg/nf-winreg-reggetvaluew).
Registered OneDrive account strings use dynamically sized UTF-16 buffers and a
bounded retry if registry data changes while reading. A corrupt configuration
allocation bound is separate from native path length; no MAX_PATH buffer is used.

## Jobs

Replace the unstructured vertical text dump with a bounded jobs list and a selected
job inspector. Preserve stable job identities when progress updates or a new job
arrives. Show an operation icon, action and state in the list. The inspector separates
phase/progress, current file, source/destination, counters and reported issues.

Only show determinate progress when its denominator is known. Copy currently reports
observed copied/verified bytes, without a total recursive byte estimate: show those
counters and its current phase, not an invented percentage or ETA. Mutation target
counts refer to top-level roots, not the number of recursively visited files.

Keep cancellation, deferred conflict resolution, Copy details and Close visible in
a fixed footer. Closing the dialog keeps jobs running. Clicking a control must not
implicitly close the dialog. Cancellation remains cooperative and completed changes
remain visible. Actual failures use the shared error glyph/card; waiting for a choice
has a distinct non-error state. Long paths and error samples scroll and copy in full.

Use the existing cached backend progress values and bounded retained error samples.
No filesystem inspection, total-size scanning or parsing of formatted error text
belongs to this UI change. Test concurrent jobs, stable selection, all terminal states,
unknown copy totals, conflicts, long Unicode paths and desktop/touch/large-font layout.
