# Native inline-tree watch implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development or superpowers:executing-plans to
> implement this plan task-by-task. The user already authorized parallel work;
> the coordinator assigns the backend slice and owns the UI slice.

**Goal:** Keep native inline trees current under manual refresh, external changes
and our own transfer notifications without losing session or selection.

**Architecture:** Extend native watch batches and the existing two backend watch
workers with accepted tree scopes and constant-space observations. Reuse the
existing gated all-loaded maintenance sweep, acknowledging a captured dirty epoch
only after successful reconciliation and exact current Commit. UI triggers share
the existing coalesced scheduler.

**Tech Stack:** Existing Rust/Tokio/Slint, notify and sha2 0.10.9; no new dependency.

**Spec:** [Native tree watches](../specs/2026-11-10-tree-watch-design.md).

**Execution record:** [dated verification](../../verification/2026-11-10-tree-watch.md)
records actual coverage and remaining qualification separately from the original
acceptance checklist below. Backend/platform and presenter implementations are
landed in the shared worktree; native UI checks and deployment are coordinated by root.

## Global constraints

- Preserve original tree admission and flat committed-operation snapshots.
- No filesystem work, full signatures or provider/subscription disposal on UI.
- Watch root plus visible expanded native branches; exclude hidden/collapsed,
  directory links and foreign/attached/archive contexts. Re-expand lists freshly.
- One native watcher batch per pane, 1..=32 paths; root plus 31 stable branch slots.
- Two serial backend probe workers; poll every eligible target fairly, including
  unslotted targets. One-second quiet/three-second maximum debounce; 15-second
  target polling and failure retry cadence with documented slow-probe latency.
- Retain at most 10,000 scope targets and 32 MiB native target paths, with
  constant-size signatures; exhaustion is explicit, never silently omitted work.
- Keep platform implementations separate, raw identities, cancellation/resource
  ownership and the 100,000-byte handwritten-source limit.
- Use the shared dirty checkout; do not reset/stage/commit unrelated files.
  Artifacts belong under `_build/qa/tree-watch`, caches under `.tools`.
- No mandatory-fee dependency, new dependency or plugin ABI change. Production
  edits start only after fresh architecture review and shared API freeze.

## Review focus

1. Same-navigation watch baselines must converge after maintenance without
   overwriting original admission or reviving flat action authority (Tasks 2/3).
2. Enrollment/re-registration and late Commit cannot erase a newer file-only
   change, including polling fallback (Tasks 2/3).
3. Collapsed/filtered ancestry and more than 32 expanded branches cannot produce
   stale subscriptions or starvation (Tasks 1/2/5).
4. Native callbacks, blocked probes and full output must release resources on
   cancellation without replacing stuck worker capacity (Tasks 1/3/5).
5. Real F5/F6 notifications must use the same app routing as external filesystem
   changes and preserve marks/caret/current viewer authority (Tasks 4/5).

## Preflight and interface freeze

- [x] Coordinator reads the spec and obtains fresh architecture review. Record
  findings/decisions in `_build/qa/tree-watch/progress.md`; resolve them before code.
- [x] Freeze public API below and confirm UI/backend file ownership. Root owns
  Windows Cargo initially; independent HTML Linux plugin work uses its own target.
  Explicitly transfer leases before any conflicting build.
- [x] Capture baseline versions of touched dirty files into QA artifacts as needed;
  do not create a checkout from HEAD that omits existing implementation.

Frozen public API: add default-Unsupported
`WatchService::watch_directories(&self, paths: &[PathBuf], changed:
Arc<dyn Fn(u32) + Send + Sync>) -> Result<Box<dyn WatchSubscription>, FsError>`;
keep the existing single-directory API. Add `TreeEvent::Changed { stamp:
TreeViewStamp }` and `TreeEvent::WatchFailed { stamp: TreeViewStamp, error:
TreeDiagnostic }`. Keep `TreeRefreshRequest` unchanged. Root may start isolated UI
edits once backend has landed these declarations, without running backend builds.

### Task 1: Batched native subscriptions (platform owner)

Files: core `src/watch.rs`; Windows/Linux `src/watch.rs`;
platform facade re-export review; per-platform watch tests. No UI edits.

Produces the frozen API above. `paths[i]` maps to bit `i`; unknown/overflow/error
maps to all active bits. One `RecommendedWatcher` owns the entire batch.

- [ ] Add tests `batch_rejects_zero_duplicate_and_33_paths`,
  `batch_maps_immediate_children_and_overlapping_targets`,
  `batch_overflow_unknown_and_65_event_paths_signal_all`, and
  `batch_cancel_waits_for_short_callback_and_teardown_releases_owner` in each
  separate OS test module. Assert 32-path admission, correct masks, nonrecursive
  behavior and no callback after subscription destruction.
- [ ] Under the assigned build lease run the affected tests; record the expected
  absent-interface/behavior failure before implementing.
- [ ] Implement the default Unsupported method, then one native
  watcher per valid batch. On partial registration failure cancel/drop the whole
  batch; never leave a subset silently watched. Preserve single-directory behavior
  and direct platform facade availability. Freeze declarations before UI work.
- [ ] Run scoped core/OS watch tests; successful tests and no native leaks are the
  deliverable. Review the exact diff without staging unrelated dirty files.

### Task 2: Accepted scope and streaming signatures (backend owner)

Files: engine `src/watch.rs`; create `src/watch/tree.rs` and
`src/watch/signature.rs` with focused tests; `src/tree_actor.rs`,
`src/providers/tree.rs` and `src/tree_projection.rs` only for model-owned accepted
scope extraction. Keep individual responsibilities below the source-size limit.

Consumes the batch API. Produces `watch::Handle` (cloneable, separate from
Controller Drop), private `TreeScope` with accepted view/session, validated native
targets/provider lease and per-target accepted display signatures. Model scope
extraction is `Session::watch_scope(&mut self) -> Result<Option<TreeScope>,
ProviderError>` and is called only after successful exact Commit; nonnative
sessions return None. `Handle::set_tree_scope(scope: TreeScope)` enqueues latest
bounded control state and checks pending successful-sweep fences.

- [ ] Add tests `scope_excludes_collapsed_filtered_and_foreign_ancestry`,
  `scope_keeps_context_ancestor_and_offscreen_expanded_directory`,
  `scope_retains_targets_and_original_admission`,
  `signature_is_order_independent_and_lossless_with_duplicate_counts`, and
  `folder_display_signature_is_separate_from_full_file_observation`. Assert
  directory-only branch display, ignored computed sizes/localization, and changes
  to file length/time/compression/permissions. Run them red under the lease.
- [ ] Implement model-owned target extraction and checked 10,000/32 MiB scope
  admission. Resolve native target identity from provider references; never use
  display paths. Keep child signatures independent of active projection filters.
  Retain registration and mounted target ownership with the scope.
- [ ] Scope failure preserves successful Commit and reports `WatchFailed` without
  a Commit retry loop. Compute eligibility before fallible construction; retain
  only same-session still-eligible old targets, or disable all if eligibility is
  unknown. Keep accepted identity and paced retry even with zero targets. Do not
  acknowledge a sweep ticket on failure; deduplicate unchanged diagnostics and
  cache listing/signature work across unchanged Commits.
- [ ] Implement tagged per-row SHA-256, wrapping 256-bit digest addition and
  checked count, domain-separated final digests. Stream one row; validate immediate
  child/raw-name identity and cancel between rows. Retain no full file vectors.
- [ ] Add/run tests `stable_slots_cap_one_batch_and_poll_all_remaining`,
  `unchanged_commit_keeps_debounce_and_observations`,
  `first_probe_and_batch_gap_preserve_dirty_interval`, and
  `failed_probe_retains_history_and_retries_at_poll_cadence`. Inject short Timing
  and barriers. Implement stable root+31 slots and fair serial polling over every
  eligible target; Unsupported/batch failure polls all, with no extra watchers.
- [ ] Verify green unit tests and review scope/signature lifecycle. Record any
  resolved interface refinements before modifying controller integration.

### Task 3: Epoch, maintenance and exact Commit integration (backend owner)

Files: core `src/tree_protocol.rs` event declarations; engine `src/lib.rs`, `src/tree_controller.rs`,
`src/tree_controller/worker.rs`, `worker_output.rs` only if necessary,
`src/tree_refresh.rs`, watch tree state/tests and `tests/tree_refresh/controller.rs`.

Consumes `Handle::set_tree_scope` and existing `RefreshLoaded`. Produce private
`RefreshTicket` capturing pane/navigation/session/request and dirty epoch through
`Handle::begin_refresh(stamp: TreeViewStamp, request: TreeRequestId) ->
Option<RefreshTicket>` after successful admission;
`Handle::finish_refresh(ticket: RefreshTicket, success: bool,
final_revision: TreeRevision)` records failure pacing or the success fence;
`Handle::clear_scope(pane: PaneId, navigation: NavigationId)` retires old scope.
The subsequent successful exact Commit/set_tree_scope clears only the captured
epoch. Dirty/epoch fields remain private and never enter public requests/events.

- [ ] Add/run red tests `same_navigation_success_then_commit_quiets_watch`,
  `callback_between_admission_and_commit_survives_ack`,
  `file_only_poll_between_sweep_and_commit_survives`,
  `failed_sweep_and_ignored_hint_retry_without_busy_loop`,
  `late_flat_ack_cannot_replace_adopted_scope`, and
  `stale_scope_ticket_cannot_affect_next_navigation`.
- [ ] Inject optional watch Handle into tree workers. Capture tickets only after
  accepted maintenance, finalize them at correlated sweep completion and process
  fences on exact current Commit. Session revisions may advance after the sweep;
  require same session and revision at least its fence. Preserve observation state
  across accepted baseline updates and stamp Changed from accepted scope only.
- [ ] Retain existing output-completion/mutation-permit behavior. Add/run tests
  `full_output_does_not_strand_completed_gate_or_scope_cancellation`,
  `blocked_probe_retains_provider_until_return`, and
  `callback_mask_and_shutdown_remain_bounded`. No lock spans probe/output waits;
  callback epoch capture/ack is serialized without lost increments.
- [ ] When preferred maintenance cannot acquire the mutation gate, run queued
  explicit Expand using ordinary branch/listing permits; make `waiting_on`
  consistent. Prove a different-branch Expand completes while Copy holds the gate,
  then the retained sweep completes after release. Keep same-branch precedence.
- [ ] Invalidate callbacks synchronously at controller navigation/close, even with
  blocked probes/output. Retire owners off-lock on backend workers; bound active
  and latest-pending ownership. Check due hints/control between serial probes;
  document that blocking work can delay the native debounce window too.
- [ ] Run engine watch, tree-refresh and controller integration tests. Verify
  ordinary flat watch behavior, exact root action aliases, independent panes,
  provider-error retention, and existing explicit-expansion precedence remain green.

### Task 4: Presenter routing and continuity (root owner)

Files: UI `src/presenter.rs`, `src/presenter/archives.rs`,
`src/presenter/tree.rs`, `src/presenter/tree_refresh.rs`; presenter watch/tree
refresh/tree tests. App `main.rs` and `visible_sync.rs` only for necessary routing
or shared helper extraction; do not replace flat caches with tree snapshots.

Consumes frozen `TreeEvent::Changed`; no paths/epochs required. Existing
`queue_native_tree_refresh` and `flush_tree_refresh` remain the single scheduler.

- [ ] Add/run red tests `manual_native_tree_refresh_retains_session`,
  `legacy_root_hint_queues_tree_maintenance`,
  `changed_hint_accepts_current_session_across_projection_revisions`,
  `changed_hint_rejects_old_navigation_or_session`, and
  `watch_maintenance_waits_for_user_and_completion_selection`. Assert no
  Navigate/Refresh/clear_tree for adopted native scope and pending retry on full
  outer queue. Keep nonnative/flat routing tests unchanged.
- [ ] Route manual native Refresh, legacy root hints and Changed through the
  existing dirty scheduler. Do not special-case successful transfers or swallow
  provider errors. Update the old tree-exit-on-refresh regression to use actual
  navigation so it continues testing fresh flat selection context.
- [ ] Add/run tests preserving marks/caret/search/filter/sort through structural
  publications, file-only size invalidation and late canceled size results. Run
  affected UI tests using the root-owned Windows build lease.

### Task 5: Real native watch/UI regression and qualification (root coordinates)

Files: UI `tests/support/tree_filters.rs`, `tests/tree_move.rs`; create
`tests/tree_watch.rs` as needed; engine `tests/watch_native.rs`; platform-specific
fixture helpers remain separate. QA logs/progress under `_build/qa/tree-watch`.

- [ ] Add a watch-enabled native harness constructor accepting `Arc<dyn
  WatchService>`; use real `Engine::configure_watch` admission and installation. Synchronize accepted flat listings
  through PaneCommitted and intercept DirectoryChanged through the same production
  `refresh_if_changed` path. Changed goes through normal tree event application.
  Track registration barriers and later debounced notifications so merely seeing
  terminal transfer completion cannot satisfy the retention assertion.
- [ ] Run a failing reproduction before the routing fix when possible. Test both
  source panes, F5 and F6, already expanded destination, surviving nested marks,
  correct independent destination bytes and F3 for newly added root files. Wait
  for actual native callbacks and verify continued observation after the transfer.
  A successful terminal sweep may acknowledge own-write events before debounce
  and correctly suppress a redundant hint; require a later external change too.
- [ ] Add external root and expanded-branch create/remove/rename, file-only change,
  manual refresh, collapsed mutation/fresh re-expansion, filter context, 33+ branch
  polling fallback, navigation during a blocked probe and independent same-path
  panes. Use private `_build` fixtures and Unicode names; retain raw Linux names
  in backend/native tests where the UI caption is necessarily lossy.
- [ ] After focused green runs, coordinate Windows and separate Linux target
  qualification with explicit leases. Run formatting/source-size and scoped lint
  plus affected engine/UI/platform integration suites; broaden only for failures
  or unresolved concerns. Record exact commands/counts/limitations, including
  software-window versus installed GPU scope. No deploy/commit is implied here.
- [ ] Fresh implementation review resolves actionable findings before updating
  `docs/developer/TREE_PANEL.md`, relevant security/verification notes and progress.
  Do not claim archive watch, generic plugin migration or complete TREE-01 parity.
