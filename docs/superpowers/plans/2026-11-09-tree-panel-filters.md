# Persistent Inline-Tree Filters Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable Ctrl+F12 persistent filters, clearing and explicit sibling sorting on already adopted native/archive inline trees without losing expansion, selection coherence or provider authority.

**Architecture:** Send one explicit, revision-bound reprojection command to the existing tree worker. Build the replacement projection and rebased selection transactionally, then acknowledge the applied options together with that publication. The presenter commits preferences and the applied-filter indicator only after accepting that receipt; the existing exact `Commit` still fences subsequent tree input.

**Tech Stack:** Existing Rust workspace, Tokio bounded workers, immutable `Arc` snapshots, current filter compiler and Slint presenter/widgets. No new dependencies or plugin ABI changes.

**Spec:** [Inline trees](../../developer/INLINE_TREES.md), [current panel integration](../../developer/TREE_PANEL.md), [metadata filters](../../developer/METADATA_FILTERS.md), TREE-01/FILTER-01 in [the backlog](../../IMPLEMENTATION_BACKLOG.md), and the user's accepted persistent-filter/sort follow-up. This is a continuation of the existing production integration, not completion of TREE-01.

## Global Constraints

- Follow `AGENTS.md`, `CODE_STYLE.md` and `SECURITY.md`; all filesystem work and large-data reclamation stay on backend workers.
- Preserve raw native paths, opaque archive member identities, source registrations and exact root entry/index allocations. Display paths never authorize operations.
- Preserve the existing 4,096 UTF-8 byte filter-expression bound, filter compiler budgets, eight-command pane mailbox, 10,000-row/16 MiB tree projection limits and bounded cancellation ownership.
- Keep Windows/Linux implementations separate. Build/test output stays under `_build/`; toolchain caches use `.tools/`.
- Ctrl+A selects selectable top-level rows (`depth == 0`) and clears nested marks. Context-only ancestors remain unselectable by every selection path.
- No prefix search, initial tree admission from a filtered flat listing, watches, container attachment, descendant file operations or dependency changes in this slice.
- Operate on the shared current tree only after the root integrator releases the source freeze. Do not stage, overwrite or revert other contributors' changes.
- Implementation is already authorized. This plan adds no user approval or two-phase accept protocol. Coordinate commits with the root integrator; do not commit shared unrelated edits.

## Review Focus

- A branch completion overtakes a pending filter: reject the stale request without applying it to a replacement projection or changing the last valid filter. Task 1.
- OK succeeds at the outer queue but the tree mailbox rejects it: show failure, retain the draft for retry and keep the previous badge/preferences/selection. Task 2.
- Cancel, navigation or a late validation response arrives around OK: only a still-current draft may enqueue; after enqueue the intent is committed, and old-generation receipts cannot affect the new pane. Tasks 2 and 3.
- An archive child has a matching caption under a nonmatching ancestor: keep its real ancestry, context-only parent and opaque identity, without extracting or resolving native substitutes. Tasks 1 and 3.
- Sorting/clearing after a filter hides selected descendants: preserve surviving occurrence identities and sibling order, never revive hidden marks, and retain the new Ctrl+A semantics. Tasks 1 and 3.

---

## Current implementation and scope boundary

`tree_projection::ProjectionOptions` already contains sort/filter/locale. `Session::reproject` builds a view, rebases selection and only then replaces its published pair. `sorted_rows` retains matching rows plus context ancestors and sorts each sibling group. Actor tests already cover these model behaviors.

Production `TreeCommand` currently exposes Enable/Commit/Branch/Viewport/Select/Navigate only. `presenter/filter.rs::open_panel_filter` refuses adopted trees; `search.rs::change_search_query` also refuses them. `Presenter::sort` navigates/re-lists, retiring a tree. `filter.rs::flush_filters` currently persists a validated flat filter immediately. These are the specific connections to change.

The original complete `TreeRootSnapshot` remains unchanged by filtering and sorting. Reprojection considers retained, already discovered tree rows only; it does not recursively discover matches in collapsed/unloaded branches. Clearing restores retained rows and expansion, but not marks removed by filtering. Existing initial-adoption restrictions remain in this slice.

## Frozen backend/UI interface

The backend owner adds these public core types in `crates/midnight-core/src/tree_protocol.rs` (already re-exported by `core/src/lib.rs`):

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeProjectionOptions {
    pub sort: SortSpec,
    pub filter: PanelFilter,
}

#[derive(Clone, Debug)]
pub struct TreeReprojectRequest {
    pub view: Arc<TreeProjection>,
    pub request: TreeRequestId,
    pub selection_revision: SelectionRevision,
    pub selection: Arc<TreeSelectionSnapshot>,
    pub options: TreeProjectionOptions,
}
```

Add `TreeCommand::Reproject(TreeReprojectRequest)` and these event variants:

```rust
TreeEvent::Reprojected {
    previous: TreeViewStamp,
    request: TreeRequestId,
    options: TreeProjectionOptions,
    publication: TreePublication,
}
TreeEvent::ReprojectFailed {
    stamp: TreeViewStamp,
    request: TreeRequestId,
    error: TreeDiagnostic,
}
```

`Reprojected` is the sole success receipt; do not additionally emit a duplicate `Published` for that same revision. Its publication must pass the same mailbox `publish` check and event delivery path as ordinary `Published`. The UI subsequently sends ordinary `TreeCommand::Commit` for the accepted new view. The previous view stamp correlates the request; exact allocation checks remain authoritative at admission.

The request's exact selection Arc and revision must match the session. A request ID is nonzero and newer than the session's prior admitted explicit request; use the existing per-pane presenter request allocator and session high-water policy rather than inventing a second namespace. Replays cannot apply twice. Queue rejection reports `ReprojectFailed`, not the row-oriented `Rejected` with a fabricated row ID.

The public options deliberately contain no locale service, provider handle, native path or filter-compiled program. The engine retains its trusted locale and compiles the supplied bounded filter on its existing model worker.

## Ownership and parallel execution

1. **Backend owner:** core protocol, `tree_actor.rs`, `tree_projection.rs` only if needed, `tree_controller.rs`, `tree_controller/{mailbox,worker}.rs`, focused engine tests and `TREE_CONTROLLER.md`.
2. **UI owner:** `presenter.rs`, `presenter/{tree,filter,search,preferences}.rs`, new `presenter/tree_options.rs`, tests and minimal existing widget/status wiring if required. The UI owner consumes the frozen interface above and does not edit core/engine files.
3. **Root integrator:** combined application/installed-plugin qualification, final status documentation and scoped integration/commit. Backend and UI owners may implement in parallel after agreeing the interface; shared builds remain serialized by the integrator.

Avoid edits to `core/tree_view.rs`, `engine/tree_selection.rs` and `ui/presenter/tree_selection.rs`; the newly qualified Ctrl+A policy already exists. Add focused tests in separate files to reduce overlap with the ongoing tree tests.

### Task 1: Add transactional production reprojection

**Files:**
- Modify: `crates/midnight-core/src/tree_protocol.rs`.
- Modify: `crates/midnight-engine/src/tree_actor.rs`, `tree_controller.rs`, `tree_controller/mailbox.rs`, `tree_controller/worker.rs`.
- Create: `crates/midnight-engine/src/tree_reproject_tests.rs` and `crates/midnight-engine/tests/tree_filters.rs` (small shared test helpers may be extracted if needed).
- Reference existing tests: `tree_actor_tests.rs::filtering_retains_unselectable_ancestor_context_and_rebases_selection_by_occurrence`, `tree_actor_selection_tests.rs::tree_filter_context_is_never_selected_even_by_ranges_or_bulk_commands`, and `tests/tree_controller.rs` for real Engine construction.

**Interfaces:** Consumes the core request above. Produces `Session::reproject_request(&mut self, request: &TreeReprojectRequest, cancel: &Cancellation) -> Result<TreePublication, ProviderError>` and the two routed event variants.

- [ ] Add `tree_filter_reprojects_exact_view_and_selection_atomically`: seed/expand a real session; select a root plus descendant; apply a filter matching the descendant; assert unchanged root entry/index Arcs, same occurrence IDs/expansion, unselectable parent context, rebased marks, and one new projection revision.
- [ ] Add `tree_filter_failure_keeps_the_last_valid_pair`: invalid syntax, oversized/NUL expression, canceled request, foreign equal-content view, wrong selection Arc/revision and replay each fail with unchanged accepted options and exact prior publication/selection Arcs. Add a direct command case bypassing UI validation.
- [ ] Add `tree_filter_clear_and_sort_preserve_siblings_without_restoring_hidden_marks`: clear by changing only `enabled`, retain expression, exercise ascending/descending sibling groups and assert Ctrl+A selects only selectable depth-zero rows after each publication.
- [ ] Run `cargo test -p midnight-engine --lib tree_filter --locked --offline`; record the causal RED failures under `_build/qa/tree-filters/`.
- [ ] Implement `reproject_request`: validate bounded input, exact acknowledged view, exact selection Arc/revision and request freshness; construct candidate options/view/selection using the existing trusted locale; check cancellation before replacing session options/publication. Failure must leave the previous published pair usable. Do not re-enumerate, resolve new provider targets or change root authority.
- [ ] Route the command through `command_pane`, `command_navigation`, bounded preflight, `known_view`, worker dispatch and dedicated failure mapping. Check expression allocation capacity as well as length before retaining it in the mailbox; normalize excessive spare capacity on the awaited backend cleanup/preflight path rather than adding a smaller expression limit. Include the selection payload in admission bounds; no giant invalid selection may occupy the queue.
- [ ] Update `Worker::emit` to register a `Reprojected` publication exactly like `Published`. Ordinary branch completions use the new session options thereafter. Preserve current cancellation, queue and permit ownership; no new worker/thread or provider call is needed.
- [ ] Add real Engine tests in `tests/tree_filters.rs`: successful native and archive flows; success requires exact `Commit`; stale branch-completion interleaving; wrong root/session; foreign selection; full mailbox; canceled navigation to native/archive/Network; shutdown and late delivery. Instrument fixture provider calls and assert filter/clear/sort adds no enumeration/read/extraction calls.
- [ ] Run `cargo test -p midnight-engine --lib tree_filter --locked --offline` and `cargo test -p midnight-engine --test tree_filters --locked --offline`; require zero failures. Hand off the interface and evidence to the UI owner/root before integration.

### Task 2: Connect the existing filter and sort controls to acknowledged state

**Files:**
- Create: `crates/midnight-ui/src/presenter/tree_options.rs` and `tree_options_tests.rs`.
- Modify: `crates/midnight-ui/src/presenter.rs`, `presenter/tree.rs`, `presenter/filter.rs`, `presenter/search.rs`, `presenter/preferences.rs` as needed for existing preference synchronization.
- Extend: `presenter/filter_tests.rs`; use existing `presenter/tests/tree.rs` fixtures without changing their policy.

**Interfaces:** Consumes `TreeCommand::Reproject` and the two receipts. Introduce a UI-only `tree_options::State` with at most one committed pending request per pane; it retains the captured basis/options/origin and bounded draft needed for retry. Add:

```rust
Presenter::request_tree_options(
    &mut self, pane: PaneId, options: TreeProjectionOptions,
    origin: tree_options::Origin, send: &dyn Fn(Command) -> bool,
) -> bool;
// Origin = FilterApply | FilterClear | Sort | Preferences
```

The existing `apply_tree_event` delegates the two receipts into this module. Change `publish_tree` to return whether it actually adopted the publication, retaining all current identity/revision checks. The options receipt may update applied state only when that adoption succeeds.

- [ ] Add `tree_filter_ok_waits_for_application_receipt`: validation success alone does not update preferences, save, show an applied badge or change rows. After successful command enqueue close the dialog, show a pending status and retain its draft. Only a matching adopted `Reprojected` receipt updates the actual filter/preferences and triggers normal saving.
- [ ] Add `tree_filter_cancel_and_enqueue_failure_keep_the_draft_uncommitted`: Cancel/X during editing or validation invalidates the attempt; stale validation cannot enqueue. If the outer command queue refuses the reprojection, keep the dialog/draft open with a retryable error. After successful enqueue OK is committed; there is no second accept/approval stage or cancelable editing session.
- [ ] Add `tree_filter_worker_rejection_preserves_previous_filter`: outer enqueue succeeds but worker rejects stale/busy/invalid input; release pending state, retain previous applied filter/marks/indicator, report the contextual error and preserve the draft for the next dialog opening. Match failure against the pending request's captured stamp, even if an ordinary branch publication has since advanced the current revision in that same session; otherwise the input fence could remain stuck forever. Do not auto-apply a stale request to a new view.
- [ ] Add `tree_filter_receipts_cannot_change_replaced_panes_or_newer_intents`: old request/session/root, duplicate success, and navigation after enqueue cannot save filters or resurrect dialogs. A receipt must match the pending request, expected options and captured root before adoption/persistence.
- [ ] Run `cargo test -p midnight-ui --lib tree_filter --locked --offline`; record the causal RED failures.
- [ ] Remove the adopted-tree guard in `open_panel_filter`; keep its backend validation and Unicode diagnostics. Capture the root/session identity with the draft, then acquire the latest acknowledged view and settled selection at enqueue time; background loading updates during editing need not invalidate the same-root draft. In `flush_filters`, send the typed request only for that still-current adopted tree; retain existing flat behavior otherwise. A draft opened before tree admission/navigation may not silently retarget.
- [ ] Implement the bounded pending state. Send only after current Commit and selection acknowledgement have settled (`revision == applied`, no pending preview/replay). If not ready, leave the draft/action retryable with an explicit busy message. While a request is in flight, fence further tree selection/disclosure/action captures; continue scrolling and allow navigation/shutdown to cancel the generation. Refuse a second options submission with a visible busy message rather than enqueueing unbounded or overlapping intents.
- [ ] Route tree Clear through the same request with only `filter.enabled = false`; preserve the saved expression. Route `Presenter::sort` before its current eager `p.sort` mutation and navigation branches; preserve the currently applied filter, and assign `p.sort` only on the matching success receipt. Queue/failure paths leave previous sort intact.
- [ ] On success adopt publication, set actual search/filter/sort presentation state, clear the pending status and use existing preference saving. On save failure retain the applied filter and report the existing unsaved-preferences error; do not pretend filesystem persistence succeeded.
- [ ] Prevent `search_filter_changed`/`flush_search` and preference loading/rollback from sending flat Search/Selection requests for an adopted tree. Reconcile persistent-filter changes through this same route, keep the applied tree indicator bound to acknowledged options, and preserve unrelated preference edits when a tree request fails. Keep prefix-search refusal unchanged.
- [ ] Clear and retire pending payloads through existing background reclamation on navigation/shutdown. After ordinary loading/branch publications, retain the explicit pending request until its correlated result; never interpret an unrelated `Published` as filter success.
- [ ] Run `cargo test -p midnight-ui --lib tree_filter --locked --offline` plus existing filter/tree presenter tests. Require exact root/provider authority, no flat Search/Resort/Navigate from tree options, no partial view/selection adoption, and no stale preview marks.

### Task 3: Qualify real callbacks and combined Engine behavior

**Files:**
- Create: `crates/midnight-ui/tests/tree_filters.rs`.
- Extend only as needed: existing `tests/support/ui.rs`, `tests/tree_panel.rs`, `tests/commander_dialogs.rs`, `tests/panel_toolbar.rs` and `tests/shared_toolbar.rs` helpers.
- Update: `docs/developer/TREE_PANEL.md`, `crates/midnight-engine/TREE_CONTROLLER.md`, `docs/FEATURE_STATUS.md`, `docs/IMPLEMENTATION_BACKLOG.md` and a focused dated verification report.

**Interfaces:** Uses the frozen core protocol, real Engine controller and installed `midnight_ui::install` callbacks. Fixture providers may supply controlled native/archive data; do not replace presenter/controller logic with copied test code.

- [ ] Add an installed bridge test that opens an expanded native tree, invokes actual Ctrl+F12/dialog controls, selects Masks/Regex/Expression, submits OK and feeds the real Engine reply through the presenter. Repeat the archive case and inactive-panel toolbar case. Verify draft/validating/pending/applied/error states and exact command targets.
- [ ] Add real-input tests for Clear, sort headers, Cancel/X, context-row clicks/ranges/rectangles and Ctrl+A. Assert context ancestors never select, top-level-only Select All clears nested marks, collapsed branches do not reappear, and sort never flattens expansion. Cover all three layouts, both panes and a 320x200 window.
- [ ] Test invalid expression after a valid applied expression, failed enqueue, tree-mailbox rejection, delayed reply after navigation and failed preference save. Use a provider fixture that fails on unexpected listing/read calls so UI-only filter/sort cannot accidentally perform provider I/O.
- [ ] Run `cargo test -p midnight-engine --test tree_filters --locked --offline` and `cargo test -p midnight-ui --test tree_filters --test tree_panel --test tree_disclosure --test commander_dialogs --test panel_toolbar --test shared_toolbar --locked --offline`. Run the existing core/engine/UI Ctrl+A regressions from the preceding qualified slice.
- [ ] Integrator runs scoped formatting, changed-package strict Clippy, source-size policy and the required package suites once the candidate is stable; log commands, source manifests, result counts and platform under `_build/qa/tree-filters/`. Broaden testing only for unresolved interactions or changed code.
- [ ] Qualify installed Windows native/archive plugin workflows with the actual app; run corresponding Linux Engine/backend/UI gates where the native runner exists. Report software-renderer versus native application evidence separately and retain any unqualified platform gate explicitly.
- [ ] Update feature status from source/evidence: adopted-tree persistent filters/clear/sibling sort delivered; prefix search, filtered-flat initial admission, watches, container attachment, descendant operations and expanded refresh retention remain open. Correct the stale claim that production panels/controller routing are entirely absent without claiming whole TREE-01 completion.

## Completion conditions and handoff

The slice is complete when the real Engine and installed UI bridge demonstrate native/archive Apply/Clear/Sort, atomic view+selection publication, unchanged root/provider identity, truthful pending/applied states and the failure/cancellation cases above. Focused model tests alone do not complete it. Keep any unavailable installed-platform qualification explicit.

The backend and UI contracts above are fixed for parallel work. Discuss a necessary interface change with the root integrator before either owner silently changes it. The existing source freeze and the integrator's build schedule take precedence over starting implementation immediately; this document itself does not authorize concurrent builds during that freeze.
