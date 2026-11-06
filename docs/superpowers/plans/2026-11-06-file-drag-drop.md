# File drag and drop implementation plan

Use the accepted [design](../specs/2026-11-06-file-drag-drop-design.md), the existing
backend copy queue, and named independent UI/native owners. Keep native code and
tests in per-OS files. User-requested clean rebuild runs while implementation
continues; final verification must compile the final source again.

- [x] Inspect Salamander, pinned Slint/winit source and primary native protocols.
- [x] Verify native registration/input handoff before relying on cross-thread APIs.
- [x] Add typed contracts, bounded sessions, copy-capacity reservation and admission.
- [x] Add captured-selection gestures, prepared-token commit/cancellation, target
  geometry and UI feedback without filesystem work on the UI thread.
- [x] Implement independent Windows and X11/XWayland native sources/targets,
  Unicode/raw-byte codecs and cancellation/cleanup. Retain internal fallback.
- [x] Add meaningful regressions, perform native protocol and verified-copy checks,
  review changes, update design/shortcuts/security, then finish release validation.
- [ ] Commit and push all reviewed, authorized source changes; exclude build output.
