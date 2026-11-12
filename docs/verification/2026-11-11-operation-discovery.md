# Metadata discovery and truthful operation totals

Copy and permanent Delete prepare metadata counts before the first destination
publication or source deletion. The native readers use pinned directory identities,
handle-relative no-follow opens, and independent enumeration cursors. They read no
regular-file contents, load no complete files and never grant mutation authority.
Traversal retains handles proportional to depth (maximum 128 directories); Windows
buffers at most 64 KiB of names per enumeration batch. Cancellation and pause are
checked between native reads on backend workers. Paused preparation retains its
bounded worker/I/O slot until resume or cancellation; it does not start replacement
workers and may delay another job sharing that capacity.

`OperationDiscovery` distinguishes NotStarted, Scanning, Complete and Unavailable.
It records files, directories and optional logical regular-file bytes. Scanning or
partially failed counts never become a progress denominator. Missing sizes and
integer overflow remain unknown. Links count as entries but are never followed.
The metadata walk is an advisory observation of a mutable directory tree, not a
frozen execution plan. Existing source/target pins and native revalidation still
authorize every actual operation.

Native same-device Move retains its constant-time rename path. A batch containing
a native rename candidate uses SelectedRoots scope, as does Trash, whose OS callback
unit is a selected namespace root. These paths never enumerate a directory solely
to invent recursive progress. All-cross-device Move and ordinary native Copy use
Recursive scope. Providers without independent recursive discovery report
Unavailable and keep their established transfer behavior.

Copy progress separates logical completion from physical I/O: completed_bytes
counts each successfully published file length once, while copied_bytes,
verified_bytes and validation_bytes retain actual traffic (including retry work).
Current-file written and verified bytes come from the same retained work item as
its source and exact destination. A file-write meter exists only while a verified
file-copy worker is active; native rename and unresolved overwrite decisions keep
their source/target paths without claiming a file write. The destination is the pinned target directory
joined with the raw chosen basename, including conflict Rename choices. Partial
or skipped roots never count as completed_roots; completed_directories requires
complete traversal without a failed/skipped descendant.

Move destination publication precedes source proof and removal. It is not a claim
that the Move is finished; the UI uses source-removal/root boundaries and phase
information for that distinction. Native rename does not count as copied bytes.
Permanent Delete reports processed entries, not physical disk-space reclamation.

Job details retain completed discovery counts for files/other entries and folders,
plus exact localized logical bytes and a readable size. They explicitly distinguish
recursive observations from selected roots whose folder contents were not counted.
Unknown bytes remain labeled Unknown. Scanning/invalidated totals do not appear as
completed discovered totals; the reason for unavailability remains in Details.

Execution exceeding discovered counts/bytes invalidates the snapshot immediately.
A successful terminal Copy or Delete also invalidates a snapshot that differs in
either direction from completed work. Unavailable details explain the mismatch;
no stale larger total or ETA survives as an asserted complete count. Discovery
cancellation always returns the owned Job through normal finish/cleanup, preserving
bounded residual diagnostics from preallocated metadata stages or Move authority.

Targeted checks exercise native Unicode trees, raw link/non-follow behavior,
metadata of unreadable files on native Linux, independent copy iterators,
pre-write counts, exact target paths, fast native Move with zero content I/O,
source growth/shrinkage after discovery, skipped roots and cancellation with an
owned stage whose cleanup is refused. Run logs are retained in the current
`_build/qa/viewer-settings-20261111` qualification directory.
