# Copy from inline directory trees

This implements the accepted inline-tree and verified-copy requirements together.
F5 must copy the actual selected occurrences, including expanded native or archive
folders; an empty selection uses the focused ordinary entry. Parent `..`, context
ancestors and unsupported providers never become operation targets. The destination
remains an explicitly captured native folder, as in the existing copy workflow.

## Approach

Capture the exact acknowledged tree view and selection on the existing backend
actor before presenting the destination dialog. Retain an opaque, backend-owned
source batch through the dialog and queued operation. Resolving the current panel
after confirmation would lose the user's original selection on navigation;
flattening rows to display paths would lose archive and nested-folder identities.
These alternatives are unsuitable.

The public application contract contains a `CapturedCopySources` trait, opaque to
the UI, with cached root count/metadata availability and a worker-only `prepare`
method. The implementation owns captured provider registrations. At most four
captured batches may be live per engine. Cancellation or any target failure rejects
the whole capture. The existing 4096-target/4 MiB capture admission limits apply.
These are operation budgets, not limits on path or file length.

Confirmation acquires normal FIFO copy admission (32 jobs). A dialog does not
reserve a queue slot and block unrelated confirmed copies. Worker preparation
uses the existing retained transfer service so Retry cannot silently repin a
changed native object. Native destination ancestry checks and source protections
remain mandatory. Archive sources retain their mount and original member record;
a directory is copied with its basename rather than as unnamed contents. Archive
traversal identity includes mount identity, with aggregate traversal budgets.

Duplicate/overlapping selected tree roots are rejected before destination writes,
using provider identities and captured occurrence ancestry rather than display
prefixes. Mixed native/archive roots are supported by the adapter even though the
present UI does not yet attach archive containers inside native branches. Native
read-only preparation must not require destination/write authority on the source.

The existing copy worker performs recursive traversal, conflict decisions,
SHA-256 destination readback and publication. Preserve the existing optional exact
modified-time policy for supported native regular-file batches; unsupported
virtual/directory metadata requests fail before writes. F6 remains separate work:
this change must not turn archive Copy into Move or enable unsupported mutation.

Successful requested roots emit their original tree occurrence and session stamp.
The presenter removes only those selections through the ordered selection actor,
in bounded batches, without moving the caret. Failed/skipped/cancelled roots stay
selected. Sorting/reprojection uses occurrence identities; navigation discards old
completion feedback. Native/archive flat callers keep their existing behavior.

Provider handles, large captures and final lease destruction stay on backend
workers, including rejected sends, stale events, dialog cancel and shutdown.
No filesystem or checksum work occurs in UI callbacks. No plugin ABI or dependency
change is required. Windows/Linux adapters remain separate.

Salamander is a behavior reference for captured Copy selection, recursive roots,
conflict decisions and deselection, not a source of copied implementation code.
Reviewed `src/fileswn7.cpp::UnpackZIPArchive` (selection/caret, explicit `..`
exclusion, successful unpack deselection), `src/fileswn8.cpp` (Copy/Move operation
scripts and progress handoff), and `src/fileswna.cpp` (provider capability routing).
This application retains the user's more specific progressive successful-root
deselection requirement and asynchronous preparation instead of blocking the UI
while constructing a script.

## Acceptance

Exercise native nested siblings and archive nested directories through actual F5
dispatch, dialog confirmation and existing copy queue. Verify output bytes and
SHA-256 failure handling; stale capture, cancellation, Retry, queue saturation,
source replacement, destination-under-source and overlap rejection. Verify root
completion selection through the real presenter, including changed projections,
pending selection edits and navigation. Existing flat copy/move, metadata,
overwrite and tree tests must remain green on Windows and Linux.
