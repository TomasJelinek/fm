# Dialogs, live selection and directory hints

This extends the current application design with the user's 11 November requests.
Verification notes distinguish implemented source from checks already run.

## Windows and shared appearance

Only modeless working tools (Find, Jobs, viewers) need independent taskbar presence
and movement outside the main window. Attributes / Times / Case remains embedded.
Make File List is owner-modal, has a title/action icon and no taskbar item, and its
preview expands with the client area. Its exact captured selection is unchanged.

All application-owned client areas inherit the selected application font and
base size. Shared action buttons use the application palette, meaningful action
icons, and identical enabled/disabled keyboard, pointer and accessibility rules.
The operating system continues to draw native window decorations. Pointer clicks
on action toolbars must not leave a misleading active outline; keyboard focus and
actual checked/active state remain visible.

Find groups file name and matching mode, search folders, content and advanced
options. The result list receives remaining space; compact forms scroll without
hiding Close. Theme/font changes apply in the same refresh without discarding a
draft. Panel Filter has contextual Masks/Regex/Expression help; the help belongs
to its modal filter occurrence, blocks that owner and returns to the unchanged
draft on close. Old window callbacks cannot close a later occurrence.

Attributes has one Review changes action, not a duplicate Refresh action.
Date/time fields and adjacent Now buttons have the same compact height. Backend
locale capture supplies supported numeric Gregorian date order, separators and
clock markers; current/proposed timestamps use native formatting. Entered times
retain an explicit UTC offset to disambiguate repeated daylight-saving times.
RFC3339 remains supported; unsupported edit calendars fall back to that format.
Long help text scrolls within the body rather than overlapping the status/footer.

## Job history and confirmations

Canceling an unaccepted Copy/Move/Delete confirmation does not open Jobs or leave
a visible history record. Backend preparation still pins and validates sources
without executing the operation. Preparation or cleanup failures are reported;
they are not silently discarded as canceled drafts.

Jobs retains session history and offers Clear finished and Clear finished without
errors. Both keep every unfinished operation. The latter retains terminal errors,
partial source cleanup and unconfirmed durability/recovery reports. Clearing a
record never cancels work, and late terminal events cannot resurrect a cleared
record. Selection moves to a remaining record or an empty inspector.

## Dragging and transient selection

Internal dragging displays Preparing/Checking, Copy, Move or Not allowed near the
pointer, plus the matching cursor. The backend checks native source/destination
identity, ancestry and device placement for the preview. A self-drop is rejected.
Ctrl/Shift intent is resolved by the same transfer service as the actual drop.
Escape cancels; changing target, modifier, navigation or host invalidates stale
feedback. The preview never reserves or authorizes an operation: release repeats
validation. External native drag feedback remains owned by the OS drag service.

Rectangle selection repaints visible marks and updates selected counts/file bytes
while dragging. A cancellable latest-value backend summary runs independently of
committed selection. Exact snapshot/token identity rejects superseded results.
The UI submits shared immutable indexes, never scans a large directory for totals,
and releases presenter borrows before model notifications. Cancel restores the
committed status; stale folder totals are not presented as belonging to a new
transient selection.

## Directory sizes and disclosure hints

Native size walks follow directory-link/junction targets through platform-specific
resolution and stable filesystem identities. Track identities on active ancestry:
a target already on that ancestry is a cycle, is skipped with a diagnostic, and
never recurses indefinitely. Separate noncyclic aliases count their logical
contents independently. Broken/inaccessible targets produce partial totals and
retained details. This read-only behavior does not relax no-follow mutation rules.
Storage-weighted I/O budgets, cooperative cancellation and fair root scheduling
remain in force; sorting/selection alone does not cancel a scan.

Flat rows request subfolder-presence hints for the visible viewport before the
first expansion click. The backend stops at the first folder child, coalesces
requests and uses existing listing/I/O budgets. Cache entries belong to an exact
root snapshot, navigation and entry ID. Empty/Has folders/error hints do not
create an inline-tree session or change flat action/selection authority. Explicit
expansion still validates and adopts its own tree publication. The parent row
uses a restrained upward chevron and remains excluded from file operations.

Options provides one Rescan all plugins action shared by the Viewers, Filesystems
and Thumbnailers pages. It invokes the existing asynchronous rescan of all kinds.

See the [active task/evidence ledger](../plans/2026-11-11-dialog-polish-followup.md),
[tree provider contracts](../../developer/TREE_PANEL.md),
[dialog verification](../../verification/2026-11-11-dialog-fonts-metadata-and-drag-feedback.md)
and [Jobs history verification](../../verification/2026-11-11-jobs-history.md).
