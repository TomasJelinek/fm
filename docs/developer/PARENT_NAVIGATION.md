# Returning to a parent folder

Opening `..`, pressing Backspace, clicking the panel's Up button, or choosing an
ancestor breadcrumb focuses the immediate child directory through which the
previous location was reached. This changes the keyboard cursor only; it does
not add checkbox selection. Existing cursor revelation scrolls that row into view.

Native navigation carries the raw child `PathBuf` in the accepted listing
request. Streaming entries can resolve it before completion, and the final sorted
index resolves it again by raw path, independently of reassigned entry IDs. An
ancestor breadcrumb chooses the immediate child of that ancestor, rather than a
deeper descendant. Archive navigation uses the mounted provider's node identity
from the location trail; equal display names cannot redirect the cursor.

A rejected navigation does not change the cursor. Failed listings restore the
previous view; stale generations cannot apply a focus hint. Deliberate cursor
movement during loading, including back onto `..`, overrides automatic return
focus. A missing child is not inserted. Persistent filters keep a visible child
focused and otherwise use the existing visible-row fallback; pending filters
continue to prohibit operations on hidden rows.

Cursor revelation waits for the virtualized repeater to expose sufficient content
height after model reset. The pending request captures the existing location
input-context token, path and cursor; a superseding location/cursor invalidates
it, including locations with equal display captions. User scrolling or selection
autoscroll cancels the request. The latest explicit reveal remains valid for
later geometry refinement, including a newly appearing scrollbar shrinking the
viewport. Reapplication only moves a clipped cursor; ordinary watch/size
refreshes do not create a new reveal or override user scrolling.
The pinned virtualized ListView can also normalize a large jump to a row
boundary, discarding a fractional-row offset. The same guard rechecks its
content-position change so the complete cursor row remains visible.

The interaction follows Salamander's parent navigation in
`src/fileswn2.cpp`: its `..` branch passes the removed directory component as
`suggestedFocusName` to `ChangePathToDisk`. This implementation retains raw native
paths and opaque archive identities rather than matching displayed captions.

Focused regressions are in presenter `parent_focus` tests and the software-window
integration target `parent_focus`. The latter dispatches Backspace, Enter,
double-clicks on `..`, and a pointer click on Up, then checks that the returned
child at row 151 is visible and unchecked. It checks the first rendered frame,
including the actual cursor-edge pixels, without a subsequent settling draw.
The backend-event cases run timer/change callbacks before event delivery and
render immediately afterward, matching the pinned Winit event ordering. They do
not grant deferred reveal callbacks an extra event-loop turn before first paint.
A streamed batch containing the resolved child is checked before the final
listing too. Presenter tests additionally cover incomplete batches without the
child, stale batches, deliberate focus changes and persistent filters.

These are software-window input tests, not native desktop or GPU tests. Detailed,
Brief and Thumbnails layouts share the same reveal mechanism and each has a
parent-return case. Passing these checks does not identify the cause of a jump
reported against a different deployed executable.
