# F5 copy dialog and captured-selection admission

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

Salamander reference: `src/dialogs3.cpp` CCopyMoveDialog/CCopyMoveMoreDialog uses a
source subject and destination editor, selecting the initial destination for
quick replacement. `src/dialogs.cpp` COverwriteDlg shows separate source/target
metadata and per-job decisions. No GPL source or assets are copied.

Replacement, directory merging and F6 require the next native transfer increment;
this dialog uses the currently supported Skip / Skip all / Copy as / Cancel
conflicts. Their absence is explicit in feature status, not implied implemented
by adding F5. The broader stage retains the no-source-deletion-before-whole-batch-
verification rule and the conditional-overwrite constraints in the main design.
# Move policy update

The later user instruction supersedes whole-batch source retention for Move.
Each file is independently copied and verified, then its source is removed before
the next file. Current native capability and safety limits are defined in the
[transfer contract](../../developer/TRANSFERS.md); Copy remains source-preserving.
