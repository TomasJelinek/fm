# Native inline-tree watch and refresh continuity

Reviewed design, 10 November 2026. Implementation is already authorized;
the architecture review and resolutions are in `_build/qa/tree-watch/`.
This document does not claim completed implementation or verification.

## Behavior and boundary

Manual Refresh, ordinary root filesystem hints, expanded-branch filesystem hints
and terminal Copy/Move results use the existing native in-session
`TreeCommand::RefreshLoaded` maintenance sweep. They preserve the adopted session,
original admission snapshot, surviving occurrences/marks, expansion, caret,
viewport and accepted sort/filter/search options. Navigation still retires the
session. Flat, archive and Network Neighborhood refresh keep their existing
behavior. An enable request that has not been adopted still has flat behavior.

Watch the native root and loaded ordinary-native directories that are expanded
in the accepted visible projection, including necessary filter context ancestors.
An expanded directory outside the scroll viewport is still part of the panel.
Hidden/filtered-away branches, collapsed branches and their descendants have no
hot watch. Re-expansion already performs a fresh listing; preserve that behavior.
Watch events never enumerate unopened branches or grant action authority.

Keep the existing parent-first sweep of all retained loaded native branches,
including collapsed caches. A smaller hot-watch scope does not change maintenance
scope. Each maintenance listing and its installation retain the mutation gate;
observation probes do not publish rows and do not need that gate. Ordinary user
Expand and flat listing keep their current semantics and responsiveness; this
work does not make them wait for an entire copy job.
If preferred maintenance cannot acquire the mutation permit, the scheduler falls
through to runnable explicit branch work using its ordinary permits. Its wait
selection must reflect that fallback; same-branch precedence stays intact.

A listing is live, not an atomic filesystem snapshot. Failed reads retain known
rows, report a bounded diagnostic and remain eligible for paced watch retry.
No destructive consumer, archive watch, plugin ABI or recursive filesystem watch
is introduced.

## Small public interfaces

Extend `midnight-core/src/watch.rs` without removing `watch_directory`:

```rust
fn watch_directories(
    &self,
    paths: &[PathBuf],
    changed: Arc<dyn Fn(u32) + Send + Sync>,
) -> Result<Box<dyn WatchSubscription>, FsError>;
```

The default returns Unsupported. Input contains 1..=32 unique native paths;
bit `i` identifies `paths[i]`. Reject an invalid/empty/oversized batch before
native registration. Errors, overflow/rescan, empty paths or unrecognized event
paths report every active bit. Partial registration fails the whole batch and
cleans up on its backend caller; the engine polls that scope instead. Keep the
single-directory API working for existing callers. The platform facade currently
re-exports the OS filesystem directly, so the new method is available through
that existing facade; any additional delegating adapter must forward it.

Add these UI-facing tree events in `tree_protocol.rs`:

```rust
TreeEvent::Changed { stamp: TreeViewStamp }
TreeEvent::WatchFailed { stamp: TreeViewStamp, error: TreeDiagnostic }
```

It is a dirty hint from an accepted native watch scope. The UI matches pane,
navigation and session; projection revision can advance while the hint waits.
No paths or internal dirty epochs cross this event. `TreeRefreshRequest`,
`Refreshed` and `RefreshRejected` remain unchanged.
`WatchFailed` reports a bounded scope/epoch diagnostic without rejecting Commit
or setting `commit_pending`. Match its navigation/session and deduplicate an
unchanged failure across selection Commits until failure changes or retry is due.

## Native subscriptions and resource ownership

Windows and Linux implement batches in their separate `src/watch.rs` files.
Each batch owns one `RecommendedWatcher`, not one watcher per path. Every watch
is nonrecursive. Prepare native registration paths on the backend caller;
callbacks perform no canonicalization, stat, content reads or path allocation.
Match at most 64 supplied event paths against at most 32 registered targets;
more paths conservatively set all bits. A path naming a watched directory itself
or an immediate child can set its bit; overlapping targets can set multiple bits.
Ignore ordinary access events while preserving rescan/error hints. Existing
native callback gates serialize short callbacks with cancellation. Dropping a
subscription disables its callback before native worker teardown.

Each pane has root at slot zero and up to 31 branch slots. Preserve eligible
branch slots by exact session/branch/target identity; fill vacancies in accepted
projection order. Rebuild a changed batch on that pane's watch worker, preserving
dirty counters and observation history for surviving targets. One batch per pane
means at most two native watcher owners. Failed batch creation uses polling for
all eligible targets, with no loop that creates single-directory replacements.

Retain the existing two backend watch/probe workers, one serial probe per pane.
All eligible branches, including those without a native slot, participate in fair
round-robin polling. Each target becomes due after 15 seconds; a blocked/large
earlier probe can increase latency, so do not promise a fixed 15-second update
bound. Native notifications use one-second quiet debounce with a three-second
maximum; failed/ignored work retries no faster than the poll cadence unless a
new native event or explicit user request arrives. Polling must not starve due
targets under continuous native events.
The debounce window is not an unconditional delivery bound: a blocking probe,
registration or full output queue can delay it. Check control/due hints between
probes; do not add replacement threads to conceal that latency.

The scope contains at most the existing 10,000 tree-node budget and at most
32 MiB of retained native target path storage, with constant-size signatures per
target. This is a watch resource budget, not a filesystem path-length limit.
Exhaustion is an explicit diagnostic with the degraded-scope policy below, never
silent omission of eligible branches. Streaming probes retain one row at a time,
check cancellation per row and never retain vectors of every descendant file.
Native calls that do not return retain their worker slot and provider ownership;
cancellation does not create replacement threads or permits.

## Exact accepted scope and authority

Add a separate cloneable `watch::Handle`; do not clone `Controller` with its
shutdown-on-Drop behavior. Inject the optional handle into tree-controller workers
from `engine/lib.rs`, where watches are constructed before trees. The handle
updates small bounded control state and wakes watch workers; it performs no I/O,
full fingerprints or subscription destruction on the caller.

On a successful exact `Session::commit`, the model worker builds an accepted
native scope. Root uses the current `view.root_actions`; original `view.root`,
`p.entries`, `tree_root` and flat committed-operation caches remain untouched.
Branch membership comes from the exact accepted projection, while expected child
entries come from the model's complete last accepted branch listing, before
filtering. Resolve targets through retained provider `NodeRef`/`BranchId` and
mounted target validation; displayed captions/paths and row positions are never
input authority. Exclude directory links and any attached/foreign/archive source
or ancestor. A native share participates only after explicit activation has
established its ordinary native location.

Each scope retains its provider registration lease and mounted targets until
probes/subscriptions really finish. Release all replaced/retired scope resources
on backend workers. A same-session scope update keeps surviving target histories;
selection-only Commit with unchanged root-actions allocation and membership must
not reset subscriptions, debounce or baselines. Use a monotonic control revision
to reject delayed updates; navigation cancels the previous scope immediately.
Delayed flat `PaneCommitted` for the adopted navigation cannot replace tree
scope/baseline. A later actual native navigation resumes ordinary flat ownership.

Scope construction failure does not undo a successful Commit. Compute bounded
eligibility identities before fallible target/signature construction. On failure,
disable ineligible targets immediately and retain only the prior same-session
eligible subset/history; if eligibility cannot be established, disable all old
targets. Never restore flat or previous-session authority. Keep the accepted
pane/navigation/session and paced retry state even with zero admitted targets,
so a later maintenance/Commit can reconstruct scope. Emit `WatchFailed`, do not
acknowledge a sweep ticket, and retry at poll cadence. Cache eligibility, accepted
listing identity and signatures so unchanged selection Commits do not rebuild or
rehash the scope just to discover it is unchanged.

Navigation/close logically invalidates callbacks synchronously through the
controller, even when a model/probe worker is blocked. Move retired owners out
under the control lock and destroy them after unlocking on a backend worker.
Keep only bounded active/latest-pending scope ownership; never accumulate an
unbounded retirement queue behind a stuck probe.

## Display and observation signatures

Use existing pinned `sha2` 0.10.9; no dependency change. Fingerprint original native
names and relevant raw metadata: kind, file logical size, modified time,
compression, native attributes and Unix mode. Ignore computed directory size,
localized captions and presentation-only fields. Encode fields with explicit
tags/lengths/presence; native names use the same lossless platform representation
for accepted records and probes. Never hash a replacement caption.

Compute each row's SHA-256, add row digests modulo 2^256 and retain an exact checked
u64 row count, with domain-separated final SHA-256. This constant-space multiset
signature is independent of enumeration order and retains duplicate count. It is
only change detection, not authenticity or operation identity. Counter overflow,
bad immediate-child identity or incomplete metadata probe is an error.

Every target has two signature domains. Root's accepted display signature includes
all root entries; branch accepted display signatures include Directory and
DirectoryLink rows, matching `BranchWork::list_folders`. Link rows participate in
change detection but never become traversable watch targets. Probe metadata does
not follow the final link. A separate full immediate-content observation
signature includes files, so file-only edits can invalidate recursive size values
even in a branch displaying directories only. Do not compare a full observation
against a folder-only accepted signature.

Retain last successful full observation across accepted baseline updates. A probe
changes the dirty epoch if its full observation changed or its display signature
differs from the accepted display signature. It may advance last-observed state
immediately because dirty state persists independently until acknowledged. Failed
probes retain signatures and mark a paced dirty/error interval. A stable mismatch
or stable failure retries at poll cadence, rather than generating a tight loop.

Enroll a new target by enabling its callback/cancellation generation before the
first observation. The first observation always records one dirty interval,
including polling-only enrollment; this conservatively closes changes between
listing acceptance and first probe and invalidates potentially old sizes once.
Preserve callbacks recorded before/during this probe. Retained targets do not
re-enroll on baseline changes. Batch replacement preserves their state and records
one dirty interval covering its subscription gap. Observation-only state never
replaces accepted rows or supplies file action authority.

## Dirty epoch and successful sweep acknowledgment

The watch controller owns a checked monotonic dirty epoch per pane/session. Native
callbacks increment it immediately while OR-ing their bounded bitmask, then wake
the worker. Poll changes/errors increment the same logical epoch. Exhaustion
reports an explicit error requiring reopening the scope; never wrap identities.
Scope replacement preserves the epoch for the same session. The control lock or
equivalent atomic protocol must serialize capture/ack with callback increments.

When the model worker successfully admits `RefreshLoaded`, capture the current
watch epoch into a private ticket correlated with its session/request. No ticket
is consumed on rejected admission. On sweep failure/abort, leave its dirty epoch
pending and set the next retry deadline. On success, record the final projection
revision as an acknowledgment fence. Clear only that captured epoch after a
subsequent `Commit` succeeds for the exact current projection of that session,
whose revision is at least the fence. New callbacks/polls after admission survive
that acknowledgment, including callbacks arriving between listing and Commit.
An intervening sort/filter/selection projection can satisfy the fence only after
its own exact Commit. Navigation retires the ticket without affecting a new scope.

Keep this ticket state private to backend watch/tree workers. Suggested handle
operations are `set_tree_scope(scope)`, `begin_refresh(stamp, request) ->
Option<RefreshTicket>`, `finish_refresh(ticket, success, final_revision)` and
`clear_scope(pane, navigation)`. Exact successful Commit supplies the accepted
scope and checks pending success fences. These operations exchange bounded state;
watch worker output sends and probes never hold the control lock. A full output
queue cannot prevent callback cancellation, navigation or releasing a completed
maintenance mutation permit.

## UI, selection and sizes

Route adopted native manual Refresh and legacy `DirectoryChanged` into the same
dirty scheduler used by terminal transfers. Handle `TreeEvent::Changed` inside
ordinary tree event application, validating navigation/session and queueing the
same scheduler. A full command channel retains pending dirtiness; never fall back
to flat navigation. Keep acknowledgment/selection/options fences in
`flush_tree_refresh`. Watch retries are backend-paced; the UI does not retry a
terminal provider failure continuously.

Surviving occurrence marks and caret follow existing structural publications.
Deleted/hidden/context-only occurrences cannot inherit selection; old flat marks
must never be restored. Existing pending exact toggles and completion deselection
settle before maintenance. Failed listings preserve rows/marks and show an error.
New root files gain F3/Enter through current `root_actions` aliases only.

Accepted `RefreshLoaded` already cancels/invalidates tree size work and frozen
Size-sort values. Reuse that conservative policy, including file-only hints. Late
retired size results cannot revive values; actual worker capacity is retained
until return. Do not automatically restart recursive size scans on every event.

## Verification and handoff

The current native transfer harness never enables watches and sends all events
through `apply_with_sender`, which intentionally ignores legacy
`DirectoryChanged`. Qualification must add real `configure_watch()?.install(...)`, accepted
`PaneCommitted` synchronization and production hint routing, then exercise F5/F6
past their own debounced native events. Include external root/expanded-branch
create/remove/rename, file-only edits, manual refresh, collapse/change/re-expand,
stale scopes, independent panes, masks/overflow, polling fallback, blocked
callbacks/probes, full output and provider teardown. Verify retained sessions,
marks/caret/options, correct bytes and new-file viewer authority on Windows and
Linux. Software-window results do not qualify GPU/installed desktop behavior.

Use [the implementation plan](../plans/2026-11-10-tree-watch.md) and
`_build/qa/tree-watch/progress.md` for ownership/build leases. Fresh architecture
review of this draft precedes production edits. The root coordinator owns UI and
Windows Cargo initially; the backend implementer owns core/engine/platform.
The HTML agent uses only its independent Linux plugin target. Preserve dirty work,
backend-only I/O, platform separation and the project's no-mandatory-fee policy.
