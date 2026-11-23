# Folder drops and native drag handoff

This extends the original [drag/drop design](2026-11-06-file-drag-drop-design.md)
after the reported inability to drag outside the application or into a folder
in the same panel. Implementation and qualification are tracked separately.

## Interaction

Dragging a selected row retains its selection; an unselected row supplies only
that row. A native inline-tree occurrence uses its captured provider identity,
including expanded folders. The parent entry is never a source. Inside the app,
an ordinary folder row is a destination; empty panel content targets its root.
The hovered folder receives an outline and a caption identifies the destination.
Details, Brief and Thumbnails use the same bounded geometry as rectangle selection,
including scrolling, panel swapping, zoom and DPI. Scrollbars, dialog overlays,
hidden panels and unsupported virtual targets cannot receive an internal drop.

The user explicitly chose Explorer-style defaults: on the same volume, Move;
between volumes, Copy. Ctrl forces Copy and Shift forces Move. The backend owns
the volume comparison; unknown or mixed source volumes fall back to Copy. Panel
position and drive-letter text are not evidence of volume identity. Until the
backend resolves the automatic action, UI feedback must not promise a specific
action. Ctrl+Shift and Alt link requests are rejected while Link is unsupported.

A drag released less than 400 ms (`MIN_DRAG_DURATION`) after the pointer press that
started it is an accident, whatever it is over: the other panel, a folder row, the
source panel or nothing. The drag is cancelled as Escape cancels it (the prepared
sources are released, no transfer, dialog or Move follows, the outline and drag
feedback clear, selection and cursor stay) and the source panel's status line leads
with "Drag cancelled: released within 0.4 s" for about four seconds or until the
next drag. The press is the window's own left-button press, read from the platform's
monotonic clock (injected through the Slint platform in tests), not the later
threshold crossing; releases at or after 400 ms drop exactly as before, Ctrl/Shift
included. Tabs, the path bar, Hot Paths, toolbars and dialogs receive no file drops,
so a release there cancels the drag at any time.

Move reuses ordinary operation admission, collision prompts and source-identity
checks. Qualified same-volume moves use native rename without checksum reads;
cross-volume moves copy and verify each file before deleting that source. Copy
and cross-volume Move freeze the user's verification setting at drop submission.
Full SHA-256 remains the default. Explicit quick mode checks size and independent
64 KiB first/middle/last samples, comparing the whole file at or below 192 KiB;
it can miss corruption between samples. Identity, publication and source-removal
guards remain required. An accepted drop is queued work, not a claim of completion.

## Captures and ownership

Flat targets use pane/navigation/entry IDs. Tree sources and destinations retain
an exact acknowledged projection, occurrence ID and provider registration. The
backend resolves them; display names and UI path strings never authorize access.
Preparation, target capture and queue handoff are cancellable and bounded. Late
receipts cannot revive a released gesture or retarget work after navigation.
Canceled and rejected provider capabilities are retired on backend workers.

Internal gestures remain in the application until the pointer leaves its client
area. Since 2026-11-23 the operating system then takes a prepared drag at once, so the
application under the pointer shows its feedback immediately; the drag carries its
earliest drop time (the press plus 400 ms) and the drop source cancels a sooner
release (Windows: Cancel; X11: XdndLeave; Wayland, whose compositor delivers drops
itself: an empty list). Until 2026-11-23 the window kept the pointer until the press
was 400 ms old, which left other applications without feedback during that time.
Incoming drops have no press in this application and are not timed. External handoff then releases UI capture only
after arming the exact live drag token. Windows' backend STA temporarily attaches
its input queue to the validated same-process window thread while calling
DoDragDrop, and detaches on every exit. COM, path serialization and file work remain on the backend. A real
owned-window test must exercise physical button release and cancellation; a
synthetic held-button value is insufficient to qualify this boundary.

The receiving application owns an outgoing operation and its verification.
Windows offers Copy and Move to the Shell target, which selects its normal
volume/modifier behavior. Bounded performed/logical drop-effect feedback reports
completion; it never grants source-deletion authority. A conventional unoptimized
Move that asks this source to delete is refused with a retained-source notice.
Missing or inconsistent completion feedback is reported as unconfirmed. A positive
target report is not independent filesystem verification by Midnight Explorer.

Linux X11/XWayland outgoing transport remains Copy-only, and so does native Wayland
transport (added 2026-11-23, not yet qualified on a compositor); internal transfers
still use the typed backend route.
Native incoming drops remain Copy-only on both platforms. Internal Move support
does not change those incoming protocol guarantees.

Windows backend qualification on 2026-11-10 includes actual Shell targets for
same-volume default Move, Ctrl-Copy, cross-volume default Copy and Shift-Move,
with files and nested directories. The owned-window input-thread/worker-STA
handoff also passes success/cancel/success. Final integrated UI and deployment
qualification remain pending; see [the dated evidence](../../verification/file-drag-drop.md#2026-11-10-folder-targets-and-windows-shell-move).

## Verification

- Source threshold, group capture, unselected rows, quick release and Escape.
- A release 399 ms after the press cancels with no transfer; one at 400 ms drops.
- Exact folder destinations in all three layouts before and after scrolling.
- Automatic volume defaults, explicit Ctrl/Shift, unsupported link modifiers.
- Tree receipts, selection acknowledgement, stale navigation and backpressure.
- Parent entries, recursive/self destinations, collisions and provider refusal.
- Real native handoff, release/cancellation and a fresh successful second drag.
- Ordinary copy/readback and native rename tests remain the integrity authority.

No dependency is added. Build and diagnostic artifacts remain under `_build/`.
