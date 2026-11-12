# Native operation confirmations and Copy/Move drafts

11 November 2026. Implements Task 2 of the accepted
[native-dialog plan](../superpowers/plans/2026-11-10-native-dialog-windows.md).
This slice covers Rename, Create directory/file, Trash/permanent Delete, and
captured F5/F6 Copy/Move drafts. Jobs, conflicts, metadata, directory comparison,
filters, Options, color picker and Hot Paths remain separate migration work.

## Implemented ownership contract

`OperationWindow` reuses the existing `OperationDialog` body. The typed host
retains only a UI session and either `OperationId` or the exact copy-draft token.
Prepared filesystem resources and provider-bearing captures remain in Presenter
and backend services. Draft cancellation uses the existing retirement path.

The shared `DialogHost` creates an owned, resizable native top-level before show.
Ordinary dialog taskbar policy, owner input gating, initial monitor placement and
focus restoration are reused. Explicit close restores the invoking control;
background draft retirement does not redirect focus to the file pane. Native
forms retain their action artwork beside the summary even though the OS titlebar
replaces the embedded header. The current matching form remains alive through
refresh, theme/font changes and resize; its input, caret and metadata checkbox
are initialized once. If asynchronous operation and transfer previews overlap,
the currently matching child remains visible and the other waits for its turn.

Return from the input, option or primary action proceeds; empty names do not.
Cancel, Escape and native X cancel the matching occurrence. A rejected backend
cancellation keeps the child visible and disables proceeding while retrying only
cancellation. If a nested timer fires during another Presenter update, it defers
without taking a conflicting borrow. Failed copy-host cleanup likewise keeps a
single retry chain scoped to the exact draft token. Old retained component callbacks must match both UI session and
current captured identity. Creation/show failure must not leave an invisible
prepared operation or recreate the failed host on every refresh.

Copy admission and validation feedback stays in the child. Only a newly generated
notice from that synchronous submission is consumed; unrelated earlier notices
remain separate. Native creation failure is reported to the main owner after
cleanup is scheduled. The metadata option preserves the existing warning that
unsupported precision fails and earlier completed copies may remain.

## Recorded evidence so far

- Baseline: `0ab38de`, authoritative `D:/experiments/dev/FM`.
- Internal failure RED: two expected failures because embedded forms did not
  create a native host or cancel a failed prepared operation; one existing
  unrelated readonly-archive control passed. Log:
  `_build/qa/native-operation-windows/internal-red.log`.
- Native integration RED: all six new cases failed at the exact missing-child
  assertion (zero distinct visible child adapters, expected one). Log:
  `_build/qa/native-operation-windows/integration-red.log`.
- First implementation compilation identified an inherited Slint `Window.title`
  setter unavailable to Rust. The wrapper now exposes `title-text` explicitly;
  this compiler failure is not counted as a passing or behavioral test result.
- Timer RED reproduced two `RefCell already borrowed` panics in explicit Cancel
  retry and failed copy-host cleanup. Four lifetime/control cases passed, and the
  real Windows desktop probe was intentionally ignored. Log:
  `_build/qa/native-operation-windows/lifetime-red.log`.
- After bounded borrow deferral, the same filtered unit gate passed all six
  nonignored cases (0.04 seconds; 49.11-second incremental build). This includes
  stale retained-window callbacks, old failed-host replacement, and exact
  cancellation after create/show failure. Log:
  `_build/qa/native-operation-windows/lifetime-green.log`.
- The first ten-target integration/layout gate passed 35 cases, including all
  three real Engine copy/move/delete fixtures. The native-body icon regression
  correctly found zero marker pixels. A synthetic transfer loop also exposed a
  fixture that left the automatic Jobs popup open; the fixture now dismisses Jobs
  with actual Escape after asserting the captured enqueue. Log:
  `_build/qa/native-operation-windows/integration-icon-red.log`.
- An actual native child Cancel regression then proved the old copy projection
  replaced the invoking command editor with file-pane focus. That obsolete
  refresh side effect is removed. The internal background-retirement fixture was
  corrected to open a clipboard Move draft: clipboard Copy intentionally submits
  immediately, so its first sentinel failure is not a product RED. Log:
  `_build/qa/native-operation-windows/focus-icon-red.log`.
- Final filtered unit gate: **7 passed, 1 ignored**, 0.04 seconds after the
  6m23s binding/Rust build. The ignored case is the separately scheduled Windows
  desktop probe. Background retirement and both timer regressions pass. Log:
  `_build/qa/native-operation-windows/final-unit-green.log`.
- Final integration gate: **98 passed across 23 targets, none failed or ignored**.
  This includes all ten users of the changed tree harness, the three real Engine
  operation cases, and nine native-host integration cases. The latter verify
  actual icon/fallback pixels, preserved invoking-editor focus/caret, both panes,
  native close, queue refusal, and 320×200 appearance combinations. Log:
  `_build/qa/native-operation-windows/final-integration-green2.log`.
- Five bounded representative captures were written under
  `_build/qa/native-operation-windows/renders/`. Inspection of the permanent-delete
  capture confirms visible action artwork, warning text and footer controls.
- A remaining-fixture audit identified `delete_confirmation` as one additional
  target. Its actual-child text, localized metadata and exact previous-identity
  cancellation assertions passed (1 test, 0.81 seconds). Log:
  `_build/qa/native-operation-windows/delete-summary-green.log`.
  The combined Windows integration count is **99 passed across 24 targets**.
  Visual inspection rejected its initially blank capture: the fixture combined a
  fresh pixel buffer with partial redraws. Only this fixture now requests full
  redraws; the image rerun remains pending. The other five captures use full
  buffers and were unaffected.
- Windows desktop qualification passed in two separate processes: the actual
  Rename/Copy adapter probe and generic owned-dialog probe. The former recorded
  actual HWND/owner identities, outside-owner movement, resizing, no separate
  taskbar style, exact WM_CLOSE cancellation and owner re-enabling.
- The final Windows UI library run passed 828 cases (18.16 seconds), excluding
  those two opt-ins from its ordinary invocation. Strict application/host Clippy,
  changed-source formatting and the 100,000-byte source-size check passed.
- Fresh debug build, protected protocol-3 runtime, software/native startup images
  and current Linux results are recorded in the
  [parallel follow-up](2026-11-11-queue-followup.md). Linux residual checks and
  native compositor qualification must not be inferred from Windows success.

Existing file-operation assertions are retained in the owned Engine fixtures.
Their keyboard input is being redirected to the actual child window; a direct
callback-only test is not a substitute for this evidence. The shared tree test
harness now supplies separate adapters, so all its consumers are included in the
affected integration gate. No real user files or external editor programs are
used by this slice's tests.

Windows native evidence must additionally identify the real owner HWND, absence
of a separate taskbar style, independent outside-owner movement, native close and
owner re-enabling. Software-adapter rendering alone does not prove these OS
properties. X11/Wayland qualification follows the existing native-dialog contract:
Wayland has application ownership and compositor placement, without claiming a
public Winit transient-parent API that does not exist.
