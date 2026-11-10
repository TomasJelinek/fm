# Archive folder sizes from stored metadata

The accepted PFS-02 slice adds Ctrl+Shift+S directory sums and single-directory
Space requests for archive listings. File lengths come from archive metadata;
size scanning never reads or extracts member content. A selection limits a panel
request to its selected ordinary directories. An empty/files-only captured
selection is a no-op. With no selection, scan all immediate ordinary directories
and retain computed results for every visited subdirectory.

The native size command/cache path remains unchanged. Its PathBuf keys cannot
represent archive entries, whose native paths are intentionally empty. Accepting
an arbitrary ArchiveLocation would also allow a UI request to invent authority.
The approved alternative captures an exact TreeRootSnapshot through backend
SnapshotState::capture_tree_root and retains its ReadyRoot/mount on a worker.

Core ArchiveSizeCommand has Compute, Entry, Observe, Cancel and Acknowledge variants.
ArchiveSizeEvent publishes immutable Arc<ArchiveSizeSnapshot> maps keyed by
VirtualEntryId, explicit completion/rejection and backend SelectedFolderSizes
summaries. Versions increase monotonically. Consumers match live mount identity,
request and selection revision; they look up visible entries in O(1) and retire
old snapshots through the existing backend reclaimer. They never scan a large
result map on the UI thread to prepare selected status.

Each pane permits one immutable map in flight. Every received map, including a
stale mount's map, is acknowledged by exact pane/mount/version. Until then the
backend retains a small weak publication descriptor and updates its bounded
mutable cache; it does not clone more maps into the event queue. A final Finished
event can precede its deferred final snapshot. Snapshot acceptance therefore uses
current mount and increasing version, independently of an active scan ID.
Successful enqueue retains ACK ownership even if its wakeup cancels the scan.

Two persistent metadata lanes bound concurrency. Each pane admits at most eight
queued scan requests and one coalesced observer. A new full-panel computation
supersedes earlier same-pane size work; Space requests remain independent. Only
explicit cancellation, replacement or shutdown stops work. Sorting, selection,
filtering and same-mount navigation do not cancel or retarget scans. Updates are
coalesced to 250 ms, with an immediate final publication, and never trigger sorting.

An iterative traversal validates source identity, opaque member IDs and parent
relationships, rejects cycles/aliases, excludes links and uses checked totals.
Unknown lengths, unreadable subdirectories and overflow yield explicit partial
results. A stale source cannot publish or reuse complete cached values. Metadata
storage is bounded by the existing 100,000-entry archive budget; file lengths have
no new artificial ceiling. Two mount caches per pane retain bounded results
across ordinary same-mount navigation. Every final large allocation or mount
release belongs to a backend worker.

Observers and pending publications hold weak source authority. Only active or
queued scans retain a mount; leaving an archive after observation does not keep
its worker permit occupied. The scanner reserves every captured sibling ID and
ancestor before traversing selected roots, so malformed metadata cannot alias an
unselected sibling into a selected subtree. Same-mount newer observations adopt
results from older active captures; older different-mount output is suppressed.
Invalidated events identify the exact mount whose cached sizes must be retired.

An explicit archive Size sort uses finished, error-free directory totals from the
last published immutable snapshot. The backend records only weak map/source
references, atomically with successful event enqueue. Lookup requires the exact
same live mount Arc, with source validation before and after listing/sorting.
Equal numeric mount IDs cannot authorize reuse. Failed enqueue preserves the
previous publication; invalidation clears only its matching source. Partial,
unfinished and missing directory totals stay unknown-last in both directions.
The shared comparator accepts immutable size keys without changing Entry or
ArchiveNode metadata. Progress publication never invokes navigation or sorting.
The existing ArchiveNavigate command already carries the explicit sort request;
the UI supplies no new map or source authority.

The first integration accepts only entries in the exact captured flat root.
Expanded-tree descendant actions require their own captured typed tree authority;
an unsupported ID is rejected explicitly and never falls back to all/root folders.

The next expanded-tree slice should admit through Session::capture_selection on
the tree worker, preserving its exact acknowledged view/selection and bounded
CapturedTarget registrations/contexts. Display occurrences and overlapping
ancestor/descendant selections need explicit result and summary semantics.
Resolved archive targets currently carry fresh LeasedArchive wrapper Arcs; a
future canonical source identity must preserve those context leases and cannot
fall back to matching numeric mount IDs. This is a researched integration boundary,
not an implemented descendant-sizing route.

Salamander source inspected locally:

- `D:/experiments/salamander-main/src/fileswn7.cpp:1896`
  CalculateOccupiedZIPSpace uses selection/caret/all-directory modes and calls
  GetDirSize on stored archive directory data.
- `src/zip.cpp:6360` GetDirSize resolves the stored subdirectory and delegates to
  GetSize, treating an unallocated empty directory as zero.
- `src/fileswn7.cpp:1989` re-sorts after multiple-directory calculations in size
  order. This project deliberately keeps scan publication independent of sorting,
  following the current user requirement.

These are behavior references; no GPL source or assets are copied. Tests use owned
in-memory mount fixtures and assert that ArchiveMount::read is never called.
