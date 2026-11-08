# Inline provider branches

`midnight-engine::providers::tree` supplies the backend branch model for TREE-01.
It is not yet connected to the production panel actor or Slint rows. Existing
panels remain flat until projection, selection and action routing are integrated.

## Identity and work ownership

`DirectoryTree` retains `MountedLocation` leases on backend workers. Its hidden
root and each visible occurrence have a nonreused `BranchId`, independent of row
position, caption and provider token. Nodes retain exact mount-scoped `NodeRef`
identities. Identical node tokens in different mounts cannot be substituted.

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
folders and directory links. Known links are navigation targets but are never
enumerated by a branch request. Native namespace replacement can still change a
directory into a link during a live browse; the current read-only adapter does
not promise handle-relative no-follow branch enumeration.

Unknown, loading, collapsed-with-children, expanded, confirmed empty and failed
are distinct states. Probes do not expand a branch. An expanded branch requires
Refresh rather than Probe, preventing an empty hint contradicting visible rows.
Failure retains the previous successful children, with an explicit error. The
hidden root has a state accessor too; failed root loading is not an empty folder.

Refresh reconciles children by mount-scoped node identity, preserves surviving
occurrence IDs, removes obsolete descendants and rejects ancestor cycles. It
does not sort the entire flattened tree: sibling locale sorting and filtering
with ancestor context belong to the pending production projection.

An explicitly captured container mount can be attached to a file occurrence.
Its source file and target mount stay separate; navigation returns the target
context, not a fabricated native path. A revoked container can be replaced while
its enclosing source and ancestors remain valid. An extension or caption never
mounts an archive implicitly. Production discovery of nested browsable container
files alongside folders remains part of the panel/provider integration.

## Budgets

Default per-tree bounds are 10,000 retained occurrences including the hidden root,
16 MiB charged payload, depth 64, eight work/result permits and 64 explicitly
attached container targets. Callers can lower these bounds. Overflow rejects the
attempt atomically; it does not silently truncate children or label them empty.
These are admission budgets, not an exact process RSS guarantee. Old presentation
snapshots and provider caches have separate lifecycle/shared-budget requirements.

Folder-only listing does not spend its retained-row quota on regular files, but
still validates every observed token before filtering. A separate 32 MiB token
ledger catches duplicate/self-referencing tokens and changed issued kinds; an
additional one-million-callback ceiling bounds inspected work. The token budget
normally triggers earlier. The existing general listing limit remains 50,000
entries/32 MiB payload; folder requests may lower their retained-row limit.
Failed/cancelled attempts issue no partial references. Provider-side node/cache
limits still apply independently.

Owned String capacities count toward payload admission, including localized date
and time. Diagnostics use a fresh bounded UTF-8 allocation, at most 4096 bytes
and the remaining tree payload budget; truncating a large allocation's length
alone would not satisfy that requirement.

## Required production integration

Keep the existing complete root listing visible if tree admission fails; tree
limits must not become new file-panel navigation or OS path-length limits.
Backend projection must map branch identities to exact typed action targets and
selection scopes before any row becomes selectable. Do not weaken native
mutation's immediate-parent and captured-snapshot checks to accept arbitrary
descendants. Archive navigation needs complete ancestry for each inline row.

Preserve provider locale enrichment, native-plugin content capture, raw path
identity, queued-operation ownership, watches, filtering, prefix search,
rectangle/file dragging and virtualization. The transitional registry native
adapter's old content capture is not an authorized substitute for the mandatory
native plugin's capture seam.

The model tests and native/archive adapter tests are backend evidence, not proof
of production tree UI, real plugin interaction or completed VFS migration.
