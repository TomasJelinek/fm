# F5 copy dialog and captured-selection admission

## 13 November initial-form amendment

**53-path batch has bounded headless-fixture qualification only.** Root's
15 November batch includes policy request routing, typed waits, current-pass
presentation, conditional Move SHA and initial-form polish, including the reviewed
paused/cancel header fix. Qualification is limited to headless producer/UI
fixtures; concurrent scheduling/native retirement and fresh app delivery remain pending.
Every new
Copy/Move dialog starts with **Wait for other operations to finish** unchecked;
the choice is not persisted. The accepted scheduling policy requires concurrent
runnability under worker/resource and safety limits; the scheduler cutover is
still pending. Checking the option waits only for earlier
unfinished Copy/Move admissions, not future jobs; failed/cancelled predecessors
release it after actual cleanup. Freeze the choice with the submitted job and
retain it through rejected admission/reconfirmation and Retry. F5/F6, clipboard,
drop, archive and captured routes default to
Immediate unless the user explicitly selects waiting. The destructive-mutation
verified-success fence remains separate.

Polish the initial form so source/selection, the captured default destination,
verification choice, waiting checkbox and actions remain readable and usable at
compact sizes and enlarged fonts. Preserve captured/raw identities, destination
editing, Enter/Escape behavior, single admission and the existing application
minimum; prove layout with targeted input tests and snapshots. Move's SHA-256
note must say **when copying is required**: native rename reads/hashes no content,
while copied data uses the job's selected full/quick verification policy.

Progress separates explicit earlier-transfer waiting, capacity waiting and
actual discovery, then shows current-readback bytes and a fresh 0..100% chart per
file/attempt. The [progress amendment](../../developer/operation-progress.md)
defines those meters. The first increment and dated evidence below remain
baseline history; they do not qualify this new form or concurrent execution.

## Original 7 November increment - retained history

This implements the first UI increment of the accepted copy/move stage. F5 and
the bottom Copy button open a compact dialog with the captured source location,
selection summary, editable destination folder (the other panel by default), Copy
and Cancel. Enter submits; Escape cancels without creating a job. After admission,
the existing Jobs view owns progress, cancellation and conflicts. Browsing remains
available and another copy can be queued. The system clipboard is untouched.

Use the existing verified-copy queue rather than a second copying implementation
or a clipboard round trip. The latter would replace user clipboard contents and
make admission depend on platform clipboard ownership. Expand the captured
selection on the backend worker, retaining the same 32-job queue budget, FIFO,
cancellation, 256 KiB transfer buffer and independent SHA-256 destination readback.

Selection gestures already have an exact-revision waiting mechanism. Reuse it:
capture the source and other-panel paths when F5 is pressed, then retain the
resolved immutable selection/view and caret fallback. Navigation or later selection
cannot retarget the dialog. Preserve raw native paths when the default displayed
destination is unchanged. Edited destinations must be absolute existing folders;
no shell expansion, implicit directory creation or filename-mask interpretation.
Archive/provider panes without native transfer capability reject the action.

The UI performs no enumeration or file access. Repeated confirmation is consumed
once; stale tokens and rejected admission never create phantom jobs. Retire
discarded selection snapshots through the existing backend reclamation path.
The form is a persistent modal layer: an asynchronous notice cannot discard its
edited target. Tab/Shift+Tab remain within its controls, and native drop regions
stay disabled until the form closes. After a notice, focus returns to the editor
without resetting its contents.

The initial Copy/Move form reserves enough height for the source subject and path,
destination label/editor, modified-time option and action buttons: at least 360
logical pixels, scaling to 20 times the configured font size. The containing
window still bounds the dialog and manual resizing remains available. Longer
source descriptions scroll; the destination remains a single-line editor. This
replaces the former 300px initial height that clipped an ordinary four-line source
summary even in a spacious main window.

Salamander reference: `src/dialogs3.cpp` CCopyMoveDialog/CCopyMoveMoreDialog uses a
source subject and destination editor, selecting the initial destination for
quick replacement. `src/dialogs.cpp` COverwriteDlg shows separate source/target
metadata and per-job decisions. No GPL source or assets are copied.

## Current Move policy

The later user instruction supersedes whole-batch source retention for Move.
Each file is independently copied and verified, then its source is removed before
the next file. F6 and Cut/Paste use ordinary Move, with no recovery opt-in,
catalogue or journal. Temporary internal staging supports exact-object removal;
pre-delete failures/cancellation restore without replacement, and exceptional
restore conflicts report the retained path without overwriting a newer source.
Successful Move retains no source copy or empty staging directory. Current native
capability and safety limits are defined in the
[transfer contract](../../developer/transfers.md); Copy remains source-preserving.

Directory merging and explicit per-file replacement are available through the
existing conflict flow. Destination replacement retention is a separate declared
policy; it does not turn source Move into a recovery product. Directory Move and
broader native deletion capability remain unfinished scope. Native dialog-window
migration is in progress; this document does not claim every form already moves
outside the main application window.
