# Inline tree directory sizes implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development or
> superpowers:executing-plans; qualify each task before reporting completion.

**Goal:** Make the existing directory-size commands work on native and archive
inline-tree selections, including Space on a descendant, without interrupting
navigation, selection or explicit sorting.

**Architecture:** Capture typed occurrences against the acknowledged backend tree
and selection. Existing native and archive scan algorithms run on bounded backend
workers. Publish coalesced immutable size snapshots, separate from the tree
projection, so progress changes values without changing row order.

**Tech stack:** Existing Rust/Tokio services and Slint presenter; no new dependency
or plugin ABI. Windows and Linux use their existing separate native adapters.

**Spec:** [accepted inline branches](2026-11-05-follow-up-stages.md),
[tree identities](../../developer/INLINE_TREES.md),
[production integration](../../developer/TREE_PANEL.md), and the user's existing
selection-only size, Space toggle/advance, four-updates-per-second and no-progress
resorting requirements. This connects an already requested action to the adopted
tree representation; it does not introduce a new product choice.

## Constraints and decisions

- Keep the existing dirty `codex/viewer-plugins` checkout and its other work.
  Build/test output stays beneath `_build/`; no duplicate build target.
- All capture, traversal, summation and large-payload cleanup run on backend
  workers. UI callbacks use immutable indexed data and bounded pending intents.
- Ctrl+Shift+S measures selected ordinary directories. A nonempty files-only
  selection does no scan. User follow-up on 10 November extends the unselected
  scope to every selectable directory in the acknowledged projection, including
  expanded descendants outside the viewport. Hidden/collapsed/context-only rows
  and links remain excluded. This supersedes the original top-level-only policy.
- Space captures the exact focused occurrence before its FIFO selection toggle;
  the UI toggles and advances immediately while that directory is measured
  independently of other marks. Insert does not scan. Earlier unsettled selection
  defers admission of that same occurrence, never substitutes the later caret.
- Captures validate the exact view/selection allocations and retain typed provider
  leases. Never reconstruct a path from a caption or weaken mutation authority.
- Reuse native SSD-aware scheduling and the shared I/O budget. Archive measurement
  reads directory metadata only, with exact mount/ancestry; never extract content.
- Bound captured batches to 4,096 targets and 4 MiB newly owned metadata, pending
  scans to eight per pane, and retained results to the tree row budget. These are
  operation resource limits, not new file-size or OS path-length restrictions.
- At most one scan job executes per pane. Its capacity remains owned until actual
  provider calls return, even after cancellation/navigation. Native batch internals
  retain their existing positively-local-SSD concurrency policy.
- Publish at most one size-map snapshot per 250 ms per pane, with one outstanding
  publication until acknowledgement. Final deferred state must eventually arrive
  without another user action. Known/partial/error/running remain distinct.
- Selection, sibling sort, filter and collapse preserve accepted size work and
  stable occurrence results. Navigation cancels it; refreshed occurrences cannot
  reuse obsolete values. Progress never reprojects or resorts the panel.
- Explicit Size sort uses a frozen lookup of the latest computed sizes; later
  branch publications retain that lookup until another explicit options request.
- Preserve existing flat-panel size behavior and guards for unimplemented tree
  mutations, viewers, clipboard, checksums and other unrelated consumers.

## Shared interfaces

Core adds `TreeCommand::Sizes(TreeSizeCommand)` and
`TreeEvent::Sizes(TreeSizeEvent)` in a dedicated `tree_sizes.rs` module.

`TreeSizeRequest` carries `view: Arc<TreeProjection>`, `request: TreeRequestId`,
`selection_revision: SelectionRevision`, `selection: Arc<TreeSelectionSnapshot>`
and `scope: TreeSizeScope`. Scope is `SelectionOrVisible` or `Single(TreeRowId)`.

`TreeSizeCommand` is `Compute(TreeSizeRequest)`,
`Acknowledge { stamp: TreeViewStamp, version: u64 }`, or
`Cancel { stamp: TreeViewStamp, request: TreeRequestId }`.

`TreeSizeSnapshot` carries stamp/version and bounded immutable value/running maps.
Its accessors are `get(row) -> Option<FolderSize>`, `running(row) -> bool`,
`len()` and `is_empty()`. A snapshot may describe an earlier projection revision
in the same navigation/session; it never grants operation authority.

Events are `Snapshot { sizes: Arc<TreeSizeSnapshot> }`,
`Selected { view, selection_revision, selection, version, summary }` where summary
is `SelectedFolderSizes`, `Finished { stamp, request, cancelled }`, and
`Rejected { stamp, request, error: TreeDiagnostic }`. Selected totals require the
exact current view and selection allocations. Acknowledge stale snapshots too,
without accepting their values into a different session.

## Review focus

1. Selected files must not trigger fallback scanning; Space must not retarget a
   newer caret or scan unrelated selected folders while selection is settling.
2. Overlapping ancestor/descendant archive selections need separate valid ancestry
   and totals, rather than one invalid common-parent batch or shared seen set.
3. Slow/cancelled provider calls retain all permits and leases until they return;
   a subsequent navigation cannot multiply concurrent scan capacity.
4. Sort/filter/collapse, delayed acknowledgements and stale selection receipts
   must preserve current data and eventually publish the final result.
5. Missing metadata, denied directories, source revocation and arithmetic overflow
   remain visibly partial/errors; no fabricated zero or followed link target.

## Task 1: Backend capture, scan and publication

Owned files: core `tree_sizes.rs`, protocol/exports; engine tree actor/controller,
new tree-size modules, shared native/archive scan seams and focused tests.

- [x] Add failing tests for selection/top-level/Single admission and stale views.
- [x] Add bounded typed batch capture and integrate native/archive scanning with
  exact ancestry and retained provider ownership.
- [x] Add coalesced snapshots, acknowledgement, cancellation/terminal receipts and
  exact-selection totals; exercise queue pressure and final delivery.
- [x] Freeze known size sort keys only on explicit options requests; test no
  automatic resort and continued scan ownership after sort/filter/selection.
- [x] Pass scoped backend Windows/Linux checks and independent review.

## Task 2: Presenter commands, progress and status

Owned files: `crates/midnight-ui/src/presenter/` tree-size state, command handling,
row size/status accessors and focused presenter tests.

- [x] Observe the current blocked bulk command and absent Space scan in failing
  tests before replacing the guards.
- [x] Submit exact current view/selection after pending selection settles. Preserve
  a bounded Space intent for its original occurrence; Insert remains toggle-only.
- [x] Adopt newer same-session snapshots, acknowledge all deliveries and retire
  replaced buffers through the existing backend reclamation path.
- [x] Show existing size/progress styles and known selected-folder byte totals;
  clear stale summaries until their exact selection receipt arrives.
- [x] Test cancellation, queue rejection, navigation, stale receipts and all
  selection/sort/filter continuity; obtain independent source review.

## Task 3: Production bridge, documentation and deployment

Coordinator owns `tests/tree_folder_sizes.rs`, shared fixture refinements,
documentation and the shared Cargo build lease.

- [x] Exercise real Engine/Slint bulk and Space commands for native/archive roots
  and descendants, known byte totals, selection/status and unchanged row order.
- [x] Exercise explicit size sorting and preservation through filter/selection;
  retain previous tree search/filter and flat size regressions.
- [x] Qualify focused Windows/Linux tests, strict scoped lint, formatting and the
  100,000-byte handwritten-source limit; record exact limitations.
- [x] Rebuild/register the final debug executable and smoke-test matching runtime.
- [x] Update tree/size contracts, hotkeys and feature/backlog evidence without
  claiming completion of the broad goal or all tree operations.
