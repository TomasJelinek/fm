# Native tree Rename implementation plan

> **For agentic workers:** Use superpowers:executing-plans for this approved scoped implementation.

**Goal:** F2 renames the exact focused native occurrence in an expanded inline tree through the
existing confirmation, overwrite and refresh flow, without granting authority to display paths.

**Architecture:** Add a focused tree request and an opaque backend capture. Preparation resolves that
retained native target and pins it through the existing MutationService, then enters the existing
bounded operation actor as Rename. The UI retains only the typed request/capture relationship and
retires stale native owners on workers. Reuse existing tree action and operation admission budgets;
reserve cleanup before provider capture.

**Tech Stack:** Existing Rust/Tokio tree actor, guarded worker ownership, native mutation services and
Rust/Slint presentation bridge. No dependency, new thread, waiter or archive write capability. Four
cleanup slots, derived from the existing shared tree capture bound, guard new Rename captures
independently of caller/runtime.

**Spec:** Existing TREE_PANEL.md typed-consumer contract plus the approved native Rename scope.

## Design and alternatives

The current operation entry routes Rename through tree_action_allowed and refuses every adopted
projection. F7 already uses the pane's current native directory, as the documented location action;
it must not silently change its destination to a focused expanded folder.

A dedicated focused capture is the chosen narrow extension, following F4's exact view/row admission
and Delete's prepared mutation handoff. Reconstructing SelectedFiles or native paths from display rows
would bypass provider authority and is rejected. A generic tree-write API would broaden this change
without helping the existing one-target Rename workflow and is deferred.

Salamander's CFilesWindow::RenameFile uses the caret (not marked selection), excludes '..', and invokes
its ordinary native Rename dialog. This is a behavior reference only; no fixed-buffer implementation
is copied. Midnight Explorer retains its own confirmation and safe replacement behavior.

## Global constraints and review focus

- The exact acknowledged projection, row, session/navigation/request IDs and cancellation token bind
  capture. Focus wins independently of marks; parent/context/archive occurrences cannot authorize it.
- Captured provider registrations outlive native preparation and prepared mutation pins. Native owners
  close on backend workers, including rejected/full queues, cancellation, stale results and shutdown.
- Flat immediate-parent validation remains unchanged. A tree display caption/path is never authority.
- Existing Rename confirmation, invalid-name refusal, collision/overwrite decisions and recovery apply.
  No mutation before confirmation; Cancel/late confirmation cannot start work.
- Reconciliation keeps expanded native views. Navigation or reprojection before handoff cancels the
  request; admitted native jobs retain their original target rather than following later focus.
- Source file/directory identity replacement after preparation is refused by existing native pins.
  Native links use existing Rename semantics; archive/mixed-provider writes remain unsupported.
- F7 continues to create in the current pane location. No change to selection or creation authority.

## Files and ownership

- New core/tree_rename.rs: TreeRenameRequest, CapturedRenameTarget and PrepareCapturedRename; narrow
  core tree_protocol/protocol/lib declarations only after the shared compile boundary is released.
- New engine/tree_rename.rs and focused tests: exact focused admission, native-only preparation and
  capture lifetime. Tree controller mailbox/worker routes use existing action capture capacity.
- engine/operation_input.rs, operations.rs, operation_worker.rs: guarded one-target Rename input,
  immutable captured scope checks, retained source after pins, existing confirmation/overwrite flow.
- New UI presenter/tree_rename.rs plus operations/tree/input lifecycle hooks: current request fencing,
  backpressure retry, native owner retirement and routing through the established operation dialog.
- Focused engine session/native operation tests, presenter tests and actual F2/F7 UI bridge tests.
- Root owns engine/lib.rs module/dispatch glue and concurrent service configuration migration; do not
  edit those files or the root-owned main.rs/test support constructors.

## Implementation steps

- [x] Add a meaningful presenter/UI RED proving F2 on an adopted native projection emits a typed tree
  request instead of the current unavailable notice; preserve '..' and F7 location expectations.
- [x] Add focused capture RED for descendant/root focus despite different marks, forged/stale/unacked
  projection, cancelled request, archive refusal and capture permit retention.
- [x] Implement TreeRenameRequest and captured native preparation; keep accessors metadata-only.
- [x] Add operation regressions for descendant Rename preview/confirmation, cancellation and immutable scope;
  implement guarded capture handoff and retain provider ownership after prepared target fields.
- [x] Wire the installed UI action and pending capture lifecycle, preserving exact stale-result fences.
- [x] Exercise actual native descendant file/directory Rename and existing collision/replacement path;
  verify original target replacement is refused and unaffected siblings remain untouched.
- [ ] Verify real F2 routing/confirmation and F7 pane-root creation, parent/archive guards and queue
  pressure with targeted UI tests. No Cargo without explicit target lease.
- [ ] Run scoped Windows/Linux qualification and strict lint, independent review and style/diff checks;
  update TREE_PANEL.md and a dedicated verification note. No commit/push by this owner.
