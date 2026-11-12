# File drag and drop

The original Copy-only milestone below is extended by the
[folder-drop and native-handoff design](2026-11-10-folder-drag-and-native-handoff.md),
which adds exact folder targets, native tree captures and Explorer-style internal
Copy/Move defaults. Native external protocol qualification remains separate.

The user requests dragging files between panels and between Midnight Explorer
and other applications. Implement copy semantics first, reusing the existing
background copy queue and independent SHA-256 destination readback. No drag may
authorize source deletion. An external destination application owns its own
operation; the host does not claim to verify that application's copy.

## User interaction

Start a drag only after a left-button movement threshold. A selected row retains
its selected group; an unselected row supplies its own captured target. Parent
entries, unavailable projections and stale rows cannot become sources. Preserve
click, double-click, Ctrl/Shift selection and touch scrolling. Escape or release
before asynchronous preparation completes cancels that gesture; a delayed result
cannot start a new drag after the user has moved on.

The first drop targets are the displayed directories of the two file-list areas.
Highlight the receiving panel, not a folder row. Toolbars, command/path editors,
dialogs, hidden panels and compact-mode activation rails are not drop targets. Respect swapped panels,
the splitter, DPI, window movement and current navigation. A future extension may
add explicit folder-row destinations, hover navigation and autoscroll.

## Native boundary and backend

Use a typed core service with separate Windows and Linux implementations. One
bounded, thread-affine actor owns the native source and target session. A prepared
source is identified by an opaque drag token. The engine resolves the immutable
selection off the UI thread; the UI commits that exact token only while the
gesture remains live. UI input-capture release belongs to small per-OS input
bridges and does not perform filesystem operations.

The target consumes coalesced snapshots of visible panel rectangles and captured
native destination/navigation identities. Closing, navigation and modal editing
invalidate incompatible targets. A native Copy acknowledgement requires immediate
reservation of actual copy-queue capacity and retention of the immutable request.
The pending drop keeps that reservation until the UI assigns its normal operation
ID and the backend transfers the reservation into the copy queue. Queue refusal,
shutdown and cancellation release the reservation. No independent ID namespace or
second unbounded queue is introduced.

Paste and admitted drops share the same operation-ID ordering: a later paste
cannot overtake a drop waiting to deliver its reserved admission. Cancelling such
a pending admission rejects its token and releases the reservation; it must not
send an early CancelCopy for a job the backend does not yet know. A transient
native source failure falls back only for that gesture, so a later drag can retry
the still-healthy native session.

Native payloads are untrusted: use the existing raw-path list limits of 4,096
entries and 4 MiB, reject malformed or non-local data as a whole, and preserve
unpaired Windows UTF-16 and raw Unix filename bytes. Neither serialization nor
target admission canonicalizes paths or follows links. The copy worker performs
its normal identity, containment, no-replace and checksum checks before success.
The drag service does not take clipboard ownership or invoke a shell command.

## Platform implementation

Windows uses an OLE source/target session with a message-pumped STA, native file
lists and Copy-only effects. Disable winit's built-in target for that window.
Registration, callbacks, cancellation and cleanup must be verified against the
actual UI-owned window; cross-thread behavior is not assumed from a unit mock.

Linux initially uses X11/XWayland XDND with a worker-owned proxy window and
byte-preserving `text/uri-list` data. The tiny UI input bridge releases its own
Xlib pointer grab before the source actor acquires one. Transfer pumping, parsing,
timeouts and ownership remain on the backend. Native Wayland external drag/drop
is unavailable until the same-connection seat serial/data-device boundary is
implemented; it must not silently fall through to an unsafe parser. Internal
panel dragging remains usable without external native support.

The verified proxy advertises XDND v5 and temporarily removes the host's own
XdndAware property. Sources must support proxy discovery; legacy sources that
ignore it are deliberately refused. A proposed Move may negotiate Copy only when
the source's bounded XdndActionList explicitly includes Copy; Move-only offers are
rejected. Both Status and Finished identify Copy, so no source-delete request is
ever sent. Preserve and restore prior native properties only while owned by this
session. INCR transport, action/type lists and outstanding requests remain bounded.

The pinned Slint 1.18.1 core provides DragArea/DropArea and data-transfer types,
but its winit backend does not implement native outgoing drag. Winit 0.30.13's
X11 incoming path parser decodes to UTF-8 and canonicalizes on the event thread.
These paths therefore cannot supply this application's lossless, backend-only
file transfer contract. Do not patch vendored dependencies or add GTK merely to
obtain drag/drop in a winit window.

Salamander behavioral references: `src/fileswn0.cpp` target registration,
`src/shellib.cpp` target/source effects and payload processing, and
`src/shellsup.cpp` copy/move dispatch. No source or assets are copied.
Primary platform references: [RegisterDragDrop](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-registerdragdrop),
[DoDragDrop](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-dodragdrop),
and [XDND](https://www.freedesktop.org/wiki/Specifications/XDND/).

## Verification

Test captured groups, filtered/stale views, quick release, Escape, delayed
preparation, queue saturation, source retention, same-folder/recursive-copy
rejection, collisions, raw names and malformed/oversized offers. Exercise actual
native protocol registration and data transfer, not only synthetic UI callbacks.
UI tests cover swapped/zoomed panels, changing geometry, hover feedback, input
capture and click/double-click preservation. Verify copy results through the same
readback tests as clipboard copies. Document native desktop dependencies and any
unsupported transport explicitly.
