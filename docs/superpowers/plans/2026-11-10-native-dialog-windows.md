# Native Dialog Windows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move every remaining application dialog into a resizable native window
with correct ownership, focus, cancellation, settings transactions and captured
backend context.

**Architecture:** Reuse existing Slint form bodies and typed Presenter requests.
A small shared UI host owns native placement/modal/focus lifetime; typed adapters
own per-form callbacks and refresh. Existing Find/Checksum/File List/viewer hosts
provide patterns and retain their backend ownership contracts.

**Tech Stack:** Existing Rust, Slint 1.18.1/Winit 0.30.13, existing x11-dl and
Windows bindings; Tokio/backend services unchanged. No new dependency planned.

**Spec:** [Native dialog windows](../specs/2026-11-10-native-dialog-windows.md).

**Taskbar clarification (accepted during implementation):** ordinary dialogs use
owned native windows with no separate taskbar entry. Independent working tools
(Find, Checksums, Make File List, viewers and comparators) retain taskbar/Alt+Tab
access. Keep this explicit role separate from modality: Help/About are modeless
but still ordinary dialogs. `DialogHost::create_tool` adopts existing tools
without removing their taskbar entry. The Windows desktop probe must check both
roles, rather than treating every modeless window as a hidden taskbar dialog.

## Global Constraints

- Every inventory form becomes an independent native top-level; ordinary menus,
  dropdowns, context menus, tooltips and in-panel notices remain controls.
- Native minimum size is 320 by 200 logical pixels; content grows/scrolls.
- X/Escape use Cancel/Close. Closing Jobs never cancels a job. Conflict Later
  remains a separate action. Options admitted-save dismissal remains blocked.
- No new UI filesystem/backend work; typed identity and off-UI retirement remain.
- Windows/Linux implementations use separate files; Wayland placement/parenting
  limits are reported honestly. macOS implementation is a later platform task.
- Rustfmt width 100; handwritten files at most 100 KB; no new dependency planned.
- Use existing `_build` and `_build/linux` targets. Artifacts/scripts/logs belong
  under `_build/qa/native-dialogs/`; no cold target directory or release iteration.
- Current workspace contains unrelated uncommitted work. Do not revert or commit
  it accidentally. Coordinate shared source/build ownership with the parent agent.

## Review Focus

1. A queued native picker returns after Options changes page or closes: it must
   neither mutate a newer draft nor own the wrong native window (Task 5).
2. An error/decision appears while the user is in another app: no unsolicited
   focus theft, and eventual close returns to the right live owner (Tasks 1, 3).
3. Native close arrives during admitted Options save or full cancellation queue:
   retain the form and its backend owner until semantic close is accepted (Tasks 1, 5).
4. Navigation changes between dialog open and submit: never reinterpret a new
   active panel/location as the original request (Tasks 2, 4).
5. Owner shutdown, creation failure or nested modal leases leave a disabled owner
   or invisible window: release inside-out with no UI final provider destruction
   and no retry/reopen loop (Tasks 1, 6).

## File structure and integration ownership

New shared files in `crates/midnight-ui/src/`: `dialog_host.rs`,
`dialog_host_tests.rs`, `dialog_native.rs`, `dialog_native_windows.rs`,
`dialog_native_linux.rs`, `dialog_focus.rs`, and `dialog_presentation.rs`. New Slint base:
`crates/midnight-ui/ui/dialog-window.slint`.

Typed host adapters and Slint wrappers use the exact names in the spec inventory.
`information_windows.rs` shares Help/About/Notice lifetime plumbing; each form
still has a separate small Slint wrapper. Extract `hot-paths-dialog.slint` and
`about-dialog.slint` from their current inline definitions.

`src/lib.rs`, `ui/main.slint`, `tooltip.rs`, native input attachment and executable
window-hook integration have one integration owner. Separate form adapters may
be implemented in parallel only after Task 1 interfaces are stable. Coordinate
edits to those shared files; do not have workers independently regenerate or
restructure the entire UI graph.

The existing `midnight-ui-bindings/build.rs` compiles `main.slint`; import/export
new window components through that graph rather than introducing another full
Slint build. Preserve its tooltip lowering/resource tracking. If a source file
approaches 100 KB, move the typed host code to its listed module rather than
inlining it into `lib.rs` or `main.slint`.

## Verification commands

Run commands with the repository's existing toolchain environment and pinned SDK
registry configuration, if required by that checkout. Add `--locked --offline`
to the Cargo commands below. On Windows serialize Cargo use of `_build`; Linux
uses the existing `_build/qa/interaction/linux-verification-env.sh` environment
and `_build/linux`. Capture output under this feature's QA directory.

For a task's RED, first write tests against the currently registered/public UI
behavior so failure proves the missing native host or semantic behavior rather
than an incomplete new API. After production changes, compile once and run the
focused targets together. Do not restart quiet Slint generation without evidence
that it stalled.

## Task 1: Native host foundation and informational windows

**Checkpoint evidence:** Task 1 implementation and Windows foundation qualification
are recorded in [dialog polish](../../verification/2026-11-10-ui-dialog-polish.md).
Lifecycle cases live in `src/dialog_host_tests.rs` and the informational-window
integration target rather than a separate `native_dialog_lifecycle` target.
Task 2 is committed through `5aee4d2`; its behavioral REDs, host lifecycle and
Windows qualification are in [operation windows](../../verification/2026-11-11-native-operation-windows.md),
with final captures and Linux limits in the [queue follow-up](../../verification/2026-11-11-queue-followup.md).
The checked items record those implemented/verified slices, not completion of
Tasks 3–6 or X11/Wayland desktop qualification.

**Files:**
- Create: shared Rust/native/Slint files listed above; `information_windows.rs`;
  `ui/help-window.slint`, `ui/about-dialog.slint`, `ui/about-window.slint`,
  `ui/notice-window.slint`; `tests/native_information_windows.rs`;
  `tests/native_dialog_lifecycle.rs`.
- Modify: `src/lib.rs`, `src/tool_window_failure.rs`, `src/tooltip.rs`,
  `ui/main.slint`, `ui/tool-window.slint`, `ui/dialog-frame.slint`,
  `ui/help-dialog.slint`, `ui/notice-dialog.slint`;
  `crates/midnight-explorer/src/window_backend_windows.rs` and
  `window_backend_linux.rs`; `tests/support/modeless.rs`.

**Interfaces:**
- Produce the exact `DialogSession`, `DialogSessions`, `DialogOwner`, `DialogModality`,
  `CloseDisposition` and `DialogHost` interfaces in the spec.
- Export `configure_dialog_window_attributes(WindowAttributes) -> WindowAttributes`
  from midnight-ui. Executable hooks compose it with their existing policy.
- `dialog_presentation::apply<C: DialogThemeTarget>(&C, &Presenter)` updates
  theme/font/palette by revision. The local `DialogThemeTarget` trait has
  `fn apply_theme(&self, scheme: i32, density: i32, family: SharedString,
  desktop: i32, touch: i32)`; typed adapters implement it by forwarding to their
  generated `apply-preferences` function. No backend work occurs in this trait.
- `information_windows::install(&MainWindow, Rc<RefCell<Presenter>>,
  Rc<dyn Fn(Command) -> bool>)`; register open/refresh/shutdown callbacks once.
- New wrappers forward content `close` to one `request-dismiss` callback. Native
  X and final unhandled Escape invoke that callback; content handles Escape first.
- `dialog_focus::FocusReturn::capture(&slint::Window) -> FocusReturn` and
  `restore(&self, &slint::Window) -> bool` preserve weak invoking-control focus;
  typed adapters provide the fallback focus action. `DialogSessions::allocate`
  provides checked generations; failed allocation does not open an ambiguous host.

- [x] Add `information_dialogs_get_independent_window_adapters` using installed
  Help/About/notice actions. Assert a second visible adapter and no main popup.
  Add `native_close_and_escape_use_one_close_route`, `show_failure_releases_modal_owner`,
  and `nested_modal_lease_restores_only_after_last_child` with simulated native
  close/show failure. Run `cargo test -p midnight-ui --test native_information_windows
  --test native_dialog_lifecycle`; observe behavioral RED against embedded hosts.
- [x] Implement UI-only host records, weak owners, counted modal leases and
  native attributes scope. Preserve Windows `with_drag_and_drop(false)`. Add
  Windows owner/enable and Linux X11 transient/type handling in separate files;
  Wayland uses application owner graph and compositor placement. Do not hold a
  strong Winit window Arc or use nested event loops.
- [x] Implement pre-show theme/model/callback initialization, owner-monitor
  placement, native titlebar/header suppression, tooltip attachment and
  close/failure cleanup. Migrate Help/About/Notice; keep their content and full
  diagnostic-copy behavior. Add notices' generation checks and owner fallback.
- [x] Extend the lifecycle test with background completion while another window
  is active, explicit-close focus return, expired owner, repeated open/close, and
  failed backend-release admission. Assert no hidden live host/disabled owner,
  bounded retry, and no focus theft. Verify `cargo test -p midnight-ui --lib
  dialog_host` plus both new integration targets and `tool_window_failures`.
- [x] Review/commit only this task's coherent changes when the integration owner
  approves the checkpoint; do not stage unrelated workspace files.

## Task 2: Operation confirmations and Copy/Move drafts

**Files:**
- Create: `src/operation_windows.rs`, `src/copy_draft_windows.rs`,
  `ui/operation-window.slint`, `tests/native_operation_windows.rs`.
- Modify: `src/lib.rs`, `src/copy_presentation.rs`, `src/presenter/operations.rs`,
  `src/presenter/copy_dialog.rs` only where host lifecycle needs it;
  `ui/main.slint`, `ui/operation-controls.slint`;
  `tests/operation_defaults.rs`, `tests/copy_dialog.rs`, `tests/dialog_layout.rs`.

**Interfaces:**
- Each adapter exposes `install(&MainWindow, Rc<RefCell<Presenter>>,
  Rc<dyn Fn(Command) -> bool>)` and uses Task 1 `DialogHost`.
- Operation records key by existing `OperationId`; copy-draft records key by
  existing draft token. `DialogSession` distinguishes reopened UI occurrences.
- Reuse `OperationDialog::begin`, confirm/cancel and focused-input behavior.
  Native window creation/refresh never changes its captured backend target.

- [x] Add `native_operation_resize_retains_input_and_target` for rename/create
  directory/create file, and `native_copy_draft_retains_source_after_other_panel_activation`
  for F5/F6. Assert a separate window, preserved text/caret and unchanged exact
  request token; simulate a late callback from a closed occurrence. Run
  `cargo test -p midnight-ui --test native_operation_windows` for RED.
- [x] Implement the two adapters using the shared `OperationWindow`. Migrate
  source summary, icons, metadata option and default focus. Retain the existing
  full command-queue and cancellation semantics; remove only the corresponding
  embedded host definitions and MainWindow form-focusing assumptions.
- [x] Move default-action regressions to actual child windows: Return in editor
  submits once; empty value does not; focused Cancel/native-X equivalent never
  confirms Delete; Escape cancels; opening/focusing never submits. Add queue-full
  retry and exact raw Unicode path assertions. Run `native_operation_windows`,
  `operation_defaults`, `copy_dialog`, `dialog_layout` and presenter operation tests.
- [x] Review/commit this slice after the integration owner checks native host
  failure releases its prepared operation instead of leaving invisible ownership.

## Task 3: Jobs and operation decisions

**Files:**
- Create: `src/jobs_windows.rs`, `src/decision_windows.rs`, `ui/jobs-window.slint`,
  `ui/decision-window.slint`, `tests/native_job_windows.rs`.
- Modify: `src/lib.rs`, `src/job_presentation.rs`, `src/metadata_presentation.rs`,
  `src/tooltip.rs`, `ui/main.slint`, `ui/copy-controls.slint`;
  `tests/jobs_dialog.rs`, `tests/copy_errors.rs`, `tests/list_scroll.rs`,
  `tests/tool_reflow.rs`, and the direct native-operation transfer fixture.
- Add native desktop qualification: `src/native_jobs_desktop_tests.rs` and the
  existing Windows test module registration. Existing backend decision policies
  and the Jobs body remain reused.

**Interfaces:**
- Both adapters use Task 1 `install(...)` signature. Jobs keeps one presentation
  record and existing selected job/auto-open token; decisions key by exact
  Copy/Move/metadata conflict identity plus revision. Do not replace backend IDs
  with row indexes or text.
- `JobsWindow::request-dismiss` closes presentation. `DecisionWindow::request-dismiss`
  maps to existing `decide(3, "")`; `later` is a separate callback.

- [x] Add RED assertions that Jobs creates another adapter while the main panel
  stays usable; native close sends no Cancel job; both conflict Escape and X send
  the exact cancel decision; Later sends only defer. Assert manual Jobs remains
  after completion and automatic successful progress closes. Run
  `cargo test -p midnight-ui --test native_job_windows`.
- [x] Implement modeless Jobs/decision hosts. Keep Jobs visible behind decisions
  instead of popup hide/rebuild. Preserve selected job, scroll and speed-chart
  cache. Cancel job remains an explicit button. Exact decision tokens reject
  stale prompts; full admission queues leave the decision actionable.
- [x] Qualify modeless ownership and background decision activation through the
  actual Windows Jobs/Decision adapter probe, plus installed software lifecycle
  cases. Cover existing-error Retry/Skip/Cancel and metadata-decision routing.
  Run `native_job_windows`, `jobs_dialog`, `copy_errors`, and
  `cargo test -p midnight-ui --lib job_speed`.
- [x] Independently review exact identity, admission failure, deferred notices,
  and closure; verify that operation cancellation, verification, cleanup and
  terminal-job retention policies remain unchanged.
- [x] Parent integrates/commits the completed slice with the coordinated batch.

**Evidence:** [Task3 verification](../../verification/2026-11-11-native-jobs-decisions.md)
records 34 targeted Windows tests, 18 Linux Jobs/error tests, and one passing
interactive Windows probe. The native probe is named
`native_jobs_close_preserves_work_and_background_decision_preserves_foreground`.
It verifies actual HWND ownership/taskbar styles, enabled modeless owner,
non-cancelling Jobs close, preserved independent foreground on decision show and
close, and exact `Cancel 1:9`. Windows may retain an already independent foreground
window when activation of the test window is refused; this is not a claim about
any particular other application. The actual native-boundary probe replaces the
proposed separate mocked activation-policy check under the user's targeted-test
priority. Broad final suites remain deferred until the implementation queue is
complete; the slice has not yet been committed by its implementer.

## Task 4: Metadata, comparison, filter and Hot Paths

**Files:**
- Create: `src/metadata_windows.rs`, `src/comparison_windows.rs`,
  `src/panel_filter_windows.rs`, `src/hot_path_windows.rs`;
  `ui/metadata-window.slint`, `ui/directory-comparison-window.slint`,
  `ui/panel-filter-window.slint`, `ui/hot-paths-dialog.slint`,
  `ui/hot-paths-window.slint`; `tests/native_panel_dialogs.rs`.
- Modify: `src/lib.rs`, `src/metadata_presentation.rs`, `src/metadata_input.rs`,
  `src/navigation_models.rs`, `src/presenter/metadata.rs`,
  `src/presenter/comparison.rs`, `src/presenter/bookmarks.rs`,
  `src/presenter/filter.rs`; `ui/main.slint`, `ui/navigation-controls.slint`,
  `ui/metadata-controls.slint`, `ui/comparison-dialog.slint`,
  `ui/panel-filter-dialog.slint`; `tests/commander_dialogs.rs`.

**Interfaces:**
- Each adapter has Task 1 `install(...)` signature. Preserve metadata
  `OperationId`/revision and comparison request IDs; filter retains pane and
  navigation; Hot Paths retains bookmark identity and typed captured Add location.
- Extract existing Hot Paths content into a body with explicit properties and
  callbacks matching its current controls. Open requests from either drive/menu
  route target one manager occurrence; they do not create separate pane popups.

- [ ] Add RED `panel_forms_are_native_and_do_not_retarget_after_navigation`
  table cases for all four forms. Include archive Hot Path Add and unavailable
  metadata capabilities; verify close/reopen loses uncommitted form text only
  according to that form's existing semantics. Run
  `cargo test -p midnight-ui --test native_panel_dialogs`.
- [ ] Implement typed adapters and native wrappers. Retain single shared Include
  subfolders, preview/revision validation, filter Show all, comparison selection
  publication, numbered/unique Hot Paths and immediate committed bookmark edits.
  Remove their embedded hosts and per-panel Hot Paths popup state.
- [ ] Add stale validation/late cancel tests, 320x200/touch-font layout, editing
  while resizing and exact source-context assertions. Run `native_panel_dialogs`,
  `commander_dialogs`, metadata/comparison/filter/bookmark presenter tests.
- [ ] Review/commit only after the callback path still reaches the existing
  capability checks and never derives paths from the displayed Slint row text.

## Task 5: Transactional Options and owned nested windows

**Files:**
- Create: `src/options_windows.rs`, `src/color_picker_windows.rs`,
  `ui/options-window.slint`, `ui/color-picker-window.slint`,
  `tests/native_options_windows.rs`.
- Modify: `src/lib.rs`, `src/color_options.rs`, `src/archive_options.rs`,
  `src/plugin_options.rs`, `src/editor_options_presentation.rs`,
  `src/user_command_settings_presentation.rs`, `src/settings_picker_presentation.rs`,
  `src/shortcut_editor.rs`, `src/native_shortcuts.rs`, `src/shortcut_bridge.rs`,
  `src/picker_window.rs`, `src/presenter/preferences/options.rs` only as needed
  for session identity; `ui/main.slint`, `ui/options-dialog.slint`,
  `ui/color-picker.slint`; existing Options/keyboard/color-picker tests.

**Interfaces:**
- `options_windows::install(...)` owns the single Options host and form-global
  bindings. Move relevant `refresh(&MainWindow, &Presenter)` and `install` form
  adapters to `&OptionsWindow` (or shared local host trait where also required by
  test fixtures); no duplicate main/child draft state.
- `color_picker_windows::install(...)` uses `DialogOwner::new(&OptionsWindow)`
  and the current color field/generation. Pickers use
  `native_path_picker_owner(options.window())`.
- Options calls existing `begin_options`, `cancel_options`, `accept_options`,
  `flush_options_acceptance` and `options_save_result`; no second settings
  transaction or persistence mechanism is added.
- Native keyboard events in Options feed a window-local shared
  `midnight_ui_input::ShortcutState` and that host's `KeyboardEditorView`.
  Main-panel dispatch does not consume child edits or physical shortcut capture.

- [ ] Add RED `options_are_native_and_cancel_restores_all_previews`,
  `native_x_during_admitted_save_keeps_options_visible`, and
  `options_picker_and_preview_return_to_the_live_options_owner`. Verify existing
  installed callbacks, not an isolated empty window. Run
  `cargo test -p midnight-ui --test native_options_windows`.
- [ ] Implement Options host and move every form/global binding into its component
  context. Apply theme/shortcut hints before show and refresh all live hosts on
  preview/rollback. Preserve topic, scroll, drafts, keyboard capture and drag
  reorder during ordinary Presenter refresh. Remove main-owned Options popup.
- [ ] Implement native color picker with field/generation correlation and modal
  lease to Options. Move picker ownership to the actual form. Open custom-command
  preview Jobs as a modeless child of Options without closing/reinitializing it.
  Native X shares Cancel while respecting admitted-save lock; first Escape can
  still end shortcut capture/drag/nested picker before closing Options.
- [ ] Add picker result after page/item/generation change; reset then Cancel;
  failed save then retry; full queue; IME/AltGr; Ctrl/Shift/digit capture; color
  cancel/accept; nested X; theme rollback across other live windows; large font
  320x200 footer. Run `native_options_windows`, `options_transaction`,
  `options_footer`, `keyboard_options`, `archive_options`, `color_picker_popup`,
  existing settings picker and user-command form tests.
- [ ] Review/commit with explicit verification that child file pickers receive
  the Options native handle and no save/cancel path silently drops a live draft.

## Task 6: Existing native tools, complete inventory and desktop qualification

**Files:**
- Modify: `src/find_windows.rs`, `src/checksum_windows.rs`,
  `src/file_list_windows.rs`, `src/tool_window_failure.rs`, `src/tooltip.rs`,
  `src/lib.rs`; `tests/find_windows.rs`, `tests/checksum_windows.rs`,
  `tests/file_list_window.rs`, `tests/tool_window_failures.rs`,
  `tests/dialog_drag.rs`, `tests/dialog_resize.rs`, `tests/dialog_layout.rs`.
- Create: `tests/native_dialog_inventory.rs`,
  `crates/midnight-ui/examples/native_dialog_probe.rs`,
  `crates/midnight-ui/examples/native_dialog_probe/windows.rs`,
  `crates/midnight-ui/examples/native_dialog_probe/linux.rs`,
  `docs/verification/2026-11-10-native-dialog-windows.md`.
- Review existing native viewer/comparator hosts in
  `crates/midnight-viewer-ui/src/lib.rs`, `crates/midnight-compare-ui/src/lib.rs`
  and `midnight-ui-input/src/placement.rs`; only change if qualification finds
  an actual ownership/focus defect, preserving their backend/session contracts.

**Interfaces:**
- Existing tools adopt Task 1 native owner/focus/theme lifetime while retaining
  their existing session maps, model revisions, closure/backpressure and
  retirement callbacks. Viewer/comparator placement remains parent-sized.
- Probe takes an isolated preferences/fixture directory and a form name; creates
  harmless forms and emits structured window IDs/bounds/visibility/focus events.
  Destructive-operation prompts use owned throwaway fixtures and never confirm.

- [ ] Add `every_dialog_inventory_entry_uses_a_distinct_native_host` driving all
  spec rows; assert menus remain popups. Add old-native-host initial-theme and
  failed-close ownership checks. Run focused RED for the missing adoption cases.
- [ ] Adopt shared host policy for Find/Checksum/File List and ensure app shutdown
  closes children inside-out. Remove obsolete embedded dialog hosts, offsets and
  dead main-dialog routing. Update migrated tests to inspect child adapters;
  retain `DialogFrame` only where a true menu/popup still uses it.
- [ ] Run focused native targets plus full `cargo test -p midnight-ui --lib` and
  the existing integration suite. Run `cargo clippy -p midnight-ui
  -p midnight-ui-input -p midnight-explorer --all-targets -- -D warnings`,
  `cargo fmt --all --check`, the platform's `scripts/check-source-size` script,
  and `git diff --check`. Repeat the qualified Linux equivalents on its existing
  target; do not label unrun desktop backends as tested.
- [ ] Run `native_dialog_probe` on Windows desktop account and Linux X11/Wayland
  when available. Record each inventory entry moved outside main bounds; native
  resize to 320x200/larger; second-monitor/DPI movement; X/Escape; nested
  Options→picker/Jobs; owner minimize/restore; external-app focus; shutdown; no
  visible ghost windows. Store probe artifacts only in `_build/qa/native-dialogs/`.
- [ ] Update verification and the existing ergonomics/movable-dialog status with
  exact passed counts, native environments and remaining platform qualifications.
  Perform independent review of lifetime, stale authority and close semantics.
  Build/deploy debug only after gates; verify the deployed artifact's native
  dialogs. Commit relevant sources/docs through the integration owner.

## Execution and completion

Task 1 defines shared interfaces. Tasks 2–4 may be parallelized by typed adapter
after that checkpoint; the integration owner serializes `main.slint`, `lib.rs`,
tooltip and executable hook edits. Task 5 follows stable ownership/focus support
and Jobs hosting. Task 6 qualifies the complete inventory, not just representative
forms. Retain one Cargo owner per target directory.

All boxes begin unchecked: this is a plan, not evidence. No task is complete from
a documentation update, a detached mock window, or embedded-frame movement alone.
The user's existing request authorizes implementation; do not insert a redundant
permission question between these stages.
