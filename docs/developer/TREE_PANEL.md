# Inline branch panel integration

The ordinary panel offers an unknown-folder disclosure beside directories after
an accepted native or archive listing. The first explicit disclosure requests a
backend tree session. Until then the panel keeps its existing flat behavior and
operation scopes. Network discovery rows do not offer branches; an explicitly
activated native share follows the native listing path.

`presenter/tree.rs` retains the original entries and index, and requests Enable
with those exact allocations and the accepted selection revision. Pending flat
selection must settle before migration. A publication seeded before a later
selection is rejected and retried; it cannot erase newer marks. Initial adoption
uses the backend root-alias map to preserve the caret. Every publication is
adopted with its matching selection and acknowledged with its exact projection
Arc before branch, navigation, selection, or viewport requests.

Each emitted projection waits for the UI to acknowledge its exact Arc before a
new unsolicited publication replaces it. At most one newer publication is retained
and coalesced on the model worker. A completed provider task releases its mutation
permit even while publication delivery waits. The mailbox retains a weak receipt
for the acknowledged view: Branch and Navigate gestures may still name that view
while a newer publication is travelling to the UI. The actor revalidates that the
same occurrence remains visible and unchanged; removed, replaced, hidden or
foreign occurrences are refused. Selection and filesystem-operation captures
still require the current exact projection.

Older automatic Commit/Viewport observations in the same pane, navigation and
session are silently retired at mailbox admission and worker dispatch. New
publications trigger fresh observations. Replaced publications and their owners
are retired on the model worker, never by a UI callback. These rules prevent an
unrelated probe completion from being reported as command-queue saturation.

The eight-slot command mailbox coalesces viewport observations to the latest one
in the same pane/navigation/session. An explicit user intent may evict that
speculative observation when the mailbox is full; it never evicts another user
intent. Optional viewport pressure is silent, while genuinely full explicit-intent
queues and stale-view refusals have different diagnostics. Retired observations
are released on the awaited backend cleanup worker. Neither capacity nor the
filesystem authorization checks are relaxed.

`Rows` reads the immutable projection directly. It does not construct a second
complete UI model. Occurrence/session/revision tokens bind disclosure callbacks;
root and descendant entry IDs never become interchangeable. Details, Brief and
Thumbnails use the existing shared disclosure widget. Loading, unknown, collapsed,
expanded, empty and failure states remain distinct. Structural publications reset
delegates and cancel rectangle gestures. Collapsing a focused descendant moves
the caret to its surviving visible ancestor. Loading updates preserve the panel
viewport.

Details and Brief reserve the same disclosure gutter for every row, including
files and known-empty directory links without an expansion button. Supported
directory links, including Windows junctions such as `Documents and Settings`,
can show an expander under the same disclosure rules as ordinary folders; their
system link overlay remains visible. Thumbnail artwork retains its centered
layout.

Tree selection uses separate backend snapshots and at most 32 small pending
intents for immediate visible feedback. Ctrl/Shift input, Insert/Space toggles,
bulk selection and rectangles submit occurrence-based requests. Context rows
cannot be marked. Loading publications replay outstanding intents after Commit;
a structural change retains only still-visible exact toggles, because replaying
an old positional range could select rows the user did not see. Rejected
selection releases its preview without reusing the request revision.

Ctrl+A replaces selection with selectable top-level files (`depth == 0`);
Ctrl+Shift+A also includes top-level folders. Both clear nested marks. Ctrl+D
clears every mark, including folders, directory links and expanded descendants.
Ctrl+Shift+D has no default assignment. The shared row policy drives both
backend results and immediate preview, so child rows never flash selected while
waiting for acknowledgement. Filters still exclude hidden/context-only rows;
explicit click/range selection of descendants remains available.

Enter and unmodified double-click submit typed tree navigation for directories.
Returned native and archive targets use the existing navigation path after
checking the request, pane, session, view revision and next navigation generation.
Regular files from the original root retain their explicit host/archive-open
behavior by resolving `original_root_entry` against the exact retained root
index and entries. Descendant IDs, captions and display paths never authorize
that shortcut. Bounded visible-row hints reuse the existing viewport sampling
and never include more than the backend's 256-row probe budget.
Those hints are scheduled in batches within the eight-request branch queue.
Showing more than eight folders is not a resource error. Completed publications
request the visible range again, so deferred folders are checked progressively;
an explicit expansion takes priority over queued background probes.

## Prefix search and tree options

Persistent filter and sort commands now route through the production tree worker.
`presenter/tree_options.rs` correlates each request with its original projection,
selection and request identity. The presenter commits sort/filter preferences only
after accepting the matching publication, and preserves the last valid result and
retryable filter draft on rejection. Clear disables the filter without discarding
its expression. Masks, Regex and Expression retain unselectable ancestor context;
sort order applies independently to sibling groups without reopening the root.

Typing in an adopted native/archive tree now uses the panel's literal prefix
search and optional quick-search filter. Matching examines loaded visible tree
rows on the backend; typing does not enumerate unopened branches or extract
archive content. The shared Unicode matcher folds valid original native names
and UTF-8 archive raw names, without normalization. Opaque names cannot match a
replacement or alternate caption. Highlighting ends at an original grapheme
boundary and is shown only when the displayed caption equals that original name.
The backend supplies ordered match positions with the projection.

Jump mode preserves selection. Filter mode intersects the prefix with the
persistent filter, retains only matches and their necessary ancestor context,
and removes hidden/context-only marks in the same publication. Context ancestors
never count as matches or selection targets. Zero matches and pending queries
cannot authorize keyboard activation through an old caret. Escape restores the
persistent-filter view and its last usable focused occurrence. Insert waits for
the cleared query's committed projection and settled selection before toggling
its captured matching occurrence; stale navigation or occurrence state cancels
that intent.

The presenter retains one latest desired query and at most one options request
in flight. Later typing, Escape, sibling sorting, persistent filters and the
quick-filter preference compose through that request, including Options preview
and Cancel. A typed transient rejection retries on a later input pump; outer
command-channel pressure also preserves the latest desired query. Correlated
receipts and exact projection/selection identities prevent an older query or
branch completion from focusing its obsolete first match. Query admission is
limited to 4,096 UTF-8 bytes; invalid requests preserve the accepted view.

Extension sorting now uses a shared borrowed filename accessor for key preparation
and comparison. Native entries use their actual path basename; archives use their
raw UTF-8 member basename without inventing a native path from a caption. Locale
extension/name keys, directories-first ordering and deterministic ties remain;
missing or undecodable archive extensions stay last in either direction.

The [10 November prefix-search report](../verification/2026-11-10-tree-prefix-search.md)
records the current verification and deployment scope separately from the earlier
filter/sort evidence below.

## Subfolder presence

Flat-panel subfolder hints are independent of tree adoption. At most 256 viewport
directory IDs are probed against the exact completed listing, with one coalescing
worker per pane, the existing listing capacity and shared I/O budget. Native
enumeration stops at the first immediate directory or directory-link child; it
does not descend into that child or recursively walk link targets. Archive probes
retain the captured member and mount identity.
Empty/has-folder hints are held in a bounded display cache. Selection, flat action
authority and navigation stay unchanged until the user's ordinary disclosure
click requests exact tree adoption. Navigation, refresh and viewport replacement
cancel obsolete probes. Failed probes preserve provider details for the existing
warning tooltip and modal listing-error notice, and never become an empty-folder
result.

Presence is an existential display hint, not a validated child listing. Native
probes use the injected directory service, validate each immediate child path and
stop at the first folder without retaining file rows, issuing node tokens or
enriching file metadata. Repeated file records cannot change the answer; actual
expansion retains the full provider identity and duplicate-node validation.
File-only directories have no arbitrary record-count cutoff. Cancellation and
the worker/I/O limits bound concurrent work; archive probes retain their existing
captured-mount validation and resource policy.

Native flat-panel admission now accepts `DirectoryLink` alongside ordinary
directories for the exact captured native root. Previously, one directory link
could discard the entire viewport batch. Archive batches still reject links and
files; empty or unavailable native links retain individual hints without losing
ordinary siblings' results. The admission regressions first ran **6 passed /
2 failed / 0 ignored**. The corrected presence gate passes **12/0/0** (admission
8 + provider 4), with compile 0 and zero source drift. The
[current presence and Find gate](../ACTIVE_WORK.md#native-link-presence-and-find-gate)
records the newer receipts. The corrected Windows debug build and registered
private entrypoint pass engine/native UI startup. This admission correction does
not add recursive presence probing or establish protected CSC access.

<a id="planned-retry-diagnostics-and-recursive-expansion"></a>

## Retry diagnostics and recursive expansion

CSC warning retention was implemented in the committed/pushed UI checkpoint
`20f507c`, with **17 passed / 0 failed** targeted presenter tests. The separate
Engine authorization tests at `3826298` passed **5 / 0**. The
[authorization verification note](../verification/2026-11-11-directory-authorization.md#12-november-csc-warning-retention-regression)
records both source/test boundaries: mocked authorization, event retention and
helper denial/cancellation/transport. Successful native CSC access through Windows
UAC remains unproved. The [current checkpoint](../ACTIVE_WORK.md#current-checkpoint)
records separate application build/startup qualification; source correctness and
startup do not establish access to protected CSC.

A denied row, including `C:\Windows\CSC`, retains its warning throughout an
explicit elevation retry until the matching current listing actually succeeds.
Authorization/helper admission is not listing success. Normal navigation failure
restores the original committed cursor and scroll position; cancellation or
another denied result preserves the original row's diagnostic. Retention is
display-only and bound to the exact committed navigation, entries/index and raw
row identity. Replacement rows cannot inherit it. Request generations remain
monotonic and action/session gates stay intact; late presence/tree events cannot
clear or mark another occurrence.

**Ctrl+click on a folder expander** routes to
`ExpandRecursive`, including directory symlink/junction expanders within the
existing model limits. It uses one asynchronous walk per pane through the existing
actor's depth-first continuation. The UI captures the modifier and submits the
typed intent; traversal stays on Backend workers. Ordinary row Ctrl+click retains
selection semantics. Recursive expansion is implemented and tested on Windows
with software UI coverage. The integrated gate passes **69 tree/CSC UI tests**:
62 presenter cases (tree 44, folder presence 9, elevation 9), five disclosure
cases and two panel cases. Compilation exits 0 with zero source drift; these
groups overlap the earlier CSC tests and are not added to them. See the
[recursive verification note](../verification/2026-11-12-recursive-tree.md).

The recursive worker batches at most **16 returned results** or **50 ms**, with elapsed
time checked only after an actual provider callback returns. These are scheduling
limits, not a callback timeout or measured latency guarantee. Each batch creates
one checkpoint/projection and waits for its exact Commit before further recursive
I/O. Explicit input first rolls back unpublished
model/continuation state; allocated branch/work IDs and row/alias high-water values
remain consumed and monotonic.

Collapse or navigation cancels obsolete traversal without releasing an in-flight
provider permit early. Ancestor listing-version checks precede dispatch of an
already-stacked child: a Refresh that reorders, inserts or removes children pauses
the walk instead of resuming a stale DFS cursor. Backend compares physical target
identities along the ancestor chain for browse-only cycle detection. Logical paths
and captured-occurrence checks remain in force for operations; a cycle-probe
identity grants no operation authority. Cycles, denied children and other errors
remain local to the affected branch. Known-empty directory links have no synthetic
`+` expander.

Allocation, work/publication and stale-occurrence guards remain unchanged.
Capacity saturation preserves the valid published tree and reports paused/
incomplete expansion without a retry spin. Paging beyond the retained model is
an explicit follow-up. Architect owns the API contract, Backend owns traversal/
cycles, and UI owns modifier/state routing. Backend's broad first run retained one
legacy fixture failure; its refined Windows junction/Delete subset passes 11/0
with unchanged production. Those exact scopes and the separate thumbnail RED
are recorded in the verification note. The current registered private Windows
runtime passes engine smoke and native UI startup; its exact EXE identity and
startup-only qualification are recorded in the
[current checkpoint](../ACTIVE_WORK.md#current-checkpoint). Linux now has separate
WSL/tmpfs receipts for the original `1714d99` fixture and the updated frozen
link-maintenance fixture, each **1 passed / 0 failed / 0 ignored** under its own
source scope. The corrected helper's separate clipboard/copy/move gate passes
**13/0/0** using software UI and real filesystem fixtures. These later results do
not establish Linux desktop qualification or broaden the earlier test scopes.

The corrected [tree scaling candidate](../ACTIVE_WORK.md#tree-scaling-candidate-gate)
passes 95 unique functional cases and Clippy with zero drift, but the debug
opt-level 0 synthetic-provider aggregate **fails**; it measures no filesystem I/O
or UI paint. Wide 10k cases take **50.83 s** (`K=0`) and **50.01 s** (`K=1`);
the depth-32 broom is censored at **60 s**, with **9,936 callbacks / 9,920 published
entries**. The initial stale compiler artifact was rejected before timing.
The current `fd94d85` startup-qualified runtime predates this candidate; no
large-tree speed or UI input-to-paint qualification follows.

## Directory sizes

Ctrl+Shift+S uses the exact acknowledged tree selection: selected directories
and native directory links, or every selectable folder in the current projection
when there are no marks, including descendants in expanded branches. This means
all projected rows, not just those inside the scroll viewport. Hidden/collapsed
descendants and context-only filter ancestors are excluded. A
files-only selection does not fall back to all folders. Space captures the focused
directory before its selection toggle, then advances the visible caret while the
backend measures that original occurrence. Insert never starts measurement.

The backend retains typed native/archive targets and their provider registrations.
Native traversal reuses the existing storage-aware batch scanner and shared I/O
budget. Archive traversal reads metadata with each captured directory's exact
ancestry; overlapping selected ancestors and descendants keep independent totals.
It does not extract archive content or fabricate native paths.

Size values and running flags arrive in bounded immutable snapshots keyed by
`TreeRowId`, separately from the row projection. Selection, sorting, filtering
and collapse preserve accepted work; progress never automatically resorts rows.
Explicit Size sorting freezes the available size lookup for that projection.
Refreshing an occurrence invalidates old measurements and frozen sort values;
navigation retires the whole scope. Native totals remain live path-based estimates,
not a transactional filesystem snapshot.

Highlighting follows each occurrence's active calculation flag, independently of
warning severity. Only actively unfinished values remain highlighted estimates.
A completed scan with inaccessible entries shows its observed partial total with
`≈` in normal metadata color and weight, even while another root is calculating.
Its yellow warning glyph and clickable details remain beside the size. An inactive
unfinished or cancelled total also retains its partial marker; neither condition
alone is a filesystem error. The [size presenter](../../crates/midnight-ui/src/size_presentation.rs)
and [row rendering](../../crates/midnight-ui/ui/file-entry.slint) implement this
follow-up in current source. Earlier recorded size-presentation tests passed the
selected size/model library groups, `size_errors` (2), `panel_warning_details` (4)
and `tree_folder_sizes` (7); see [current evidence](../ACTIVE_WORK.md#current-checkpoint).
The latest modal-only gate does not rerun that entire scope. These
software UI tests do not establish full native/layout qualification. A completed
scan with warnings is not a complete count of inaccessible contents.

Native size scans follow directory junction/link targets using native final-path
resolution and stable
directory identities. The active ancestry set stops self/ancestor cycles before
recounting contents; separate noncyclic aliases and separately selected roots keep
independent logical totals. Cycle/file-link omissions appear as skipped counts in
the size tooltip, without an access-error marker. Failed target resolution keeps
partial bytes and the existing detailed warning report. Providers without the
identity service retain ordinary-directory sizing and report unsupported links.
The cycle-detection state grows with active traversal depth and is released as
each branch unwinds. It has no arbitrary directory/depth cutoff; the existing
pending-path memory budget and bounded worker/I/O capacity remain in force.

Each pane admits one executing job and at most eight waiting captured requests.
Cancellation retains capacity and provider ownership until the actual calls
return. Map publications are coalesced at most four times per second and require
acknowledgement. A dedicated acknowledgement mailbox slot avoids losing the final
result when ordinary commands saturate the queue. Selected-folder totals are
computed on the backend and accepted only for the exact current selection/view.

The [implementation plan](../superpowers/plans/2026-11-10-tree-folder-sizes.md)
tracks qualification; production bridge tests are in `tests/tree_folder_sizes.rs`.

Native, archive and tree scans attach immutable report handles to partial totals.
Yellow warning tooltips show a real issue and counts; clicking opens all retained
paths and reasons in the basic viewer. Red listing diagnostics have a separate
detail action from Retry/Collapse. Full-error Copy uses the captured backend
source. See the [diagnostic contract](PANEL_DIAGNOSTICS.md) and
[qualification](../verification/2026-11-10-panel-warning-details.md).

## Viewers from expanded panels

F3 and Alt+F3 can use the regular files already present in the root listing while
folders are expanded. `ViewerLaunch.tree` retains the exact immutable projection
and focused occurrence. The UI only clones shared handles; the viewer worker maps
visible root aliases through `original_root_entry` and builds the captured file
order. It does not reinterpret occurrence IDs or display captions as native paths.
Space therefore visits only files visible in that captured projection, in its
sort order. Subsequent panel navigation cannot change this viewer's source list.

Native files keep the installed provider's content capture. Archive files retain
their mount, source version and member/parent IDs, including the relative-resource
scope used by the HTML viewer. Existing viewer routing, Basic mode, preferences
and disabled-plugin choices are unchanged. Parent rows, folder descendants,
unreadable archive members and pending projections cannot open a file viewer.

Expanded branches currently contain directories only. This integration does not
grant content or mutation authority to arbitrary descendant display paths; future
files inside attached containers need a typed provider capture before admission.

## External editor from expanded panels

F4 captures the focused ordinary native root file while folders are expanded.
Selection does not replace the caret target. Expanded branches remain folder-only;
this action does not add descendant file rows or permit editing a directory, link,
archive member, parent row or filter-only context ancestor.

`TreeEditorRequest` carries the exact acknowledged projection and occurrence.
The model worker resolves that occurrence through its retained provider context;
UI captions, display paths and original root IDs never become native paths. A
bounded captured source retains the registration, request/navigation cancellation
and one permit from the existing four-slot tree action pool. The presenter checks
the original pane, projection, request and returned occurrence before sending the
captured launch. Switching the active pane cannot retarget it; source navigation,
projection replacement or application close cancels pending work. Queue pressure
retains one pending editor intent with its original settings.

Resolution, native regular-file validation, spawning and final provider disposal
run on backend workers, including shutdown before an async delivery task starts.
Existing process/launch limits and literal executable/argv handling remain. Raw
absolute provider paths retain valid internal `..` components; parent navigation
itself is rejected. An external editor receives a live path, so later filesystem
replacement and the editor's own modifications are outside a file-manager
transaction. See the [editor qualification](../verification/2026-11-11-tree-external-editor.md).

## Checksums from expanded panels

Ctrl+F8 captures the acknowledged occurrence selection, including selected native
directories inside expanded branches and ordinary root files. Without marks only
the focused regular native file is used, matching the flat checksum workflow.
The exact originating pane, projection, selection revision and request are checked
before the existing independent checksum window opens. Pending selection settles
before capture; outer queue refusal retains the original intent. Navigation,
reprojection or shutdown cancels pending work and late/duplicate receipts cannot
open another window. Parent/context/link/archive/unsupported rows are excluded.

`TreeChecksumRequest` resolves through retained provider contexts on the model
worker; `StartCapturedChecksums` carries opaque ownership into the ordinary checksum
actor. Recalculation retains those roots across later panel changes. Native reads,
provider revalidation and final disposal run on workers, with ownership retained
through in-flight calls even after cancellation. A nonrecursive first calculation
reports selected directories as skipped; Options can explicitly enable recursion.
Focused sidecars still prefill Verify file without starting it. The existing
algorithm choices, result formatting, native manifest pickers and verified export
contract remain in [CHECKSUMS.md](CHECKSUMS.md).

## Copy from expanded panels

F5 captures the exact acknowledged tree selection on a backend worker before
opening the ordinary Copy dialog. With no marks it uses the focused occurrence;
the parent row and filter-only ancestor context cannot become copy targets.
Selected nested native or archive directories retain their basename and contents.
The capture owns provider registrations, so navigation after the draft opens
cannot substitute another source. It is not an immutable filesystem snapshot:
native identity is pinned by ordinary copy preparation and retained across Retry;
archive member/version checks remain authoritative.

Four live captured batches share an engine-wide permit pool. Opening the draft
does not reserve a job; confirmation uses the existing 32-job FIFO copy admission.
Preparation validates duplicate roots, source ancestry and native destination
ancestry before writes. The ordinary staged copy independently reads back the
destination under the job's frozen verification policy: Full SHA-256 by default,
or explicit quick verification of exact size plus first/middle/last 64 KiB.
Quick mode compares every byte for files up to 192 KiB and can miss corruption
between samples in larger files. Source identity/revision, synchronization,
publication, conflict consent, cancellation and error guards remain mandatory;
see the [transfer contract](TRANSFERS.md).
Supported distinct native regular files retain the optional exact modified-time
policy; unsupported directory/provider metadata cannot silently downgrade it.

Successful root completion carries its original tree session and occurrence.
The presenter sends bounded, idempotent Remove edits through ordered selection
handling, waiting for earlier user edits and retrying after a changed projection.
Skipped, failed or canceled roots retain their marks. A completion cannot alter
another navigation/session or move the caret. Final provider ownership from
discarded drafts, rejected requests and shutdown is released on backend workers.

The [design](../superpowers/specs/2026-11-10-tree-copy-design.md) and
[implementation plan](../superpowers/plans/2026-11-10-tree-copy.md) track this
integration. Engine/Slint bridge tests use actual F5/Return input and verify both
native and archive nested-directory contents plus independent destination reads.
Clipboard and destructive operations have separate authority requirements.

## Move and transfer reconciliation

F6 shares F5's capture/draft protocol with an immutable Move intent. Native root
files and selected native directory occurrences use the ordinary Move worker;
archive and mixed-provider scopes are refused before preparation. Same-volume
native Move uses the retained rename capability without reading or hashing file
contents. Cross-volume Move and copied children in explicit directory merges
independently verify each copied file under the frozen Full SHA-256 or explicit
quick size plus first/middle/last 64 KiB policy before removing that file's source.
The quick-mode limits above apply. Source/parent identity, revisions, conflict
consent and finalization guards remain authoritative on both routes; verification
policy does not authorize replacing a changed object. Empty source directories
are removed postorder; skipped or failed roots keep their marks. A later failure
never implies rollback of earlier completed files.
Provider ownership covers prepared authorities, descendants, finalizers and
cleanup tickets, including a draft abandoned after failed preparation.

Every terminal Copy/Move result marks both adopted native tree sessions dirty,
including failed or canceled transfers. The backend reconciles the hidden root
and previously loaded native branches, including collapsed cached branches,
parent-first through one provider lane. The accepted link-maintenance extension
includes already-loaded provider-resolved native directory links and their loaded
descendants. It does not discover unopened branches or enumerate foreign/archive
mounts. Surviving occurrence IDs, expansion, marks, filters and sort options remain;
deleted occurrences disappear.
Failed parent listings retain known contents, report a diagnostic and suppress
descendant work for that pass. Cycle/retarget validation and logical action paths
remain intact; read-only maintenance grants no mutation, ACL or elevation authority.

Maintenance waits for preceding selection acknowledgements and holds the mutation
gate through each list/apply step, releasing it before publication backpressure.
Queue refusal remains pending; a transfer completed during a pass causes another
pass. Correlated terminal receipts settle only their own epoch, while navigation
retires the old scope. The original `TreeProjection.root` is admission authority;
`root_actions` is the current backend-owned root listing for F3/Enter and viewer
normalization. Newly copied root files therefore work without reopening the panel
or treating row IDs as original file IDs.

See the [Move design](../superpowers/specs/2026-11-10-tree-move-design.md) and
[reconciliation design](../superpowers/specs/2026-11-10-tree-transfer-reconciliation.md).
Native manual refresh and filesystem notifications now use this same in-session
maintenance. They retain expanded branches, surviving marks and caret instead of
replacing the tree with a flat panel. Archive refresh keeps its separate semantics.

## Native clipboard from expanded panels

Ctrl+C and Ctrl+X now use `TreeClipboardRequest` with the exact acknowledged
projection, selection allocation/revision and optional focused occurrence.
Pending selection feedback settles before capture. A nonempty selection wins
over the caret; `..` and filter-only context rows cannot become clipboard targets.
`Session::capture_clipboard` reuses the common occurrence capture and accepts
only all-native batches. Archive and mixed-provider batches return Unsupported
without replacing the native clipboard.

`TreeEvent::Clipboard` returns an opaque `Arc<dyn CapturedClipboardFiles>`;
`Command::CopyCapturedFiles` hands it to the existing clipboard worker. The UI
calls only its cached intent accessor. The worker opens the OS clipboard,
resolves the retained native targets, rechecks every provider context and request
cancellation, validates the whole payload, then publishes its original Copy/Move
intent. It retains provider ownership until the native write returns. No display
path, member caption or extraction filename is interpreted as a native target.

Four capture permits are shared with F5/F6, followed by the existing 16-request
clipboard queue. The common capture admits at most 4,096 targets and 4 MiB of
newly owned capture data; native clipboard payload limits also apply. Rejected,
canceled and stale captures are retired on backend workers, including queue
refusal and pending engine-command destruction during shutdown. The presenter
keeps one pending clipboard capture and retries outer command-channel pressure
with the same source. Navigation cancels pending work; it does not erase an
already published OS clipboard entry.

Ctrl+V retains the native panel root as its destination even when a nested row
has focus. Copy uses the ordinary verified queue; Move intent opens the existing
Move confirmation. Native source authority and overlap checks remain in that
worker. Copied files use independent destination readback under the frozen
Full/quick policy above, with per-file source removal for Move only after its
proof and identity guards succeed. Same-volume native Move renames without
content verification. Conflict consent and tree reconciliation remain unchanged.
Cut alone does not modify files. Clipboard paths are live names, not immutable
file identities, and external clipboard changes retain ordinary Paste behavior.

The [implementation plan](../superpowers/plans/2026-11-10-tree-clipboard.md) and
[dated evidence](../verification/2026-11-10-tree-clipboard.md) distinguish this
source implementation and focused backend checks from pending UI/native-platform
qualification. Virtual clipboard export requires a separate data-transfer
protocol; this integration does not enable archive Paste, Cut or mutation.

## Native watches and reconciliation

An exactly acknowledged publication supplies the backend's watch scope: the root
and loaded, expanded native branches visible in the projection. The accepted
link-maintenance extension includes provider-resolved native directory links and
their eligible loaded descendants.
An expanded empty branch retains observation so a new child appears. Collapsed,
filtered-out and foreign-mount/provider branches stay outside native watch scope.
Targets come from retained provider registrations, never from displayed paths.

Each pane retains a bounded nonrecursive native batch for its root and up to 31
branch slots. Known physical browse identities deduplicate notification slots;
logical aliases remain separate display rows, polling targets and action
occurrences. A serial fair poll covers all eligible logical branches, including
aliases and those beyond the native slots. Accepted browse-identity changes rebind
subscriptions. Polling checks identity before and after observation; a mismatch
requests maintenance rather than accepting the observation as unchanged.
The two existing backend workers own subscriptions, metadata probes and cleanup.
Events use a one-second quiet interval and a three-second batching ceiling; slow
probes or output backpressure can add delay. Polling/failure retries use 15 seconds.

The accepted listing signature and full immediate-directory observation remain
separate, so files hidden by the folder-only branch view still invalidate sizes
without causing a perpetual refresh. Unchanged publication commits reuse cached
targets/signatures only while accepted browse identities still match, including
paced admission failures. A successful sweep and its exact acknowledged
publication settle only the captured dirty epoch; a newer event remains pending.
Failed listings retain known rows and report a diagnostic.

Navigation or terminal session failure invalidates that generation immediately;
the backend releases native/provider resources after any outstanding call returns.
Late callbacks, sweep receipts and fatal errors cannot cancel a newer navigation.
Watch diagnostics do not turn into Commit retries. The mutation gate remains held
through each maintenance list/apply step, while explicit expansion can proceed
when maintenance is waiting for a transfer.

See the [watch design](../superpowers/specs/2026-11-10-tree-watch-design.md).
The earlier [verification record](../verification/2026-11-10-tree-watch.md) retains
its dated scope; [ACTIVE_WORK](../ACTIVE_WORK.md#current-checkpoint) records current
qualification. This does not add archive watches, generic watch providers or the
remaining tree operation consumers.

## Delete from expanded native panels

F8/Delete and Shift+F8/Shift+Delete capture the exact acknowledged selection, or
one focused real occurrence if the selection is empty. Parent/context-only rows
are never mutation targets. `TreeDeleteRequest` retains its projection, selection
revision, request ID and Trash/permanent mode. UI queue pressure waits for a safe
capture opportunity; navigation, reprojection, replacement and shutdown retire it.

`CapturedDeleteSources` keeps provider registrations and backend native authority.
The entire original selection must be native before overlap suppression. Selecting
an ordinary directory and its descendants deletes the covering directory once,
based on captured occurrence ancestry rather than displayed path prefixes or inode
identity. Separate hard-link names remain separate entries. Overlapping selected
ancestors and descendants whose captured ancestry crosses a junction or symbolic
link are rejected before preparation; they must be deleted separately. This Delete
overlap guard applies independently of browse-only directory-link expansion. A
covering directory that has changed into a file or link is rejected before
confirmation.

This is a mutable-subtree operation, not a snapshot of every displayed descendant.
Native preparation pins the selected topmost roots. Descendants moved away before
execution are not chased, and destructive traversal never follows a child that
has become a junction/link. Captured ancestry only removes duplicate selection
roots; it is not a retained proof of every intermediate filesystem identity.

Preparation pins every remaining target through the existing platform mutation
service; returned paths, count and immutable capture scope are checked. Display
`source_directory` is only presentation/refresh context and is never authorization.
Flat mutation snapshot and immediate-parent checks are not relaxed. Junctions and
links use the existing link-removal semantics; destructive traversal does not follow
their targets. Unsupported archive/mixed selections never fall back to extracted
files or permanent deletion.

The ordinary Preview/Confirm pipeline admits work only after explicit confirmation,
then uses the same `JobProgressWindow` as Copy/Move. Cancelled confirmation creates
no visible job. Rejected, stale and duplicate captures retain a backend reclamation
owner. Terminal mutations queue loaded native-tree reconciliation, preserving the
expanded view instead of replacing it with a flat listing. Capture and operation
budgets remain the existing bounded backend limits; this is not archive write support.

## Rename from expanded native panels

F2 captures the focused real occurrence independently of marked selection. The exact
acknowledged projection, request and row are checked by the tree worker; a displayed
path, caption or original root EntryId cannot authorize a mutation. Native root files
and expanded native directories use the existing Rename dialog. Parent and context-only
rows are rejected; archive occurrences never acquire native write authority. Expanded
branches remain folder-only: this does not introduce arbitrary descendant file rows.

`CapturedRenameTarget` retains the provider-resolved native path and registration; capture
is not a filesystem snapshot. Native identity is pinned during mutation preparation.
The shared four-slot
capture admission pool also bounds its cleanup ownership. Each capture reserves one
slot from the existing runtime-independent retirement domain before provider capture;
its native target and registration are destroyed before its capture permit is returned.
The final source may safely leave a queued reply or UI pending request on any thread.
The operation worker retains native prepared pins ahead of that source in destruction
order. No extra cleanup thread or unbounded owner queue is introduced.

Preparation uses the existing platform mutation service and checks the returned native
path and immutable capture scope. The normal confirmation, name validation, cancellation,
versioned overwrite consent and recovery behavior applies. No rename runs before
confirmation, and later navigation cannot retarget admitted work. Before admission,
navigation, reprojection and shutdown cancel pending captures; queue pressure retries
the same focused occurrence. Completion reconciles loaded native branches through the
existing tree maintenance route, rather than replacing the panel with a flat listing.

F7 retains pane-location authority: it creates under the current pane directory, not the
focused expanded descendant. Flat immediate-parent checks remain unchanged. See the
[Rename verification](../verification/2026-11-12-native-tree-rename.md) for the exact
qualified cases and remaining limitations.

## Metadata from expanded native panels

Attributes, timestamps and filename case use the existing metadata editor after an
exact acknowledged tree capture. Marked real occurrences win over focus; with no
marks, the focused selectable occurrence is captured. Parent/context-only rows,
archives and mixed native/virtual selections never receive native mutation
authority. No displayed path or extracted temporary file becomes a target.

`CapturedMetadataSources` retains native provider registrations through the same
four-slot capture pool and runtime-independent retirement domain as Rename. The
ordinary operation controller reserves the complete batch from its shared
256-target metadata pin budget before opening native targets, then checks the
captured scope and returned target count again. Native pins close before retained
provider ownership. Preparation, traversal, mutation and final resource cleanup
remain on backend workers.

All selected roots remain in the preview. Nonrecursive edits process selected
descendants before selected ancestors and close each completed root's native
pins before the next root. This prevents a parent case conversion from
invalidating pending child names or being blocked by completed child handles. Recursive edits suppress independently
executing selected descendants already covered by another selected ordinary
directory; that ancestor's ordinary metadata traversal reaches them once.
Coverage comes from exact occurrence ancestry, never path-prefix or inode equality.
An overlap crossing a link/non-directory is rejected, and covering directory
kinds are checked again during preparation. Separate hard-link names remain
separate selected entries. This is live subtree traversal, not a snapshot or a
promise to chase descendants moved elsewhere after capture.

The existing explicit patch preview, exact revision confirmation, conflict tokens,
cancellation and partial-result behavior apply. Outer command refusal and transient
tree-capture admission refusal retain the original intent for a later input pump,
with one-pump deferral to prevent receipt loops. Provider/target resource limits
require explicit review or a smaller selection. Navigation,
reprojection, replacement or shutdown cancels pending captures. After admission,
navigation cannot retarget the pinned job. Terminal metadata jobs reconcile loaded
native branches through tree maintenance and preserve the expanded panel.

See [metadata verification](../verification/2026-11-12-native-tree-metadata.md)
for the actual test scope and remaining platform/desktop limits.

## Native Custom Commands

Expanded native trees use the same configured Program/Terminal commands and
preview workflow as flat panels. Activation captures the exact acknowledged view,
marks and focused occurrence independently. Once binds file scalars to the captured
focus; Per selected item and grouped arguments retain marked display order, with
focused fallback only when no items are marked. A literal Once command can run at
the parent row without turning `..` into an argument.

The worker resolves only retained native provider targets and preserves each source
kind. Context-only rows never supply file arguments. Mixed archive/provider
arguments and revoked captures are refused; display paths and temporary exports never supply argument authority. The
captured native root must match the original active-directory context. Existing
native entry/directory pins, command preflight and process admission are reused.
Whole-capture validation precedes preparation; each process then revalidates only
its immutable source IDs and native pin scope. Provider owners and pins stay alive
through the actual native launch call and retire on the existing worker domain.

Pending marks settle before capture. Queue pressure retains the original command,
focus and locations for retry; navigation, reprojection, settings-preview edits or
shutdown cancel pending captures. Accepted launches survive later panel navigation.
Marks and expanded branches remain unchanged. See
[verification](../verification/2026-11-12-tree-custom-commands.md) for test scope.

## Directory comparison with expanded branches

Ctrl+F10 remains available after native tree adoption. Comparison uses selectable
root-level rows from the exact captured display projection; expanded descendant
folders are not an implicit recursive comparison. Names, size, modified time,
unmatched-folder handling and time tolerance use the same backend comparison as
flat panels. Both roots must be native; any virtual/provider entry is rejected,
including virtual descendants of a native root. Display paths cannot authorize
native access. No files are opened or modified by this metadata comparison.

The result is correlated with both original panel paths, navigation IDs, displayed
projections and settled selection revisions before either preview changes. A flat
panel keeps its existing immutable selection snapshot. A tree panel queues a
`TreeSelectionReplacement` through its normal selection worker and displays the
bounded preview immediately. The replacement contains only numeric occurrence
identities and selection bits, at most 512 KiB; it retains no projection, provider,
path or report owners. The model worker validates every occurrence, allowing
metadata-only updates but rejecting changed row identity, hierarchy or selection
eligibility. UI lookup and mailbox admission do not walk the whole tree.

The dialog waits for selection acknowledgements and reports rejection instead of
claiming that selection succeeded. A failed intent can be retried immediately.
The two workers are independent: this is not a transactional filesystem operation
or an atomic commit across both backend actors.

## File comparison while branches are expanded

Ctrl+Shift+C can compare native root-level files while directory branches remain
expanded. Inline descendant rows are folders only. Focused files and up to two
marked files resolve through their exact original root aliases, not labels or the
old flat selection. The backend selection snapshot caches at most two occurrence
IDs, so choosing a pair does not enumerate the panel on the UI thread.

When the opposite panel has no marks, same-name lookup runs on the comparator
worker against its captured native root listing and a bounded cached visibility
set. Hidden, errored and virtual rows do not supply automatic counterparts.
Marked folders, more than two marks, pending selection changes or otherwise
unresolved inputs open the ordinary File A / File B choice. File comparison keeps
its existing native validation and does not export archive members implicitly.

## Properties and native menus in expanded branches

Alt+Enter and item context menus capture exact native occurrences through the
backend tree controller. Right-clicking a marked item captures the marked set;
right-clicking an unmarked item captures that item without changing the marks.
The synthetic parent and context-only ancestors are not action targets.
Properties, file opening and system menus resolve native paths on admitted backend
workers and retain the source through the native call. UI labels are never paths
with authority. Captured desktop work uses the existing bounded process and
cleanup ownership.

Managed Copy/Cut/Delete/Rename verbs re-enter the existing typed action flows.
Paste into one captured folder resolves that destination on the backend; ordinary
panel paste keeps its panel-location authority. Rename and Delete retain their
confirmation/overwrite/progress behavior. Providers without native authority are
not converted into temporary native paths for these actions.

A managed menu choice requires the original projection and settled selection.
Navigation, changed marks or a new projection require reopening the menu; this
includes conservative refusal after metadata or disclosure publications. The
command cannot silently act on a different row. Unsupported system menus use the
existing fallback menu. Virtual clipboard export remains separate work.

## Remaining integration

TREE-01 remains partially integrated. Native Delete, Rename and metadata editing join the typed capture
consumers described above: viewers, external editor, checksums, Custom Commands,
Copy/Move and native clipboard/drag operations. This is not generic provider write support. Native Properties and context menus
use the typed consumers above; virtual clipboard export remains guarded. Native
directory comparison is described above.
Original flat panels remain available; opening a folder returns to the ordinary
operation path. Pane-directory actions such as terminal, bookmarks, creation and
paste destinations keep their existing location authority. Native immediate-parent
checks and the flat `SelectedFiles` contract are unchanged.

Browsable container discovery/attachment and true archive-inside-archive targets
remain pending. Navigation retires the tree and establishes a new flat selection
scope; it does not restore stale pre-tree marks. Native refresh retains the adopted
session through the maintenance path described above.
No production tree completion
claim follows from widget or presenter tests alone.

The integration tests in `crates/midnight-ui/tests/tree_panel.rs` exercise the
installed Rust/Slint bridge. Presenter tests in
`crates/midnight-ui/src/presenter/tests/tree.rs` cover admission, selection,
navigation, focus and operation fences. Existing disclosure tests cover mouse,
touch, all three layouts, both panes, stale presses and compact geometry.

## Filter and sort evidence

The 9 November 2026 Windows debug logs under `_build/qa/tree-filters/` qualify the
current source routing beyond the earlier
[panel admission report](../verification/2026-11-09-tree-panel-admission.md):

- `backend/green-actor.log`: 73 scoped engine units passed.
- `backend/green-controller.log`: 14 production controller and seven tree-filter
  integrations passed, including native/archive roots, stale selection, queue
  pressure and cancellation on native/archive/Network navigation or shutdown.
- `ui/final-green.log` (22:30): 668 UI units, four Engine/Slint tree-filter
  integrations and one tree-panel bridge test passed. Controls exercise apply,
  clear, sorting and Cancel across layouts, inactive archive panels, stale
  receipts after navigation and branch completion overtaking a request.
- `ui/clippy-green.log` records the successful UI all-target lint rerun with
  `-D warnings`; its engine dependency still emits the separately documented
  13 unused/dead-code warnings. This is not a strict workspace result.

These are backend/software-window checks, not installed native application or
Linux desktop qualification for filter/sort. The earlier
[controller Windows/Linux checks](../verification/2026-11-09-tree-controller-integration.md),
[probe scheduling and Windows debug deployment](../verification/2026-11-09-tree-budget.md)
and [top-level Ctrl+A checks](../verification/2026-11-09-tree-select-all.md)
retain their narrower dated scopes. Full native plugin/UI workflows, release
qualification and the remaining operation consumers above stay open.
