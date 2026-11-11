# Native dialog windows

Date: 10 November 2026. Status: implementation design; not a completion claim.

## Required result

Every application dialog must be an independent, resizable native top-level
window which the user can move outside the main window and onto another monitor.
Its native X and Escape follow the form's Cancel/Close action. Content uses extra
space and remains usable at 320 by 200 logical pixels, including touch density
and large fonts. Moving, resizing, changing theme, or receiving background events
must not reset a draft, submit an operation, or change its captured target.

Ordinary dialogs (Options, confirmations, Help/About and notices) belong to the
main application and must not add separate Windows taskbar buttons. Set their
native owner and suppress the taskbar entry before first show. Keep a normal
resizable title bar and movement outside the owner; use owned top-level windows,
not child windows or the tool-window visual style.

Find, Checksums, Make File List, viewers and comparators are independent working
windows. They retain taskbar and Alt+Tab access, using the same application
identity/icon for system grouping. The user explicitly confirmed this distinction
after noting that a modeless Find window would otherwise be difficult to locate.
Their logical owner still controls application lifetime; native taskbar policy is
separate from modality. Help/About remain ordinary dialogs despite being modeless.

This implements the existing user request and
[dialog ergonomics contract](../../developer/DIALOG_ERGONOMICS.md). It supersedes
the embedded-host portion of the
[earlier movable-dialog design](2026-11-07-movable-dialogs-design.md). No additional
user permission is needed to implement the already requested native windows.

The deliverable includes all forms listed below, not just a shared shell or a
documentation update. Ordinary menus, dropdowns, context menus, tooltips and
in-panel error notices remain their existing kinds of controls.

## Current source evidence

`crates/midnight-ui/ui/dialog-frame.slint::DialogFrame` limits its body to its
owner's width/height and clamps both position coordinates. The owner-sized
`PopupWindow` wrappers in `main.slint` and `navigation-controls.slint` therefore
cannot meet the outside-window requirement. `DialogWindowStyle.native-titlebar`
exists but no current host enables it.

Native examples already exist: `ToolWindow`, `FindWindow`, `ChecksumWindow` and
`FileListWindow`; their Rust hosts are `find_windows.rs`, `checksum_windows.rs`
and `file_list_windows.rs`. Viewer and comparator windows are also independent.
The tool hosts retain backend session IDs, refresh models by revision, use weak
callbacks, and keep native close requests pending when backend cancellation
cannot yet be admitted. `tool_window_failure.rs::Failures` avoids a creation
failure/reopen loop and retries backend release at a bounded rate. These are
reusable patterns, not evidence that all other dialogs have migrated.

`midnight-ui-input::WindowPlacement` applies viewer/comparator geometry before
showing the window. It copies maximized/fullscreen state intentionally for those
windows; ordinary dialogs must instead open at their own preferred size.

## Complete host inventory

All paths in this table are under `crates/midnight-ui/` unless stated otherwise.
The modality choices below are implementation decisions. They preserve
transaction boundaries while keeping running operations independent of the UI.

| Form | Current body/location | New host/adapter | Close policy |
| --- | --- | --- | --- |
| Rename, Delete, create directory/file | `ui/operation-controls.slint`, `main.slint::operation-frame` | `OperationWindow`, `operation_windows.rs` | Cancel exact `OperationId`; owner-modal |
| Copy/Move draft | Same body, `main.slint::copy-draft-frame` | `OperationWindow`, `copy_draft_windows.rs` | Discard exact draft token; owner-modal |
| Copy/Move conflict or failure | `ui/copy-controls.slint` | `DecisionWindow`, `decision_windows.rs` | X/Escape = Cancel job; explicit Later defers; modeless |
| Metadata conflict | Same conflict body | `DecisionWindow`, `decision_windows.rs` | X/Escape = Cancel exact metadata operation; explicit Later defers; modeless |
| Jobs/progress | `ui/jobs-dialog.slint` | `JobsWindow`, `jobs_windows.rs` | Close presentation only; modeless |
| Attributes, Times and Case | `ui/metadata-controls.slint` | `MetadataWindow`, `metadata_windows.rs` | Existing discard/cancel callback; owner-modal |
| Compare directories | `ui/comparison-dialog.slint` | `DirectoryComparisonWindow`, `comparison_windows.rs` | Existing cancel/close operation; owner-modal |
| Panel filter | `ui/panel-filter-dialog.slint` | `PanelFilterWindow`, `panel_filter_windows.rs` | Discard pending form/validation; owner-modal |
| Options, all pages | `ui/options-dialog.slint` | `OptionsWindow`, `options_windows.rs` | Transactional Cancel; owner-modal |
| Color picker | `ui/color-picker.slint` | `ColorPickerWindow`, `color_picker_windows.rs` | Discard unaccepted color; modal to Options |
| Manage Hot Paths | Inline body in `ui/navigation-controls.slint` | Extract `hot-paths-dialog.slint`; `HotPathsWindow`, `hot_path_windows.rs` | Close manager; already committed individual edits remain; owner-modal |
| Help | `ui/help-dialog.slint` | `HelpWindow`, `information_windows.rs` | Close; modeless |
| About | Inline body in `main.slint` | Extract `about-dialog.slint`; `AboutWindow`, `information_windows.rs` | Close; modeless |
| Notices and error message dialogs | `ui/notice-dialog.slint` | `NoticeWindow`, `information_windows.rs` | Close exact notice generation; owner-modal to requester |
| Find, Checksums, Make File List | Existing native hosts | Retain bodies/typed adapters; adopt shared lifetime support | Existing close/cancellation contracts |
| Text/image/Markdown/PDF/HTML viewers, file comparator | Separate viewer/compare crates | Retain native windows and current placement | Existing Escape/X session close |

Each new Slint host has its own `ui/<name>-window.slint` file. `OperationWindow`
and `DecisionWindow` deliberately reuse content for related forms. Do not copy
the form implementation into the host.

## Shared UI-only host contract

Keep backend authority in existing Presenter/service records. Introduce a small
shared host module in `midnight-ui/src/dialog_host.rs`, not a new backend dialog
protocol. Its public crate interfaces are:

```rust
pub(crate) struct DialogSession(u64);
pub(crate) struct DialogSessions { /* checked monotonic counter */ }
pub(crate) enum DialogModality { Modeless, OwnerModal }
pub(crate) enum CloseDisposition { KeepOpen, Hide }
pub(crate) struct DialogOwner { /* weak component access; no native strong Arc */ }
pub(crate) struct DialogHost { /* session, owner, UI lifetime and modal lease */ }

impl DialogOwner {
    pub(crate) fn new<C: slint::ComponentHandle + 'static>(component: &C) -> Self;
    pub(crate) fn with_window<T>(&self, f: impl FnOnce(&slint::Window) -> T) -> Option<T>;
}
impl DialogSessions {
    pub(crate) fn allocate(&mut self) -> Result<DialogSession, String>;
}
impl DialogHost {
    pub(crate) fn create<C: slint::ComponentHandle + 'static>(
        session: DialogSession,
        owner: DialogOwner,
        modality: DialogModality,
        create: impl FnOnce() -> Result<C, slint::PlatformError>,
    ) -> Result<(C, Self), slint::PlatformError>;
    pub(crate) fn show(&self, child: &slint::Window) -> Result<(), slint::PlatformError>;
    pub(crate) fn close(&self, child: &slint::Window);
    pub(crate) fn request_close(
        &self,
        child: &slint::Window,
        semantic_close: impl FnOnce() -> CloseDisposition,
    );
    pub(crate) fn is_current(&self, session: DialogSession) -> bool;
}
```

`DialogSession` is an increasing UI generation, never a filesystem authority.
Each typed adapter retains its existing `OperationId`, draft token, job/request
ID and revision separately. A `DialogHost` owns only presentation state, native
ownership and modal/focus leases. It must not become the last UI owner of a
provider, native filesystem handle, or generated report source.

`create` scopes the native window-attributes hook to this one construction and
clears it on failure/unwind. Creating a child requires an already materialized,
live owner. Do not retain `Arc<winit::window::Window>`: the pinned Slint backend
documents that retaining one can keep a hidden window visible. Borrow it through
`with_winit_window` only during a UI operation.

The caller binds callbacks, installs tooltip/input observation, applies theme,
fills the initial model/draft, and then calls `show`. `show` places the child and
acquires its modal lease only after it can be displayed. Creation/show failure
must release the lease, preserve or explicitly cancel the matching backend
context, and report once to a live owner; never retry creation on every refresh.

Store typed host records by their session identity. A refresh updates a record;
it does not recreate it. Initialize editable fields only when a new session or
explicit draft revision is adopted. Never write the Presenter model back over
text being edited merely because progress or theme changed.

Keep one host per singleton form and at most one decision host for each admitted
job/request; existing backend admission limits bound the latter. Help/About/notice
are singleton records, with a newer notice generation explicitly replacing the
old one. Find/Checksum/File List/viewer limits remain their existing limits. Do
not accumulate hidden host records or native window handles after closure.

Existing `Failures<Id>` remains the backend release retry mechanism where a
close command can be rejected. Its bounded scheduling and deferred refresh after
releasing Presenter borrows must remain intact. Native `on_close_requested`
returns `KeepWindowShown` until the semantic close callback actually accepts
closure; successful adapters hide/remove their record through one common route.

## Owner, modality and focus on each platform

Add a UI-thread native adapter in `dialog_native.rs` with separate
`dialog_native_windows.rs` and `dialog_native_linux.rs`. Platform selection stays
in the module declaration. These are UI window operations, not filesystem work.

Expose `midnight_ui::configure_dialog_window_attributes(WindowAttributes) ->
WindowAttributes` for the executable's existing Slint/Winit creation hook. The
Windows hook must still set `with_drag_and_drop(false)`; the new hook must not
overwrite or re-enable Winit's eager drop target. Renderer selection is unchanged.

- **Windows:** scoped child creation uses Winit's
  `WindowAttributesExtWindows::with_owner_window`. Use an owned top-level, never
  `with_parent_window`/a child control. Owner-modal leases use
  `WindowExtWindows::set_enable(false)` and a counted application lease. Re-enable
  the owner before closing/destroying its last modal child. Nested leases release
  inside-out. Modeless Jobs and decision windows do not disable the main window.
- **Linux/X11:** independent top-level window, dialog type and
  `WM_TRANSIENT_FOR` pointing at the actual owner, using existing `x11-dl` for
  hints and the borrowed native display/window handles. Application modal leases
  gate the owner's controls/shortcuts and restore focus; native modal hints may
  supplement but never replace the application gate. Do not embed into the owner.
- **Linux/Wayland:** the pinned public Winit interface has no general transient
  parent setter and cannot force absolute top-level coordinates. Use independent
  compositor-managed windows and the same application owner/modal/focus graph.
  State this qualification in verification; do not fabricate X11 handles or
  claim native transient parenting. Users can still move/resize the top-level.
- **macOS later:** reserve the same adapter boundary. Do not claim macOS support
  from Windows/Linux tests or place its future Cocoa implementation in their files.

Owners are weak. A modeless window may outlive a focus change, but closing the
application invokes existing backend cancellation/shutdown and closes children
inside-out before releasing the main native owner. No window-local event loop,
blocking modal call, or extra Slint event loop is introduced.

For ordinary dialogs, start at their specified preferred logical size centered
on the live owner where positioning is supported. Clamp initial placement to the
owner monitor's usable bounds, not the main window rectangle; if unavailable,
use the primary monitor. Do not inherit main fullscreen/maximized state. User
movement/resizing is then independent; later refreshes never re-center it.
Viewer/comparator parent-sized placement stays as currently designed.

Focus returns to the immediate live owner, not unconditionally to `focus-files`.
Within that owner, return to the invoking control when still valid, otherwise its
safe default. Restore foreground focus only for an explicit close/activation by
the user and when another child does not own focus. Background completion or an
error must not steal focus from a different application. Store no stale raw
native handle as an independently trusted owner.

The shared `dialog_focus.rs::FocusReturn` captures a weak focus-item reference
from the pinned Slint runtime: `capture(&slint::Window) -> FocusReturn` and
`restore(&self, &slint::Window) -> bool`. It restores only a still-live, visible,
enabled item belonging to that owner. Typed adapters supply their default focus
action if restoration returns false. Keep runtime-dependent access in this one
module, with focused tests; no strong item reference may keep a closed owner alive.

## Content, keyboard and theme behavior

`DialogWindow` extends the shared native `ToolWindow`, sets a real title/icon,
minimum 320 by 200, and enables `DialogWindowStyle.native-titlebar` in its own
component context. Existing content fills the client area; remove outer embedded
frame margins, dimming, fake resize edges and duplicate titlebar/X. The native
titlebar supplies dragging, resizing and X. Keep meaningful warning icons/summary
inside the body even when its duplicated header is suppressed.

Escape must first reach the content. A shell-level capture handler must not close
Options while the keyboard editor consumes Escape, while a command drag is being
cancelled, or while the nested color picker is active. Use the form's single
`request-dismiss` route as the final unhandled-Escape fallback. Native X invokes
that same semantic route, subject to the save lock; key-capture Escape can still
finish key capture first. Preserve native IME/AltGr/physical shortcut handling in
the new Options window; do not dispatch its keys to file-panel commands.

For operation confirmations, Enter remains limited to the input/option/default
action. Focused Cancel/X never confirms deletion. Opening/focusing is inert.
For conflicts, fix today's mismatch: X and Escape both cancel the exact job;
Later is a distinct button that leaves the job waiting and closes only its
decision window. The backend decision/revision token is always checked.

Each native component has separate Slint globals. Apply `Theme`, `Palette`,
shortcut hints, icons and tooltip placement to each live host before first show
and when their corresponding revision changes. Refactor Options global bindings
onto `OptionsWindow`; merely moving `OptionsBody` leaves callbacks on the wrong
component. Reuse helpers/model adapters instead of duplicating the binding code.
Theme preview and rollback propagate to all live windows. Geometry, selected
rows, scroll offsets, draft text, caret and selection do not reset on theme change.

The subsequent complete-UI-polish request supersedes fixed large initial sizes.
Ordinary operations and Copy/Move choose height from their wrapped summaries,
fields and actions; long content scrolls. Options starts at 740x520 logical
pixels and Jobs at 720x500, with font-relative sizing. Hot Paths sizes its list to
saved rows; short notices use compact content-based heights. Native migration
must carry these updated presentation rules forward rather than restoring the
earlier large dimensions. Clamp initial placement/size to the monitor work area.
Every form must still resize down to the 320x200 minimum without growing that
minimum to its preferred dimensions.

## Transaction and backend invariants

- **Options:** `begin_options` captures one baseline; all pages participate in
  one transaction. OK validates then saves; only successful save closes. Cancel
  rolls back live previews, including history/theme. Validation is cancellable.
  An admitted save disables Cancel and rejects native X until success/failure.
  A failed save leaves the full draft and an actionable error visible.
- **Nested Options UI:** color selection updates only the requesting color field
  and generation. Color Cancel leaves the prior value. Executable/icon/file
  pickers receive the Options native owner, not MainWindow. Existing raw-path and
  stale page/item/revision checks remain. A custom-command preview opens/focuses
  Jobs as a modeless child of Options; closing it restores Options without saving
  or discarding its draft.
- **Jobs:** closing presentation never issues Cancel job. Automatically opened
  progress may close after its successful job; manually opened Jobs stays open.
  A decision window may coexist with Jobs; do not hide/destroy and later rebuild
  Jobs just to show a prompt. Retain selection, speed-chart cache and scroll state.
- **Captured context:** operation/copy/metadata requests keep their existing
  captured identities. A native window callback checks both its UI generation
  and backend token. It cannot reinterpret the currently active panel as its
  source. Panel Filter retains its pane/navigation context. Compare directories
  retains its two captured request contexts. Hot Path actions retain bookmark IDs
  and the captured typed native/archive location for Add path, not display text.
- **Ownership:** rejected/stale/closed result owners use the existing backend
  retirement mechanism. No native dialog migration adds UI filesystem access,
  relaxes `..` protection, follows links, or exports virtual display paths.

## Panel Filter and Hot Paths implementation checkpoint

11 November 2026: Panel Filter and Hot Paths now use separate resizable,
owner-modal native hosts. Filter retains its captured pane/navigation or tree
session and a per-opening generation. Refresh and resize preserve edited text;
Cancel invalidates pending validation, and Apply/Show all reject stale captures.
The compact first-open path focuses a visible footer control so Escape works at
320 by 200 with font 32.

Hot Paths captures a typed native/archive/Network location when the manager opens.
Add retains that location across subsequent navigation; displayed path text never
becomes authority. Existing bookmark IDs, deduplication, numbered-slot uniqueness
and slot swaps remain in force. Edits commit immediately through the existing
presenter/settings path; closing the manager does not roll them back. Old window
callbacks cannot mutate or close a replacement session.

Windows has **22 focused passing tests**: ten native-window integration cases,
three affected commander/bookmark cases, two stale-window callback cases, two
captured-bookmark cases and five filter presenter cases. Linux passes the same
ten native Filter/Hot Paths integration cases with the software test platform.
These checks establish source behavior, including compact first-open Escape,
draft preservation, stale callbacks and injected host-creation failure recovery.
The latter requires a visible owned Notice after ordinary refresh, no backend
mutation and explicit reopening; Filter also retires its failed draft while
preserving the applied filter. Desktop movement, native focus placement and
cross-monitor behavior remain separate; the current Windows debug build and
startup qualification are recorded with the evidence below. Jobs/decision qualification is recorded separately and remains outside
this checkpoint. See the [current evidence and remaining checks](../../verification/2026-11-11-native-tool-windows.md).
The overall inventory and completion boundary below remain open.

## Verification and completion boundary

Adapt the software-native `tests/support/modeless.rs` harness to distinguish
actual window adapters, native close requests and visibility. Exercise every
inventory row through its real installed callbacks. Keep body-layout tests but
replace embedded-popup assertions for migrated forms. `dialog_drag.rs` and
`dialog_resize.rs` alone are not native-window evidence.

Automated coverage must include: separate adapter; resize without draft loss;
minimum/large-font layout; Escape/X/Cancel; focused-close Enter; nested ownership;
full command queue; stale request; save lock/failure; native creation/show failure;
owner closure; theme preview/rollback; late backend result retirement; and no
hidden-window leak after repeated open/close cycles. Tooltip coordinates must
belong to the new host, including monitor-scale changes.

Run actual desktop probes on Windows and Linux/X11 and Wayland where available:
drag beyond main bounds, resize via native decorations, place on a second monitor,
change DPI, minimize/restore parent, close nested windows, and move focus to an
unrelated app during completion. Software tests do not establish compositor or
native ownership behavior. Log unavailable environments honestly. Use isolated
preferences/owned fixtures; screenshots, logs and scripts belong under
`_build/qa/native-dialogs/`.

No new dependency or license change is planned. Respect the pinned Slint/Winit
APIs, separate platform source files, 100-column Rust formatting, and the 100 KB
handwritten-file limit. Integration is complete only when every inventory row is
native and the relevant behavioral/desktop checks pass; staged commits are
review boundaries, not permission to stop after the first migrated form.
