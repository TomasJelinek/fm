# Inline tree deletion UI implementation plan

> **For agentic workers:** Use superpowers:executing-plans; core/engine work is separate.

**Goal:** Route F8/Shift+F8 from native inline branches into the existing delete confirmation
and shared operation progress window, retaining expanded branches after completion.

**Architecture:** Capture exact acknowledged tree selection via TreeDeleteRequest and validate
DeleteCaptured scope, mode and root count. PrepareCapturedDelete carries opaque backend authority;
no displayed path enters flat mutation admission. Reuse common operation registration/confirmation.

**Tech Stack:** Existing Rust presenter, Slint operation windows, bounded backend commands.
**Spec:** docs/developer/TREE_PANEL.md; docs/developer/OPERATION_PROGRESS.md.

## Constraints and review focus

- Never target parent row, unavailable filesystem or unacknowledged selection.
- Correlate source stamp/request as well as event envelope, mode and count.
- Cancel/retire captures after navigation, reprojection, replacement or shutdown.
- Failed queue admission must create neither confirmation nor Jobs receipt.
- Capture provider destructors run off UI, including rejected/duplicate results.
- Accepted preparation uses the existing explicit confirmation and cancellation policy.
- Terminal mutation refresh must preserve loaded native branches and use backend reconciliation.

## Tasks

- [x] Add presenter regressions in tests/tree_delete.rs for both modes, exact delayed
  selection, stale/invalid captures, backpressure, parent/archive fences, cancel and refresh.
- [x] Add presenter/tree_delete.rs with Pending and request/cancel/flush/result handling,
  using existing tree_capture::Capture and deferred retirement.
- [x] Wire module, pending operation field, tree event lifecycle and shutdown.
  Refactor ordinary Preparing registration to share one helper with captured deletion.
- [x] Replace the old tree Delete fence expectation with typed capture expectation;
  leave Rename/metadata restrictions intact.
- [x] Run focused tree/delete/history/operation tests, affected Clippy and formatting.
- [x] Integrate engine native evidence and peer review; update TREE_PANEL and feature status.

## Targeted evidence

`_build/qa/tree-delete-20261112/ui-green.log`: eight new presenter regressions
passed in 0.02 s. They include rejected cancellation, exhausted operation IDs,
duplicate capture delivery and failed preparation enqueue as well as the planned
scope/lifetime cases. `operations_native_ui.rs` additionally exercises expanded
native branch deletion through the installed key handler and common confirmation;
all four native operation tests passed, including this case. Full affected suites,
strict Clippy and the debug build passed; see
[the integration record](../../verification/2026-11-12-tree-delete-integration.md).
