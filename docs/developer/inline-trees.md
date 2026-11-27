# Inline provider branches

`midnight-engine::providers::tree` supplies the backend branch model for TREE-01.
The production Engine routes `Command::Tree` and `Event::Tree` through bounded
per-pane workers. Native and archive panels integrate disclosure,
projection/selection adoption and typed directory navigation. Persistent filters,
prefix/quick search and sibling sorting use acknowledged backend reprojection.
Viewers accept immutable root-file aliases from expanded panels and prepare their
visible file order on the viewer worker; folder descendants grant no file authority.
Copy/Move, Delete, Rename, metadata, external editor, checksums, custom commands,
directory sizes, dragging and native Copy/Cut clipboard publication consume exact
captured occurrences; archive/mixed clipboard exports remain unsupported. The
remaining native workflow qualification of TREE-01 is tracked in the
[backlog](../implementation-backlog.md). See the
[panel contract and dated evidence](tree-panel.md).

## Identity and work ownership

`DirectoryTree` retains `MountedLocation` leases on backend workers. Its hidden
root and each visible occurrence have a nonreused `BranchId`, independent of row
position, caption and provider token. Nodes retain exact mount-scoped `NodeRef`
identities. Identical node tokens in different mounts cannot be substituted.
Each session resolves its own mount and lists it only through its branch work, so
it is that mount's only lister and the authority on which provider references are
live (see [budgets](#budgets)).

`request` creates an owned `BranchWork` without performing provider I/O. The
engine must run `BranchWork::run` under its shared blocking-worker admission.
`apply` accepts only the current request for the same live occurrence and checks
the provider chain again. Collapsing, replacing or removing a branch cancels its
pending work. A late result cannot reopen it or overwrite a new result. Dropping
the tree cancels every pending request.

Work and results retain a per-tree permit until they actually finish/drop.
Cancelling a worker does not release its slot while an OS/provider call is still
running. Abandoned work/results cancel their pending loading indicator. The
model does not spawn threads or promise to interrupt a blocked OS call.

## Listing and disclosure

The root listing retains files and folders. Expanded descendants list only
folders and directory links. Native directory links, including junctions and
directory symlinks, can be enumerated by branch work when the provider supplies a
browse-target identity. Ordinary directories retain their existing browse
semantics when that identity capability is unsupported.

Before listing, branch work rejects a physical target identity already recorded
for an ancestor. Mount-scoped logical node references returned by an expansion
or refresh are also checked against the ancestor chain. Distinct logical aliases
elsewhere remain separate occurrences. A second target-identity observation after
listing, or after a probe, rejects an observed retarget. These browse-only
identities grant no operation authority.
Native namespace replacement remains live; the adapter does not promise an
atomic snapshot or handle-relative no-follow branch enumeration.

Windows compatibility junctions (`My Music` in Documents, `Application Data`,
`Documents and Settings`) carry an explicit Everyone deny-list entry on the link
itself; their targets stay readable. `providers::browse_listing` lists such a
row for browsing: when a node known as a directory link fails to list with
`PermissionDenied` before any entry, it lists the OS-resolved target
(`resolve_size_directory`, the final path of an actual directory) and gives each
entry its path below the link, so children, tokens and later actions keep the
link's logical path, exactly as for a readable junction. Root-row presence hints
(`folder_presence`), branch probes, expansions and refreshes (`NativeMount`) and
watch probes of an expanded link (`ScopeTarget::link`) use it. An unresolvable link
keeps its own denial and a target that cannot be listed reports its own error.
Ordinary folders, other errors and other platforms list the path only; recursive
walks (sizes, Find, Compare, Copy/Move) never use it.

The native adapter validates immediate child paths and raw basenames before
binding tokens. It forwards a consumer's callback stop result to the injected
directory service and stops processing further callbacks after a stop or entry
failure. This does not promise to interrupt an already blocked OS/provider call.

Unknown, loading, collapsed-with-children, expanded, confirmed empty and failed
are distinct states. Probes do not expand a branch. A probe of a descendant asks
only whether it contains a folder or directory link: enumeration stops at the
first one, and the probe issues no child reference or row and has no row limit,
so a folder with more children than a listing may hold still shows as
collapsed-with-children. Only the entries inspected before that first folder are
validated; a child that repeats an ancestor reference fails a later expansion or
refresh, not the probe. An expanded branch requires Refresh rather than Probe,
preventing an empty hint contradicting visible rows. Failure retains the previous
successful children, with an explicit error. The hidden root has a state accessor
too; failed root loading is not an empty folder.

Refresh reconciles children by mount-scoped node identity, preserves surviving
occurrence IDs, removes obsolete descendants and rejects ancestor cycles. The
references of removed rows are released at a later reclamation; surviving rows
keep their tokens, occurrence IDs, expansion and marks, so an unchanged
re-listing stays unchanged. It does not sort the entire flattened tree. The
production projection sorts each sibling group independently. Persistent panel
filters and optional prefix filtering use the same unselectable ancestor-context
model.

Prefix search examines only loaded visible model rows and performs no enumeration
or archive extraction. It shares the flat panel's Unicode folding and original
grapheme highlighting. A nonempty query matches valid Unicode native names or
UTF-8 archive raw names; a lossy or alternate caption cannot invent a match.
Other raw names remain addressable in the ordinary view. Persistent filters
intersect with the prefix before ancestor context is retained. Context-only rows
never match or remain selected. Jump mode preserves marks; filter mode publishes
the visible rows and rebased selection together. Empty search restores the
persistent-filter projection.

`TreeProjectionOptions::search` is transient. Its query has a 4,096-byte admission
budget; invalid requests leave the accepted projection and selection intact.
`TreeDisplayIndex::search_rows` contains ordered projection positions, with its
allocation included in the projection budget. Matching and index construction
stay on the backend. The presenter retains the latest desired query behind one
in-flight reprojection, retries typed transient failures on a later pump, and
keeps sorting, saved filters and quick-filter preview/Cancel consistent with it.
Insert retains an exact occurrence until query clearing and selection settle.

Extension sorting uses the same borrowed filename accessor for locale-key
preparation and comparison: actual native basenames or raw UTF-8 archive member
basenames. It preserves directories-first, locale extension/name order and stable
ties; missing or undecodable archive extensions remain last in either direction.
Archive entries retain their opaque member identities and empty native path
fields; captions never become fabricated native member paths.

An explicitly captured container mount can be attached to a file occurrence.
Its source file and target mount stay separate; navigation returns the target
context, not a fabricated native path. A revoked container can be replaced while
its enclosing source and ancestors remain valid. An extension or caption never
mounts an archive implicitly. Production discovery of nested browsable container
files alongside folders remains part of the panel/provider integration.

`resolve_entry_target` resolves the source entry for a currently visible branch.
It rejects the hidden root, collapsed descendants and foreign branch IDs. For an
attached archive this means the container file itself; `navigation_target`
instead returns its entered root. Losing the container does not make a still
valid source file unusable. Descendant targets retain and validate all enclosing
registrations, including when their archive handles are cloned independently.
The final operation must still capture/validate panel selection and native
object identity; obtaining a target never grants mutation permission.

## Budgets

Default per-tree bounds are 10,000 retained occurrences including the hidden root,
16 MiB charged payload, depth 64, eight work/result permits and 64 explicitly
attached container targets. Callers can lower these bounds. Overflow rejects the
attempt atomically; it does not silently truncate children or label them empty.

Depth is a row's tree depth, as `TreeProjection::MAX_DEPTH` counts it: the shown
folder's own entries are depth 0 and the folders inside them depth 1, so a
folder shows 64 levels of subfolders. A folder at depth 64 is still listed: an
empty one expands, and a listing that would add depth 65 fails on that folder
with "Subfolders here are more than 64 levels deep and are not shown." Depth
costs nothing else: every occurrence is charged
once for its own entry, never for its path, and a listing returns its permit
when its result is applied, so a 64-level chain holds one occurrence per level
and no permit between levels.

These are admission budgets, not an exact process RSS guarantee. Old presentation
snapshots have separate lifecycle requirements; provider reference tables are
reclaimed down to the model as described next.

After applying, restoring or discarding each branch completion, the session
checks the root mount's references that no occurrence holds
(`DirectoryTree::reclaim_after_completion`) and releases them once that is due.
It counts released references from the larger of the lease's issued references
and the provider's records, so native records that failed or presence-only
listings bound but never issued count too. Release is due once they reach the
larger of the live occurrence count and 4,096, once they exceed 4,096 after a
failed or unpublished listing, or once 1,024 completions have passed. The tree
skips the check while any of its branch work or results is outstanding, because
a listing that returned but is not applied yet has issued references that no
occurrence holds yet; a pane runs one branch job at a time, so in production
none is outstanding then. The lease's reclaim gate covers only provider calls in
flight and skips the pass if one ever is. Local listing checkpoints are committed
or restored by then. An unpublished recursive batch defers reclamation until it
is published or rolled back, because a rollback restores occurrences from its
checkpoint. Captured operation, clipboard, drop and watch targets check
registration only, so a queued copy or paste whose row disappeared keeps its
target and an old watch scope keeps probing until it is replaced. Under churn,
such as temporary files appearing and disappearing in a watched folder, the
provider tables therefore hold the live references plus at most one threshold
and the listings since the last settled completion (one listing, or one
unpublished recursive batch of at most 128 listings), regardless of uptime.

A pass runs on the pane's model worker before that completion's publication. The
ignored `tree_reference_churn_harness` and `reclamation_cost_harness` tests measure
its cost: in an optimized test build (release opt-level without fat LTO), applying a
root completion that triggered a pass took 1.4-2.0 ms longer with 1,103 live rows
(6.6-8.9 ms without a pass) and 8.6-10.6 ms longer with 6,603 live rows (54-65 ms
without). One isolated pass over both tables took 6-10 ms from 25,000 records to
10,001 and 27-63 ms from 99,000 records, near the caps.

Without reclamation a long-lived churning mount would reach the 100,000-reference
caps. A listing that then meets a new name fails as resource-limited. The root is
refreshed first and a failed branch drops its subtree from the sweep, so with a
churning root every later sweep would stop at the root, leaving the whole tree
stale behind an explicit refresh error until navigation.

Recursive expansion batches at most 128 returned results or 50 ms per
publication (`tree_recursive.rs`). Retained model bounds do not establish fast
large-tree expansion: the synthetic-provider fixtures measure no filesystem I/O or
UI paint. See
[recursive expansion](tree-panel.md#retry-diagnostics-and-recursive-expansion).

A budget that stops the walk does not undo its batch: the listings it made
within the budgets are published, and the folder it stopped at shows its
failure. Only a display projection that cannot hold the batch rolls it back to
the last publication. A folder refused by
the depth budget stays local to its row, like a denied folder, and the walk
goes on with the others. The walk's final `TreeEvent::Rejected` for its request
names the row where it stopped and says why: the first folder the depth budget
left out and how many others, or the folder where another budget, a change or
an admission failure stopped it.

Folder-only listing does not spend its retained-row quota on regular files, but
still validates every observed token before filtering. A separate 32 MiB token
ledger catches duplicate/self-referencing tokens and changed issued kinds; an
additional one-million-callback ceiling bounds inspected work. The token budget
normally triggers earlier. The existing general listing limit remains 50,000
entries/32 MiB payload; folder requests may lower their retained-row limit.
Failed/cancelled attempts issue no partial references. Presence probes use the
same ledger for the entries they inspect, retain no row and issue no reference.
Provider-side node limits still apply; their tables are reclaimed together with
the issued references (above).

Owned String capacities count toward payload admission, including localized date
and time. Diagnostics use a fresh bounded UTF-8 allocation, at most 4096 bytes
and the remaining tree payload budget; truncating a large allocation's length
alone would not satisfy that requirement.

## Required production integration

### Selection and publication prerequisite

The backend `tree_actor::Session` owns an occurrence-based selection through
`tree_selection::Selection`. `publication()` returns the exact projection and its
selection together. A successful reprojection rebases selected occurrence IDs
before either object becomes visible; collapsed, filtered-out and context-only
rows drop out of the selection. A failed or cancelled reprojection leaves the
previous published pair intact. Root selection migrates using the original
native index positions or archive member IDs, never a display name or virtual
path. Exact root index identity is required for native migration.

Selection actions require the acknowledged current projection Arc and a newer
selection revision. An authoritative seed must use that same view and precede
its action; stale seeds cannot overwrite newer accepted selection. Validation
and snapshot construction finish before state is replaced. Repeating a root
seed at the same revision is idempotent; conflicting selected bits are rejected,
including after publication of the initial empty revision. Context rows cannot
be selected by clicks, ranges or rectangles. Ctrl+I inverts non-directory entries
and retains both folder and directory-link selection.

The production controller owns these sessions on one persistent model worker per
pane, which runs on one of the engine's resident worker threads outside Tokio's
blocking pool; branch listings still run as separate blocking jobs. Completed-root notifications, bounded command
admission, loading and collapse publications, selection and typed navigation use
the real Engine command/event boundary. Selection, viewport, reprojection and
operation captures require the exact `Commit` of the current projection. Branch
and navigation gestures may still name the previously acknowledged view while a
newer publication travels to the UI; the actor revalidates that the occurrence is
still visible and unchanged. A shared listing permit and the persistent branch
budget remain held through actual provider return and completion retirement,
including after cancellation or pane replacement.

Native, archive and Network navigation cancel the previous tree generation. A
pending or current Network generation cannot admit an earlier native/archive
root. Accepted archive navigation targets retain their registration independently
of later pane replacement. See the [controller contract](../../crates/midnight-engine/tree-controller.md)
for queue, payload, shutdown and retirement limits. Presenter browsing is integrated.
Dated evidence: filter/sort routing in
[tree-panel.md](tree-panel.md#filter-and-sort-evidence) and prefix integration in the
[prefix search record](../verification/2026-11-10-tree-prefix-search.md).

### Completed roots and typed action admission

Native and archive listing completion retain the exact entries and directory
index on the backend before publishing them. `tree_roots::ReadyRoot` retains
either the native source path or the existing archive location/mount. Admission
requires the same pane, navigation and both original Arc allocations. A new
pending navigation rejects an old root request. Accepted captures keep their
source alive across later navigation; equal displayed contents are not authority.
Opening a root adapter reuses the captured listing without enumerating or
extracting it again, while archive source validation remains mandatory.
Replacing or draining cached roots detaches ownership under the snapshots lock
and releases it afterward on a blocking worker. Archive completion holds its
existing worker permit through that cleanup. Shutdown cancels publishers before
draining the two pane caches; late cancelled completions cannot repopulate them.

`Session::capture_selection` requires the exact acknowledged projection and
current selection Arc/revision. An empty selection may use a fallback occurrence;
a nonempty selection always wins. It resolves the entire batch on a backend
worker, rejects hidden/context-only rows and releases all provisional targets on
failure or cancellation. It revalidates retained provider contexts after the last
resolution so a later resolution cannot silently revoke an earlier target.

Each result retains its view, selection, ordered occurrence IDs, typed targets
and registration leases. Admission permits as many targets as the projection holds
(10,000 rows) and 16 MiB of newly owned target/context metadata, and each consumer
keeps its own bound (Copy and Move take every captured item); shared view/provider allocations retain
their separate existing budgets. This does not impose a new OS path limit.
Overlapping ancestors/descendants and aliases remain explicit occurrences;
operation-specific preflight must resolve their semantics through provider
identity and ancestry. These captures are not native mutation proofs. Existing
immediate-parent, identity and source-revision checks remain required.

### Remaining connections

Keep the existing complete root listing visible if tree admission fails; tree
limits must not become new file-panel navigation or OS path-length limits.
Production selection uses occurrence identities and exact acknowledged projection
and selection snapshots. [Directory sizes](tree-panel.md#directory-sizes) and the
operation consumers listed above consume captured native/archive targets.
Do not weaken native mutation's immediate-parent and captured-snapshot checks to accept arbitrary
descendants. The adapters provide complete typed native/archive ancestry;
the production actor must consume it instead of reconstructing a display path.

Preserve provider locale enrichment, native-plugin content capture, raw path
identity, queued-operation ownership and virtualization. Persistent filtering,
prefix search, sibling sorting, rectangle selection, expanded native branch
watches ([watch scope](tree-panel.md#native-watches-and-reconciliation)) and file
dragging are integrated.
The transitional registry native adapter requires explicit capture injection and has no implicit native read
fallback. Production must supply the mandatory native plugin's capture seam.

The model tests and native/archive adapter tests are backend evidence. The
[panel report](../verification/2026-11-09-tree-panel-admission.md) and
[later filter/sort logs](tree-panel.md#filter-and-sort-evidence) separately record
production presenter/software-window checks; neither establishes full native
plugin interaction, descendant operation support or completed VFS migration.
