# F5 Copy Dialog Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans task by task.

**Goal:** Queue the active panel's captured selection through a working F5 dialog.
**Architecture:** Immutable selection request joins the existing copy controller;
the worker expands paths and uses the same verified copy engine as paste/drop.
**Tech Stack:** Rust, Tokio, Slint; existing dependencies only.
**Spec:** [F5 design](../specs/2026-11-07-copy-dialog-design.md).

## Global constraints

- Backend-only non-UI work; raw native identities, no MAX_PATH ceiling.
- Shared 32-job capacity, no source deletion, independent SHA-256 readback.
- Rustfmt width 100, source files at most 100000 bytes, output under `_build`.

## Review focus

- Pending selection must capture both panel paths at the original gesture.
- Cancellation/repeated submission must not enqueue an unintended copy.
- Edited destination must never reinterpret a lossy default native path.
- Filtering and later navigation must not include hidden or new selections.
- Dialog focus must consume Enter/Escape and preserve file-panel key mappings.

### 1. Backend selected-copy admission

- [x] Add a failing native fixture test that queues an immutable selection and
  verifies only those files are copied; add shared-capacity and invalid-target tests.
- [x] Add `SelectedCopyRequest` and `Command::EnqueueSelectedCopy`.
- [x] Generalize private copy input; expand selection inside the existing worker
  preparation, retaining original admission/cancellation and progress behavior.
- [x] Run copy controller and verified-copy tests.

### 2. Presenter and UI

- [x] Add failing captured-selection/destination, duplicate submit, cancellation,
  admission failure and raw-native-path preservation tests.
- [x] Add a bounded copy draft using the existing selection-intent mechanism.
- [x] Add a reusable Slint copy-request dialog and bridge; route semantic F5 and
  the bottom Copy action through the configurable shortcut system.
- [x] Exercise actual Slint focus/Enter/Escape and queued progress behavior.

### 3. Qualification and documentation

- [x] Update design, feature status and hotkeys without claiming F6/replacement.
- [x] Run relevant tests, formatting, Clippy, source-size checks and independent review.
- [x] Build release, verify launch, commit and push this completed increment (`1440275`).
- [ ] Continue the accepted stage with native replacement/merge and verified Move.
