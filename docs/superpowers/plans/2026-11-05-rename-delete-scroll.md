# Rename, delete and scrolling implementation plan

**Goal:** Deliver native rename/delete with an asynchronous operation queue and
fix visible wheel stepping using native evidence.

**Architecture:** Native capabilities prepare identity-bound mutation targets;
Tokio owns confirmations, FIFO jobs and progress. Slint only edits, confirms and
displays messages. Scroll routing/animation remains UI work.

**Spec:** [design](../specs/2026-11-05-rename-delete-scroll-design.md).

## Constraints and review focus

Rustfmt 100 columns, handwritten sources <=100000 bytes, artifacts in `_build`.
Raw native paths and Unicode, OS implementations/tests in separate files.
No silent overwrite/trash fallback, no recursive link following, no UI I/O.
Test stale confirmations, replacement races, queue refusal, inaccessible children,
and cancellation after partial work. Existing changes are committed at 0961c96.

## Tasks

- [x] Native providers: implement `MutationService`/`PreparedMutation` in each OS
  crate with focused isolated-fixture tests. Agree native failure guarantees.
- [x] Engine: add prepared-request and progress protocol, bounded serial mutation
  controller, immutable selection capture, one-shot confirmation/cancellation,
  per-job progress/error handling and delayed-worker/duplicate/refusal tests.
- [x] UI: add rename/confirmation/operations controls, presenter request state,
  F2/Delete/F8 and buttons, exact basename selection, automatic refresh and tests.
- [x] Scroll: reproduce native stepping, fix the proven cause and verify rendered
  pixel intermediates plus idle-only row snapping without breaking touch input.
- [x] Integrate: Windows/Linux tests, Clippy, source guards, normal builds and
  native QA; update security, design, README, shortcuts and verification report.
- [x] Review, commit intended sources/docs and push as previously requested.

## Accepted additions during implementation

- [x] Click blank area after breadcrumb to edit custom path; Enter navigates,
  Escape cancels; remove right-hand path button and preserve segment gestures.
- [x] Middle-click closes tab; persist order, native paths and active tab per pane.
- [x] Exact byte sizes and date/time use backend-captured system locale.
- [x] F7 recursive relative directory creation through the mutation queue.
- [x] F11/F12 native Windows connect/disconnect dialogs; Options after F12.
- [x] Independent Brief, Detailed and large-icon Thumbnails modes, preserving
  navigation/selection and virtualization; document future media libraries/cache.
