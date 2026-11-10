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

`Rows` reads the immutable projection directly. It does not construct a second
complete UI model. Occurrence/session/revision tokens bind disclosure callbacks;
root and descendant entry IDs never become interchangeable. Details, Brief and
Thumbnails use the existing shared disclosure widget. Loading, unknown, collapsed,
expanded, empty and failure states remain distinct. Structural publications reset
delegates and cancel rectangle gestures. Collapsing a focused descendant moves
the caret to its surviving visible ancestor. Loading updates preserve the panel
viewport.

Details and Brief reserve the same disclosure gutter for every row, including
files and directory links without an expansion button. Thus a Windows junction
such as `Documents and Settings` aligns with ordinary folders; its system link
overlay remains visible. Thumbnail artwork retains its centered layout.

Tree selection uses separate backend snapshots and at most 32 small pending
intents for immediate visible feedback. Ctrl/Shift input, Insert/Space toggles,
bulk selection and rectangles submit occurrence-based requests. Context rows
cannot be marked. Loading publications replay outstanding intents after Commit;
a structural change retains only still-visible exact toggles, because replaying
an old positional range could select rows the user did not see. Rejected
selection releases its preview without reusing the request revision.

Ctrl+A replaces selection with selectable top-level rows (`depth == 0`), including
files and folders. Nested marks are cleared. The shared row policy drives both
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

## Directory sizes

Ctrl+Shift+S uses the exact acknowledged tree selection: selected ordinary
directories, or every selectable ordinary directory in the current projection
when there are no marks, including descendants in expanded branches. This means
all projected rows, not just those inside the scroll viewport. Hidden/collapsed
descendants, context-only filter ancestors and links are excluded. A
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

Running values remain highlighted estimates. Actual read failures retain the
partial-result marker and error indication; the error glyph stays immediately
beside its size rather than at the opposite edge of the Size column. An unfinished
or cancelled scan alone is not a filesystem error. Automatic scans do not follow
directory links/junctions, which display a dash instead of a duplicate target total.

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
ancestry before writes. The ordinary staged copy performs SHA-256 destination
readback and uses the existing conflict, cancellation and error decisions.
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
archive and mixed-provider scopes are refused before preparation. Every file is
independently verified at the destination before its native source is removed.
Empty source directories are removed postorder; skipped or failed roots keep
their marks. A later failure never implies rollback of earlier completed files.
Provider ownership covers prepared authorities, descendants, finalizers and
cleanup tickets, including a draft abandoned after failed preparation.

Every terminal Copy/Move result marks both adopted native tree sessions dirty,
including failed or canceled transfers. The backend reconciles the hidden root
and previously loaded native branches, including collapsed cached branches,
parent-first through one provider lane. It does not discover unopened branches
or enumerate foreign/archive mounts. Surviving occurrence IDs, expansion, marks,
filters and sort options remain; deleted occurrences disappear. Failed listings
retain their last known contents and report a diagnostic.

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
Move confirmation. Native source proof, overlap rejection, independent SHA-256
destination readback and verified per-file source removal happen in that worker.
Cut alone does not modify files. Clipboard paths are live names, not immutable
file identities, and external clipboard changes retain ordinary Paste behavior.

The [implementation plan](../superpowers/plans/2026-11-10-tree-clipboard.md) and
[dated evidence](../verification/2026-11-10-tree-clipboard.md) distinguish this
source implementation and focused backend checks from pending UI/native-platform
qualification. Virtual clipboard export requires a separate data-transfer
protocol; this integration does not enable archive Paste, Cut or mutation.

## Native watches and reconciliation

An exactly acknowledged publication supplies the backend's watch scope: the root
and loaded, expanded native branches visible in the projection. An expanded empty
branch retains observation so a newly created child appears. Collapsed, filtered
out, link and foreign-provider branches do not become native watch targets.
Targets come from retained provider registrations, never from displayed paths.

Each pane uses one nonrecursive native batch for its root and up to 31 stable
branches. A serial fair poll covers all eligible branches, including those beyond
the native slots. The two existing backend watch workers own subscriptions,
metadata probes and cleanup. Events use a one-second quiet interval and a
three-second batching ceiling; slow probes or output backpressure can add delay.
Polling and failure retries use a 15-second interval.

The accepted listing signature and full immediate-directory observation remain
separate, so files hidden by the folder-only branch view still invalidate sizes
without causing a perpetual refresh. Unchanged publication commits reuse cached
targets/signatures, including paced admission failures. A successful sweep and
its exact acknowledged publication settle only the captured dirty epoch; a newer
event remains pending. Failed listings retain known rows and report a diagnostic.

Navigation or terminal session failure invalidates that generation immediately;
the backend releases native/provider resources after any outstanding call returns.
Late callbacks, sweep receipts and fatal errors cannot cancel a newer navigation.
Watch diagnostics do not turn into Commit retries. The mutation gate remains held
through each maintenance list/apply step, while explicit expansion can proceed
when maintenance is waiting for a transfer.

See the [watch design](../superpowers/specs/2026-11-10-tree-watch-design.md).
The [verification record](../verification/2026-11-10-tree-watch.md) gives the actual
tested scope; this does not add archive watches,
generic watch providers, or the remaining tree operation consumers.

## Remaining integration

TREE-01 remains partially integrated. Captured tree targets
are not yet consumed by native mutation, virtual clipboard export, external editor,
comparison, checksums, Custom Commands, context menus or file drag.
Their entry/selection paths explicitly refuse tree scopes through the common
tree action guard. Original flat panels remain available; opening a folder
returns to the ordinary operation path. Pane-directory actions such as terminal,
bookmarks, creation and paste destinations keep their existing location authority.
The native immediate-parent checks and `SelectedFiles` contract are unchanged.

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
