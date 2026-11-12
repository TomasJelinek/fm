# Native inline-tree Delete implementation plan

> For agentic workers: implement task by task using the executing-plans workflow.

**Goal:** Delete native selected or focused inline-tree occurrences through ordinary confirmation,
Trash/permanent execution and shared progress, without treating display paths as authority.

**Architecture:** An immutable typed tree request captures exact acknowledged occurrences and retained
provider registrations. The backend suppresses descendants of selected ordinary directories, pins
all remaining native targets before preview, and reuses the existing mutation queue and worker.

**Tech stack:** Rust core/engine contracts, bounded Tokio workers, existing separate Windows/Linux
MutationService implementations. No new dependencies or OS deletion APIs.

**Spec:** docs/developer/TREE_PANEL.md and the parent task's approved typed Delete capture design.

## Constraints and review focus

- Preserve flat PrepareOperation exact snapshot and immediate-parent protections.
- Native identity is pinned during preparation before confirmation; capture is not a filesystem snapshot.
- Reject unsupported/mixed provider captures before opening any mutation targets. Never downgrade Trash.
- Use occurrence parent IDs for overlap suppression; distinct hard-link names remain distinct targets.
- No parent pseudo-row, context-only/foreign/stale occurrence, or display-string authority.
- Keep final provider destruction on backend workers, including rejection, cancellation and shutdown.
- Four shared tree capture permits and the existing 32-operation/2-preparation bounds remain enforced.
- Root owns UI integration, confirmation, exact pending capture correlation and loaded-tree refresh.
- No commits. Cargo runs require the parent's coordinated lease; all test artifacts stay under _build.

## Tasks

- [x] Add focused capture regressions for parent+descendant selection, descendant fallback, exact view/
  selection acknowledgements, immutable Trash/permanent intent, cancellation and capture permit release.
- [x] Add core tree_delete.rs contracts: TreeDeleteRequest, CapturedDeleteSources, TreeDeleteCapture,
  PrepareCapturedDelete. Wire TreeCommand::Delete and TreeEvent::DeleteCaptured.
- [x] Implement engine tree_delete.rs: capture_selection, all-native preflight, parent-chain suppression,
  bounded retained lease, worker-only native preparation and exact returned-path verification.
- [x] Add captured input to existing operation admission, using the shared high-water mark/queue and
  worker retirement guard. Preserve ordinary Preview/Confirm and cancellation behavior.
- [x] Add native controller regressions for deletion of multiple parents, no work before confirmation,
  cancel/replay/stale replacement and off-worker retirement; preserve all flat admission regressions.
- [x] Run targeted core/engine tests under the lease; inspect failures and scoped strict Clippy.
- [x] Obtain independent review, document actual evidence and hand exact interface/risks to root.

## UI contract

TreeCommand::Delete(TreeDeleteRequest { mode, view, request, selection_revision, selection, fallback,
cancel }) returns TreeEvent::DeleteCaptured { stamp, request, result }. The successful capture contains
mode, post-dedup root_count and opaque sources. Send Command::PrepareCapturedDelete with id, sources
and source_directory (display/refresh metadata only); ordinary OperationEvent::Preview then controls
confirmation. Reject/stale/dismissed source ownership must use backend DeferredDrop retirement.

Ruling: retain ordinary mutable-subtree semantics. Current model and core projection validation
reject inline link expansion; the additional ancestry guard is defensive for future shapes.
Selected roots are pinned before confirmation; later descendant replacements remain governed
by the existing native no-follow delete behavior, without a claim of frozen descendant identities.

Qualification: Windows backend23 and presenter8 pass; real native Slint/Engine test1 passes.
Native Linux backend23 and strict core/engine library Clippy pass. See
../../verification/2026-11-12-native-tree-delete.md for logs and corrected fixture/review premises.
