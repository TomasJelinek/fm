# Confirm cancellation in operation progress

Scope: the existing shared Copy/Move/Delete progress window. Jobs history and
backend cancellation semantics remain unchanged. The question belongs to its
existing native window, so other operations and the main window remain usable.

- [x] Capture an actual Escape regression against immediate cancellation.
- [x] Make close/Escape/footer Cancel open a question; focus Continue by default.
- [x] Make Continue, question Escape and question X dismiss only the question.
- [x] Confirm only the current window session and current active/cancellable job.
- [x] Preserve the question and retry feedback if cancellation admission fails.
- [x] Clear stale questions on terminal updates; never alter pause state.
- [x] Cover Copy, Move, Delete, native X, keyboard, rejection, completion race,
      stale windows, independent windows and compact/theme layouts.
- [x] Run focused Windows regressions, then hand off final combined qualification.

Implementation: keep prompt visibility in the existing per-job window record's
Slint state. A separate confirmation callback rechecks the immutable captured
identity/session and fresh presentation row before dispatch. The question replaces
only this window's interactive body with the existing themed header/footer/button
controls. No extra native dialog, backend command, polling, I/O or global modal
state is introduced. Successful admission preserves the existing dismissal policy;
failed admission retains an actionable question. Opening/dismissing the question
never sends Pause or Resume.

Evidence: [focused verification](../../verification/2026-11-12-progress-cancel-confirmation.md).
