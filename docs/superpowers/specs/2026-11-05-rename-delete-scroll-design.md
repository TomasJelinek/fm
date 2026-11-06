# Rename, deletion and native smooth scrolling

## Scope and behavior

Implement the requested native rename/delete milestone through the existing
backend boundary. F2 and the Rename button edit the focused entry, excluding `..`.
Select the basename without its extension (a leading-dot filename is selected in
full), following Salamander `src/fileswn5.cpp:2329`. Enter accepts, Escape cancels.
Names remain Unicode; source identities retain native path bytes/code units.
Only a single basename is accepted, with native validation on the backend.
Rename never overwrites an existing destination; a collision gives a clear error
and leaves both entries intact. Case-only renames must use tested native behavior.

Delete/F8 operates on the accepted multiselection, or the focused entry if empty.
The user-confirmed default is system Trash, with Shift+Delete/Shift+F8 requesting
permanent deletion. Confirm the mode, file/folder counts and a bounded filename
preview before enqueueing. Never silently turn a trash failure into permanent
deletion. Cancellation does not restore entries already removed. This follows
Salamander's separate recycle/permanent routes (`src/fileswn8.cpp`) while avoiding
its ANSI buffers, MAX_PATH limits and target-delete-before-rename fallback.

The mouse wheel must produce intermediate pixel positions in the actual native
renderer. Snap to a row only after input/inertia ends. Native touchpad/touch
physics remain intact. Diagnose event routing, timer cadence and render cadence;
synthetic 16 ms tests alone do not prove smooth visible scrolling.

F7 creates a new directory, including a nested relative path, based on Salamander
`src/fileswn5.cpp:1946`. Existing intermediate directories may be reused; the
final name must be absent. Validate the entire input before creating anything;
reject absolute paths, dot/dotdot components and links inside the entered path.
Keep a pinned base directory. Cancellation/failure can leave new intermediate
directories; report this honestly and never remove unrelated directories to undo it.

Click the blank area after the top breadcrumb to edit the complete custom path.
Enter navigates, Escape restores the original display; remove the redundant
right-hand button. Component clicks/right-clicks retain navigation/copy behavior.
Middle-click closes a tab except the last one. Persist ordered raw native paths
and each pane's active index; restore lazily without retaining directory data.
Explicit CLI locations and local edits before initial preference load take precedence.
Exact byte sizes and separate date/time fields use system regional formats. Locale
queries and native calendar formatting run on backend workers; UI uses cached values.

The bottom toolbar extends through F12: F11 connects and F12 disconnects a network
drive using Windows system UI (Salamander `salamand.rc:100–104`,
`mainwnd3.cpp:3848`). The optional native capability controls availability; other
platforms do not receive Windows drive assumptions. Midnight Explorer never collects passwords.
Options follows F12. Jobs exposes bounded queue/progress/cancellation details.

Panel modes and future media preview architecture are specified in
[panel views and thumbnails](2026-11-05-panel-views-thumbnails-design.md).

## Architecture and chosen approach

Extend the existing platform crates and Tokio engine. Shell-command based
mutations would weaken identity/path handling; doing I/O in a dialog would block
the UI. Neither fits the established requirements. Native providers implement
`MutationService::prepare` and return backend-only `PreparedMutation` objects
holding native identity/anchors. The UI receives display summaries and opaque
request IDs; it never owns native handles or parses displayed paths as authority.

Preparation and execution use bounded backend queues and separate capacity from
browsing. One mutation executes at a time initially, so conflicting queued work
is serialized. Confirmation consumes a prepared request once; duplicate IDs,
stale views, `..`, invalid names and changed targets fail explicitly. Duplicate
request/confirmation IDs are ignored idempotently. Navigation
or changing selection after acceptance never retargets an operation. New requests
remain possible during an active job. Progress is coalesced to at most 4 updates/s,
with reliable acceptance/terminal events and per-job cancellation. Root-item
counts and recursively processed entries are distinct; unknown totals do not
produce fabricated percentages. Shutdown requests cancellation without treating
an outstanding native call as completed.

Windows and Linux implementation files and tests remain separate. Destructive
traversal never follows symlinks/junctions. No application MAX_PATH or filename
length limit is introduced; platform errors are reported. Native identity and
trash guarantees and any limitations must be documented and tested. Tests use
isolated fixtures; never delete/recycle real user files or empty the user's trash.

## Verification

Provider tests cover Unicode/long paths, collisions, case-only names, invalid
components, missing/replaced sources, root rejection, symlinks/junctions, nested
deletion, trash metadata/unsupported cases and cancellation/partial failure.
Engine tests cover delayed workers, queue bounds, duplicate confirmations,
selection identity, navigation changes, errors, cancellation and refresh.
UI tests cover real F2/Delete/F8 keys, dialogs, basename selection, focus and queue
feedback. Scroll tests must include native input and rendered intermediate offsets.
Run both platforms' applicable tests, strict Clippy and native Windows smoke QA.
