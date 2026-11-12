# Native Jobs and operation decisions

This is Task 3 of the accepted [native dialog plan](../superpowers/plans/2026-11-10-native-dialog-windows.md). It follows the native OperationWindow checkpoint `5aee4d2`. The current user instruction prioritizes targeted meaningful checks; broad final regression runs are deferred until the implementation queue is complete.

## Implemented scope

- Jobs uses an owned, modeless `JobsWindow`. Closing its client Close button, Escape, or native X closes presentation only. Explicit Cancel job still uses the actual backend job identity.
- The existing Presenter job rows, bounded history, automatic-owner token, and speed-chart cache remain authoritative. Progress updates reuse the child body, preserving selection and scrolling. Manual Jobs stays open; automatic successful progress retains its existing close conditions.
- Copy/Move errors, name conflicts, and metadata conflicts use modeless `DecisionWindow` occurrences. Jobs remains visible behind them. Each callback requires its current DialogSession and exact backend job/conflict token.
- Escape and native X send Cancel for the exact waiting operation. Decide later is a separate deferral action. Failed command admission keeps the same child and editable draft with an inline diagnostic.
- Selecting another metadata job in Jobs retains that exact conflict through refresh. A newer token retires the old native occurrence; a callback from its old window has no authority.
- Native host creation failure leaves the backend decision pending. The existing bounded failure reporter shows an owned Notice, with no backend cancel/retry callback. Progress does not repeatedly create a failing window; Jobs Resolve is the explicit retry route. Jobs presentation failure uses the same notice path and an explicit reopen.
- Native titlebars, owner styles, taskbar suppression, placement, scroll hooks, and weak focus return use the existing DialogHost. Ordinary progress does not call explicit activation. Existing native attribute policy requests initial activation only when the owner has focus.

No engine operation, source-deletion, retry, verification, metadata editing, or filesystem protocol semantics are changed by this migration.

## Targeted evidence

The initial five installed-Presenter regressions compiled against the embedded implementation and all failed at the expected independent-visible-adapter assertion: actual zero children, expected one or two. Log: `_build/qa/native-jobs-windows/red.log`.

The corrected Windows gate passes **21 tests**: `native_job_windows` 9, `copy_errors` 9, and `jobs_dialog` 3. Log: `_build/qa/native-jobs-windows/green.log`. This includes exact Retry/Skip/Cancel, Later, stale child callbacks, two metadata jobs, manual/automatic Jobs lifetime, and failed native creation/recovery.

Both failed-host regressions first failed on Windows and Linux because the transient main status was overwritten or hidden. The final Windows tests require actual visible diagnostic text in an owned Notice at 320×200, dismiss that Notice, and recover through an explicit reopen/Resolve. They reject one host creation while permitting the fallback Notice; they do not simulate loss of every native-window facility. Corrected RED log: `_build/qa/native-jobs-windows/corrected-red.log`.

The first integration pass also identified fixture mistakes: drop tokens were mistaken for independently assigned operation IDs, and native titlebar suppression left a hidden duplicate Cancel control in the accessibility tree. Fixtures now assert the actual published job identity and select visible client controls. No backend semantics were changed for those failures.

Existing `jobs_dialog` and `copy_errors` tests were moved to real native body/adapter fixtures, retaining their backend-command assertions, terminal-history cases, row selection, speed-curve geometry, and compact controls. Additional direct Jobs cases in `list_scroll` and `tool_reflow` retain actual native wheel/pixel and scroll-geometry checks. Popup takeover remains a separate Options/ComboBox regression because Jobs itself has no such popup.

The native-operation transfer fixture now closes the separate Jobs child through its actual Escape route after checking the exact F5/F6 request; it no longer assumes an embedded Jobs popup.

## Follow-up evidence and integration status

- **Linux follow-up passed:** `native_job_windows` 9/9 and `copy_errors` 9/9; log `_build/qa/native-tools-20261111/linux-jobs-green.log`. The preceding representative native-body layout target also passed 3/3.
- **Windows follow-up passed:** `job_speed` 8, native Jobs list/scrollbar cases 2, compact Jobs reflow 1, popup takeover 1, and the changed native-operation transfer case 1. Logs: `_build/qa/native-jobs-windows/{job-speed,scroll,popup-scroll,transfer}.log`. Together with the 21 integration cases above, this is **34 passing targeted tests** with no failures.
- Direct native-operation consumers passed on Windows and Linux: native operations, tree clipboard, tree copy and tree move. The two legacy name-conflict fixtures also pass through installed native decisions, retaining every exact backend choice and Unicode Copy As; see the combined qualification below.
- Representative native body layout/framebuffer checks passed 3/3 on both Windows and Linux.
- **Actual Windows desktop probe passed:** `native_jobs_close_preserves_work_and_background_decision_preserves_foreground`, 1/1 in 0.65s. Log: `_build/qa/native-tools-20261111/windows-native-jobs-desktop.log`. The probe checks actual HWND owner/style (no separate taskbar entry), keeps the modeless owner enabled, confirms that native Jobs close sends no backend cancellation, and verifies exact `Cancel 1:9` on the decision's native WM_CLOSE. Both background decision show and close preserve the observed independent foreground window.

The initial foreground setup attempt was refused by Windows. The corrected probe obeys foreground policy: it activates only its own independent test window when permitted, otherwise observes an already independent foreground HWND without sending input to it. The passing evidence establishes preservation of that observed foreground window; it does not identify or qualify a particular other application.

Implementation, coordinated integration and targeted qualification are complete for this slice. The fresh debug build, protected deployment and three startup checks passed; see the [combined batch record](2026-11-11-native-tool-windows.md). The broad final regression gate remains deferred until the planned implementation is complete.
