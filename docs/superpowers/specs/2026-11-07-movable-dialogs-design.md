# Movable dialog headers

## Superseding native-window requirement

The later request requires every dialog to be resizable, have a top-right X with
Cancel semantics, and move outside the main window onto another monitor. The
embedded implementation described below cannot satisfy that requirement.
[Current dialog ergonomics](../../developer/DIALOG_ERGONOMICS.md) and the
[latest extension](2026-11-07-panel-viewer-followups.md#dialogs-and-settings)
supersede the bounded-popup decision. Native window hosting is still being
integrated; shared frame/X and resize component tests do not prove that migration.

## Earlier embedded implementation (historical)

The user requests dragging every dialog by its header. Keep the existing popup
and embedded-modal lifetimes, keyboard focus, outside-click policy and callbacks.
Menus, tooltips and native operating-system dialogs retain their existing behavior.

The pinned Slint 1.18.1 runtime updates popup properties in
`WindowInner::update_popup_properties`, but its embedded-popup branch evaluates
the position outside the dependency tracker. Only native popup windows explicitly
subscribe to position changes. Do not depend on an x/y-only binding to move
embedded popups. Keep each existing PopupWindow as a viewport-sized host and move
its inner dialog body. Explicit outside-body dismissal preserves the previous
outside-click policy. This was the earlier implementation choice; the subsequent
native-window requirement intentionally changes its host while preserving the
draft, backend-request and cancellation contracts.

Hosts that previously dismissed on an outside click keep that native policy too,
so clicking outside the application still dismisses them. Modal hosts keep
`no-auto-close`. The pinned compiler constructs a fresh popup component on each
show; popup offsets therefore start centered without observing `is-open` from
inside the popup, which its current lowering pass does not support reliably.

A shared `DialogHeader` translates left-button pointer movement into displacement
without taking keyboard focus. A shared `DialogFrame` centers each dialog
initially and clamps dragged bounds within the current host viewport. Resizing
keeps the title and actions reachable; new dialog sessions reset their offset.
The same header and position contract applies to embedded operation confirmations.
When a constrained resize would clip a scrolling dialog's focused field, the
comparison, filter and Hot Paths editors focus their visible close action. This
keeps Escape usable without changing the draft; header movement retains focus.

Options and Jobs reserve compact header/footer bands and scroll overflowing
content. Options uses a topic picker in narrow layouts. Dialog movement must not
alter draft contents, trigger actions, or accidentally close an open modal.

The attributes editor uses an original document-and-clock SVG in its header and
function-bar action. Its redundant captured-file list is removed. The draft,
Preview changes validation, revision checks and guarded Apply remain intact.
Narrow layouts scroll editor contents and secondary actions while keeping the
close action visible.

At the 320×200 minimum, operation prompts keep Cancel visible and scroll long
confirmation labels horizontally. Application and fallback file menus cap their
popup bounds to the viewport and scroll overflowing actions. Scrollbar clicks
keep these menus open; choosing an action or pressing Escape closes them. Arrow
navigation in the fallback file menu scrolls its selected row into view.

Verification uses actual pointer events, open-popup geometry, resize, keyboard
close, retained draft/focus and existing dialog interaction tests. Generated
evidence remains under `_build/qa`.
