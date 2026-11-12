# Tree controller lifecycle

The global `Command::Tree` and `Event::Tree` variants connect the production
Engine to one persistent model worker per pane. The presenter adopts the exact
published view and retires obsolete tree payloads through its background
reclaimer, including archive navigation targets that own a mount. This lifecycle
contract does not establish container attachment or descendant operation support.

## Authority and publication

Native and archive completion cache their exact entry and index allocations
before publishing the completed listing. Success and failure notify the pane
worker directly; there is no root polling loop. Enable admission captures that
exact completed root through `SnapshotState::capture_tree_root`. Equal contents,
restored paths and foreign index allocations are insufficient.

An accepted session publishes its projection and selection together. A later
Enable may update the root selection before Commit. Branch, viewport, selection
and typed navigation require the exact acknowledged projection. The mailbox
also rejects allocations other than its last published view before queueing
them. Every worker publication checks the current navigation cancellation;
session application checks the pane, navigation, session and branch version.

Typed navigation preserves full native paths or archive locations and raw node
identities. An archive target retains the source registration after pane changes
and engine shutdown. Each pane uses one persistent registry, so retaining targets
across sessions cannot evade its 64-mount limit. Files and unopened containers
produce an explicit unsupported response. Native immediate-parent mutation
proofs and flat action admission remain unchanged.

## Persistent filtering and sibling sorting

`Reproject` carries the exact acknowledged projection, the settled selection Arc
and revision, a fresh explicit request ID, and bounded filter/sort options. The
mailbox rejects foreign projection/selection bases before queueing. The model
worker checks the exact live selection and request high-water mark again; a
branch publication or newer selection can make an already queued request stale.

Reprojection uses retained rows, cached identities and the existing trusted
locale. It does not enumerate, extract, validate archive sources or resolve new
provider targets. Matching children retain nonmatching ancestors as unselectable
context. Sorting stays within sibling groups. Clearing preserves the expression
and expanded model branches without restoring marks removed by filtering. The
complete original root entries/index remain the authoritative root snapshot.

Candidate projection bookkeeping, options and rebased selection are prepared
privately on the model worker. Validation, compilation, resource or cancellation
failure leaves the previous accepted publication and occurrence records intact.
Successful replacement emits one correlated `Reprojected` event containing the
applied options and atomic publication. The mailbox registers that publication
exactly as for `Published`; the presenter must send its exact `Commit` before
further tree input. Failure emits `ReprojectFailed` with the captured stamp and
request ID, including queue rejection. It never fabricates a row-oriented error.

The presenter owns draft/committed/applied presentation state. Backend success
means the options were applied in this session; it does not mean preferences
were saved. Native/archive/Network navigation and shutdown retain the ordinary
generation cancellation rules and prevent canceled option publications.

## Resource limits and backpressure

- Each pane has eight queued commands, one pending Enable, one coalesced reset,
  one coalesced root-ready notification and one branch completion slot.
- Enable preflight checks both retained allocation sizes and index capacities.
  A root has fewer than 10,000 entries because the hidden root also consumes a
  model occurrence. Each root/projection has the existing 16 MiB payload limit.
  Queueing a native selection requires its exact root index. Archive seed hash
  capacity is at most 20,000; viewport hints have at most 256 rows, and selection
  ranges have at most 4,096 spans. These bounds keep the eight-command payload
  queue below 132 MiB per pane even when all shared allocations are distinct.
  This is a queue bound, not a bound on the application's complete flat listing,
  event transport, provider caches or separately retained operation targets.
- Each Session has eight pending branch requests, coalesced by branch. Only one
  branch job per pane can be outstanding. Its persistent budget survives session
  replacement and shares the pane's existing two listing permits.
  Viewport probes are best-effort read-ahead: excess probes wait for a later
  publication/viewport update rather than reporting resource exhaustion. Explicit
  expansion or refresh can replace a queued probe and runs before background
  probes. A queue containing eight explicit requests retains normal backpressure.
- Root construction and typed navigation validation hold a common listing
  permit. Branch work retains both its branch and common permits through actual
  provider return, completion application/projection and stale-result cleanup.
  A saturated typed navigation request is rejected explicitly and may be retried.
- Queue or payload saturation returns `Unavailable`, `Rejected` or
  `SelectionFailed` as appropriate; options use `ReprojectFailed`.
  Reprojection retains at most 4,096 UTF-8 expression bytes per queued command;
  valid expressions with excessive spare allocation are normalized on one
  awaited backend worker before queueing. Invalid payloads retire through the
  same bounded cleanup path. Selection storage must use the exact projection
  basis and its bounded bit count. Candidate projection bookkeeping is one
  additional bounded worker-owned copy, released on failure or replacement.
  Root/provider failure preserves the complete
  flat listing. Rejected large payloads are released by one awaited blocking
  cleanup task; rejection cannot accumulate detached cleanup workers.

## Cancellation and retirement

Native, archive and Network navigation cancel the generation directly before
notifying the model worker. A competing Network generation also prevents an old
native/archive completed root from being admitted.
Branch cancellation is linked to that generation, so it reaches provider code
even while the model worker is blocked in another provider callback. Collapse
cancels its branch independently. Cancellation never substitutes a fresh permit
for a still-running callback.

Model construction, projection, target resolution and session/command/completion
retirement run on blocking workers, outside snapshot and mailbox mutexes. Branch
jobs run separately so collapse and navigation can be processed while enumeration
is blocked. Root-ready messages contain no provider resources.

Shutdown or disconnected command input closes both lanes and cancels their
generations. Idle model workers terminate and drain their queues. Controller
shutdown waits at most 200 ms per worker; an uncancellable provider callback
retains its bounded worker and resources until its actual return. A late branch
completion retires directly on its branch worker when the lane is already closed.
Accepted navigation targets retain their independent lifetime until the receiver
releases them through its normal background reclamation path.
