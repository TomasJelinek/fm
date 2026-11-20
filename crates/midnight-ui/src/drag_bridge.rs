//! UI input and geometry only; native payloads and filesystem work stay on workers.
use crate::{
    BeginOwned, MainWindow, Presenter,
    drag_lifecycle::{GestureSnapshot, Phase},
    pane,
    presenter::drag_drop::{MIN_DRAG_DURATION, invalidate},
};
use i_slint_core::window::{WindowInner, WindowKind};
use midnight_core::*;
use slint::{
    ComponentHandle,
    platform::{PointerEventButton, WindowEvent},
    winit_030::WinitWindowAccessor,
};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::Duration,
};

#[path = "drag_input_native.rs"]
mod native;

/// Cadence while a gesture, cancellation, shutdown, native hand-off, external hover
/// or refused drag command is pending: pointer feedback and retries stay prompt.
const ACTIVE_POLL: Duration = Duration::from_millis(16);
/// Safety net otherwise. Refreshes, invalidations and native releases, wheel and touch
/// input republish on the next loop turn. A region change without any of those reaches
/// the published targets only here, up to this interval late, in either direction: a
/// region that became droppable (a scroll animation ended) can refuse a drop meant for
/// it, and a region that should be withdrawn (a popup opened by a timer, such as a
/// long-press context menu) can still accept one.
const IDLE_POLL: Duration = Duration::from_millis(250);
/// How long the source panel's status line keeps the note that a drag was released
/// too soon (`MIN_DRAG_DURATION`); the next drag removes it at once. It goes on the
/// next poll after this time, up to `IDLE_POLL` later.
const QUICK_NOTE: Duration = Duration::from_secs(4);

thread_local! {
    /// This UI thread's bridges, each found through its own window.
    static BRIDGES: RefCell<Vec<Weak<Bridge>>> = const { RefCell::new(Vec::new()) };
}

type DragVisual = (
    Option<Phase>,
    usize,
    Option<PaneId>,
    i32,
    InternalDropIntent,
    slint::SharedString,
    i32,
);

struct Target {
    destination: InternalDropDestination,
    pane: PaneId,
    row: i32,
    name: slint::SharedString,
}

struct Bridge {
    window: slint::Weak<MainWindow>,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
    begin_owned: BeginOwned,
    targets: SharedDropTargets,
    cursor: Cell<slint::LogicalPosition>,
    modifiers: Cell<(bool, bool, bool)>,
    handoff: Cell<bool>,
    gesture: GestureSnapshot,
    cancel_pending: Cell<Option<DragId>>,
    shutdown_pending: Cell<bool>,
    visual: RefCell<Option<DragVisual>>,
    lifetime: RefCell<Option<(u64, Cancellation)>>,
    timer: slint::Timer,
    /// One zero-delay turn is queued; later requests in the same loop turn share it.
    turn_scheduled: Cell<bool>,
    /// The last publication could not run or a drag command was refused.
    retry: Cell<bool>,
    /// When the primary button went down in this window, while it is held (`observe`).
    press: Cell<Option<Duration>>,
    /// When the press that started the current gesture went down: drags are timed from it.
    gesture_press: Cell<Option<Duration>>,
    /// When a source panel began to note a drag released too soon, until it retires.
    quick_note: Cell<Option<Duration>>,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        invalidate(&self.targets);
        if let Some((generation, lifetime)) = self.lifetime.get_mut().take() {
            lifetime.cancel();
            (self.send)(Command::DetachDragDrop {
                host_generation: generation,
            });
        }
    }
}

/// The platform's monotonic clock; tests inject theirs through the Slint platform.
fn now(window: &MainWindow) -> Duration {
    WindowInner::from_pub(window.window())
        .context()
        .platform()
        .duration_since_start()
}

fn host(window: &MainWindow) -> Option<NativeDragWindow> {
    let mut host = None;
    window
        .window()
        .with_winit_window(|window| host = Some(native::host(window)));
    host
}

fn rectangles(window: &MainWindow) -> [Option<DropRect>; 2] {
    if !window.window().is_visible()
        || window.get_dialog_input_blocked()
        || window.get_embedded_modal_open()
        || !window.get_copy_draft_token().is_empty()
        || !window.get_operation_token().is_empty()
        || WindowInner::from_pub(window.window())
            .active_popups()
            .iter()
            .any(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        || (window.get_scroll_interaction_active() && !window.get_drag_active())
    {
        return [None, None];
    }
    let bounds = [
        (
            window.get_left_list_left(),
            window.get_left_list_top(),
            window.get_left_list_width(),
            window.get_left_list_height(),
        ),
        (
            window.get_right_list_left(),
            window.get_right_list_top(),
            window.get_right_list_width(),
            window.get_right_list_height(),
        ),
    ];
    let scale = window.window().scale_factor();
    let size = window.window().size();
    // Search results fed to a panel cover its list: nothing is dropped into the
    // folder under them (the presenter refuses such a target as well).
    let feeds = window.global::<crate::FeedPresentation>();
    let fed = [feeds.get_left().open, feeds.get_right().open];
    std::array::from_fn(|pane| {
        if fed[pane] || (window.get_panel_zoomed() && window.get_active_pane() != pane as i32) {
            return None;
        }
        let (x, y, width, height) = bounds[pane];
        if ![x, y, width, height, scale].into_iter().all(f32::is_finite) {
            return None;
        }
        let left = (x * scale).ceil().clamp(0.0, size.width as f32) as i32;
        let top = (y * scale).ceil().clamp(0.0, size.height as f32) as i32;
        let right = ((x + width) * scale).floor().clamp(0.0, size.width as f32) as i32;
        let bottom = ((y + height) * scale)
            .floor()
            .clamp(0.0, size.height as f32) as i32;
        (right > left && bottom > top).then_some(DropRect {
            x: left,
            y: top,
            width: (right - left).max(0) as u32,
            height: (bottom - top).max(0) as u32,
        })
    })
}

impl Bridge {
    fn remember_host(&self, p: &Presenter) {
        if let Some(host) = &p.drag_drop.host
            && self
                .lifetime
                .borrow()
                .as_ref()
                .is_none_or(|(id, _)| *id != host.generation)
        {
            *self.lifetime.borrow_mut() = Some((host.generation, host.lifetime.clone()));
        }
    }
    /// A refused drag command stays queued in Presenter; retry it at the quick cadence.
    fn observe(&self, sent: bool) -> bool {
        if !sent {
            self.retry.set(true);
        }
        sent
    }

    /// How long ago the press that started the current gesture went down.
    fn held(&self, window: &MainWindow) -> Duration {
        // `row_drag` records the press of every gesture it starts.
        self.gesture_press
            .get()
            .map_or(Duration::MAX, |press| now(window).saturating_sub(press))
    }

    /// Publish `pane`'s status line after its quick-drag note came or went.
    fn show_details(&self, window: &MainWindow, pane: PaneId) {
        let Ok(p) = self.presenter.try_borrow() else {
            return;
        };
        let details = crate::panel_models::details_rows(&p, pane);
        drop(p);
        crate::panel_models::refresh_details(window, details, pane);
    }

    /// Retire the quick-drag note once it has been shown for `QUICK_NOTE`.
    fn expire_quick_note(&self, window: &MainWindow) {
        if self
            .quick_note
            .get()
            .is_none_or(|shown| now(window).saturating_sub(shown) < QUICK_NOTE)
        {
            return;
        }
        let Ok(mut p) = self.presenter.try_borrow_mut() else {
            return;
        };
        self.quick_note.set(None);
        let pane = p.clear_quick_drag_note();
        drop(p);
        if let Some(pane) = pane {
            self.show_details(window, pane);
        }
    }

    fn flush_cancel(&self) -> bool {
        let send = |command| self.observe((self.send)(command));
        if self.shutdown_pending.get() {
            let Ok(mut p) = self.presenter.try_borrow_mut() else {
                return false;
            };
            self.shutdown_pending.set(false);
            p.shutdown_drag_drop(&send);
        }
        let Some(id) = self.cancel_pending.get() else {
            return true;
        };
        let Ok(mut p) = self.presenter.try_borrow_mut() else {
            return false;
        };
        self.cancel_pending.set(None);
        if p.drag_drop
            .lifecycle
            .active()
            .is_some_and(|gesture| gesture.id == id.0)
        {
            // Presenter retains the exact CancelDrag in its existing retry queue
            // even when backend backpressure outlives this local gesture.
            p.cancel_file_drag(&send);
        }
        true
    }

    fn update(&self, window: &MainWindow) {
        use crate::presenter::drag_drop::preview::Status;
        if !self.flush_cancel() {
            return;
        }
        let Some((active, visual, label)) =
            crate::with_owned_commands(&self.presenter, &self.begin_owned, |p, send| {
                let active = p.drag_drop.lifecycle.active();
                let target = active
                    .filter(|g| g.phase == Phase::Ready)
                    .and_then(|_| self.target(window, p));
                let intent = self.intent();
                p.update_drag_preview(
                    target.as_ref().map(|target| target.destination.clone()),
                    intent,
                    send,
                );
                let feedback = match active.map(|g| g.phase) {
                    None | Some(Phase::Native) => 0,
                    Some(Phase::Ready) => match p.drag_preview_status() {
                        Status::Pending => 2,
                        Status::Allowed(TransferIntent::Copy) => 3,
                        Status::Allowed(TransferIntent::Move) => 4,
                        Status::Rejected => 5,
                    },
                    _ => 1,
                };
                let hover = if active.is_some_and(|g| g.phase != Phase::Native) {
                    target
                        .as_ref()
                        .filter(|_| matches!(feedback, 3 | 4))
                        .map(|target| target.pane)
                } else {
                    p.drag_drop.hover
                };
                if active.is_some_and(|g| g.phase != Phase::Native) {
                    p.drag_drop.hover = hover;
                }
                let row = target
                    .as_ref()
                    .filter(|_| matches!(feedback, 3 | 4))
                    .map_or(-1, |target| target.row);
                let item_count = p.drag_drop.item_count;
                let visual = (
                    active.map(|g| g.phase),
                    item_count,
                    hover,
                    row,
                    intent,
                    target
                        .as_ref()
                        .map_or_else(Default::default, |target| target.name.clone()),
                    feedback,
                );
                let destination = target
                    .as_ref()
                    .filter(|target| target.row >= 0)
                    .map_or("this panel", |target| target.name.as_str());
                let label = match feedback {
                    1 => "Preparing drag…".into(),
                    2 => "Checking destination…".into(),
                    3 | 4 => format!(
                        "{} {} item(s) into {destination}",
                        if feedback == 3 { "Copy" } else { "Move" },
                        item_count
                    )
                    .into(),
                    5 => "Not allowed here".into(),
                    _ => "Copy into this panel".into(),
                };
                (active, visual, label)
            })
        else {
            return;
        };
        let (_, _, hover, row, _, _, feedback) = &visual;
        let (hover, row, feedback) = (*hover, *row, *feedback);
        // Property updates may synchronously report lost focus/cancellation.
        // Never keep the Presenter borrow across those callbacks.
        let previous_feedback = self.visual.borrow().as_ref().map_or(0, |visual| visual.6);
        if self.visual.borrow().as_ref() != Some(&visual) {
            *self.visual.borrow_mut() = Some(visual);
            window.set_drag_active(active.is_some());
            if self.gesture.current() != active {
                return;
            }
            window.set_drag_hover_pane(hover.map_or(-1, |pane| pane.index() as i32));
            window.set_drag_hover_row(row);
            window.set_drag_feedback_state(feedback);
            window.set_drag_label(label);
        }
        let cursor = self.cursor.get();
        window.set_drag_x(cursor.x);
        window.set_drag_y(cursor.y);
        // Slint may restore its normal pointer during dispatch. Reapply only for
        // our internal gesture, then restore once; native OLE owns external feedback.
        if feedback != 0 || previous_feedback != 0 {
            use slint::winit_030::winit::window::CursorIcon;
            let cursor = match feedback {
                1 | 2 => CursorIcon::Progress,
                3 => CursorIcon::Copy,
                4 => CursorIcon::Move,
                5 => CursorIcon::NotAllowed,
                _ => CursorIcon::Default,
            };
            window
                .window()
                .with_winit_window(|window| window.set_cursor(cursor));
        }
    }
    fn publish(&self, window: &MainWindow) {
        let rectangles = rectangles(window);
        let Ok(mut p) = self.presenter.try_borrow_mut() else {
            self.retry.set(true);
            return;
        };
        // This flush retries every queued drag command; its refusals set the cadence.
        self.retry.set(false);
        let send = |command| self.observe((self.send)(command));
        if let Some(host) = host(window) {
            p.attach_drag_host(host, &send);
        }
        self.remember_host(&p);
        p.publish_drag_targets(rectangles);
        p.flush_drag_control_commands(&send);
        drop(p);
        let _ = crate::with_owned_commands(&self.presenter, &self.begin_owned, |p, send| {
            p.flush_drag_commands(&|command| self.observe(send(command)));
        });
    }

    /// One poll, then the cadence its outcome needs.
    fn turn(&self, window: &MainWindow) {
        self.poll(window);
        self.expire_quick_note(window);
        self.cadence();
    }

    fn cadence(&self) {
        let pending = self.retry.get()
            || self.handoff.get()
            || self.shutdown_pending.get()
            || self.cancel_pending.get().is_some()
            || self.gesture.current().is_some()
            || self
                .presenter
                .try_borrow()
                .map_or(true, |p| p.drag_drop.hover.is_some());
        let interval = if pending { ACTIVE_POLL } else { IDLE_POLL };
        if self.timer.interval() != interval {
            self.timer.set_interval(interval);
        }
    }

    /// Queue one zero-delay turn after the current callback has released its borrows.
    fn schedule_turn(self: &Rc<Self>) {
        if self.turn_scheduled.replace(true) {
            return;
        }
        let bridge = Rc::downgrade(self);
        slint::Timer::single_shot(Duration::ZERO, move || {
            let Some(bridge) = bridge.upgrade() else {
                return;
            };
            bridge.turn_scheduled.set(false);
            if let Some(window) = bridge.window.upgrade() {
                bridge.turn(&window);
            }
        });
    }

    fn owns(&self, window: &MainWindow) -> bool {
        self.window
            .upgrade()
            .is_some_and(|own| std::ptr::eq(own.window(), window.window()))
    }

    fn target(&self, window: &MainWindow, p: &Presenter) -> Option<Target> {
        // Ctrl+Shift requests a link in native shells. We do not silently copy/move it.
        let (control, shift, link) = self.modifiers.get();
        if link || (control && shift) {
            return None;
        }
        let cursor = self.cursor.get();
        let scale = window.window().scale_factor();
        let index = rectangles(window).iter().position(|rect| {
            rect.is_some_and(|rect| {
                let x = cursor.x * scale;
                let y = cursor.y * scale;
                x >= rect.x as f32
                    && x < rect.x as f32 + rect.width as f32
                    && y >= rect.y as f32
                    && y < rect.y as f32 + rect.height as f32
            })
        })?;
        let pane = pane(index as i32);
        let row = crate::selection_bridge::drop_row_at(window, p, pane, (cursor.x, cursor.y))?;
        let panel = &p.panes[pane.index()];
        let displayed = row
            .and_then(|row| row.checked_sub(panel.parent_offset()))
            .and_then(|row| panel.display_entry(row));
        if row.is_some() && displayed.is_none() {
            return None;
        }
        let identity = displayed.map(|entry| panel.row_identity(entry));
        let destination = p.preview_internal_drag_target(pane, row.zip(identity.as_deref()))?;
        let (row, name) = match &destination {
            InternalDropDestination::PaneRoot { .. } => (-1, Default::default()),
            _ => (
                row.map_or(-1, |row| row as i32),
                displayed.map_or_else(Default::default, |entry| {
                    crate::shown_text(&entry.name.to_string_lossy())
                }),
            ),
        };
        Some(Target {
            destination,
            pane,
            row,
            name,
        })
    }

    fn intent(&self) -> InternalDropIntent {
        let (control, shift, _) = self.modifiers.get();
        if control {
            InternalDropIntent::Copy
        } else if shift {
            InternalDropIntent::Move
        } else {
            InternalDropIntent::Auto
        }
    }

    fn moved(&self, window: &MainWindow, x: f32, y: f32) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        self.cursor.set(slint::LogicalPosition::new(x, y));
        self.publish(window);
        self.update(window);
    }

    fn released(&self, window: &MainWindow, x: f32, y: f32) {
        if self.handoff.get() {
            return;
        }
        self.cursor.set(slint::LogicalPosition::new(x, y));
        self.publish(window);
        self.update(window);
        if !self.flush_cancel() {
            return;
        }
        let held = self.held(window);
        let Some(noted) =
            crate::with_owned_commands(&self.presenter, &self.begin_owned, |p, send| {
                let target = self.target(window, p).filter(|_| {
                    !matches!(
                        p.drag_preview_status(),
                        crate::presenter::drag_drop::preview::Status::Rejected
                    )
                });
                p.release_file_drag_held(
                    held,
                    target.map(|target| target.destination),
                    self.intent(),
                    send,
                )
            })
        else {
            self.cancel(window, false);
            return;
        };
        self.update(window);
        // Released too soon: the presenter cancelled the drag and its panel says so.
        if let Some(pane) = noted {
            self.quick_note.set(Some(now(window)));
            self.show_details(window, pane);
        }
    }

    fn cancel(&self, window: &MainWindow, force: bool) -> bool {
        if self.handoff.get() {
            return false;
        }
        let Some(gesture) = self
            .gesture
            .current()
            .filter(|g| force || g.phase != Phase::Native)
        else {
            return false;
        };
        self.cancel_pending.set(Some(DragId(gesture.id)));
        window.set_drag_active(false);
        window.set_drag_hover_pane(-1);
        window.set_drag_hover_row(-1);
        window.set_drag_feedback_state(0);
        // The timer retries this exact ID while refresh owns a Presenter borrow.
        // Every release/new-gesture/native-handoff path drains it first.
        if self.flush_cancel() {
            self.update(window);
        }
        self.cadence();
        true
    }

    fn poll(&self, window: &MainWindow) {
        if !self.flush_cancel() {
            return;
        }
        self.publish(window);
        let id = {
            let Ok(presenter) = self.presenter.try_borrow() else {
                return;
            };
            let cursor = self.cursor.get();
            let size = window
                .window()
                .size()
                .to_logical(window.window().scale_factor());
            let outside = cursor.x < 0.0
                || cursor.y < 0.0
                || cursor.x >= size.width
                || cursor.y >= size.height;
            // Other applications get only a drag that has lasted MIN_DRAG_DURATION.
            // Until then this window keeps the pointer and sees its release, which
            // cancels the drag as `released` does inside it; once the operating
            // system runs the drag, no drop can come sooner.
            (outside && self.held(window) >= MIN_DRAG_DURATION)
                .then(|| presenter.drag_native_ready())
                .flatten()
        };
        if let Some(id) = id {
            let armed = crate::with_owned_commands(&self.presenter, &self.begin_owned, |p, _| {
                p.arm_native_drag(id)
            })
            .unwrap_or(false);
            if armed {
                // Mark the native phase before releasing Slint's and the OS's
                // grabs. Their synchronous cancel/release notifications are not
                // a new user cancellation of the committed source gesture.
                self.handoff.set(true);
                window.invoke_release_drag_input();
                window
                    .window()
                    .dispatch_event(WindowEvent::PointerReleased {
                        position: self.cursor.get(),
                        button: PointerEventButton::Left,
                    });
                let mut released = false;
                window
                    .window()
                    .with_winit_window(|window| released = native::release_input(window));
                self.handoff.set(false);
                if !released || !(self.send)(Command::StartDrag(id)) {
                    self.cancel_pending.set(Some(id));
                    self.flush_cancel();
                }
            }
        }
        self.update(window);
    }
}

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
    begin_owned: BeginOwned,
) {
    let (targets, gesture) = {
        let p = presenter.borrow();
        (
            p.drag_drop.targets.clone(),
            p.drag_drop.lifecycle.snapshot(),
        )
    };
    let bridge = Rc::new(Bridge {
        window: window.as_weak(),
        presenter,
        send,
        begin_owned,
        targets: targets.clone(),
        cursor: Cell::new(slint::LogicalPosition::new(-1.0, -1.0)),
        modifiers: Cell::new((false, false, false)),
        handoff: Cell::new(false),
        gesture,
        cancel_pending: Cell::new(None),
        shutdown_pending: Cell::new(false),
        visual: RefCell::new(None),
        lifetime: RefCell::new(None),
        timer: slint::Timer::default(),
        turn_scheduled: Cell::new(false),
        retry: Cell::new(false),
        press: Cell::new(None),
        gesture_press: Cell::new(None),
        quick_note: Cell::new(None),
    });
    BRIDGES.with(|bridges| {
        let mut bridges = bridges.borrow_mut();
        bridges.retain(|bridge| bridge.strong_count() > 0);
        bridges.push(Rc::downgrade(&bridge));
    });
    let state = bridge.clone();
    window.on_drag_invalidate_targets(move || {
        invalidate(&targets);
        state.schedule_turn();
    });
    let weak = window.as_weak();
    let state = bridge.clone();
    window.on_row_drag(move |id, row, identity, x, y| {
        let Some(window) = weak.upgrade() else {
            return false;
        };
        let Ok(row) = usize::try_from(row) else {
            return false;
        };
        state.cursor.set(slint::LogicalPosition::new(x, y));
        if !state.flush_cancel() {
            return false;
        }
        // The row crossed its movement threshold after the press that holds it.
        let pressed = state.press.get().unwrap_or_else(|| now(&window));
        let drag_host = host(&window).unwrap_or(NativeDragWindow::Unsupported);
        let Some(started) =
            crate::with_owned_commands(&state.presenter, &state.begin_owned, |p, send| {
                p.attach_drag_host(drag_host, send);
                state.remember_host(p);
                if !p.drag_drop.attached {
                    return false;
                }
                p.begin_file_drag(pane(id), row, identity.as_str(), send)
            })
        else {
            return false;
        };
        if started {
            state.gesture_press.set(Some(pressed));
        }
        crate::refresh(&window, &state.presenter.borrow());
        state.publish(&window);
        state.update(&window);
        state.cadence();
        started
    });
    let weak = window.as_weak();
    let state = bridge.clone();
    window.on_drag_moved(move |x, y| {
        if let Some(window) = weak.upgrade() {
            state.moved(&window, x, y);
        }
    });
    let weak = window.as_weak();
    let state = bridge.clone();
    window.on_drag_modifiers(move |control, shift, link| {
        state.modifiers.set((control, shift, link));
        if let Some(window) = weak.upgrade() {
            state.update(&window);
        }
    });
    let weak = window.as_weak();
    let state = bridge.clone();
    window.on_drag_released(move |x, y| {
        if let Some(window) = weak.upgrade() {
            state.released(&window, x, y);
        }
    });
    let weak = window.as_weak();
    let state = bridge.clone();
    window.on_drag_cancel(move || {
        if let Some(window) = weak.upgrade() {
            state.cancel(&window, false);
        }
    });
    let weak = window.as_weak();
    let state = bridge.clone();
    window.on_drag_escape(move || {
        weak.upgrade()
            .is_some_and(|window| state.cancel(&window, true))
    });
    let state = bridge.clone();
    window.on_drag_host_closed(move || {
        invalidate(&state.targets);
        if let Some((_, lifetime)) = state.lifetime.borrow().as_ref() {
            lifetime.cancel();
        }
        state.shutdown_pending.set(true);
        state.flush_cancel();
        state.cadence();
    });
    let weak = window.as_weak();
    let state = Rc::downgrade(&bridge);
    bridge
        .timer
        .start(slint::TimerMode::Repeated, IDLE_POLL, move || {
            if let (Some(window), Some(state)) = (weak.upgrade(), state.upgrade()) {
                state.turn(&window);
            }
        });
    bridge.cadence();
}

/// The bridge installed on `window`, if any. None while thread storage is torn down.
fn bridge(window: &MainWindow) -> Option<Rc<Bridge>> {
    BRIDGES
        .try_with(|bridges| {
            let mut bridges = bridges.borrow_mut();
            bridges.retain(|bridge| bridge.strong_count() > 0);
            bridges
                .iter()
                .filter_map(Weak::upgrade)
                .find(|bridge| bridge.owns(window))
        })
        .ok()
        .flatten()
}

/// Remember when the primary button went down in `window`, or forget it once released.
fn pressed(window: &MainWindow, down: bool) {
    if let Some(bridge) = bridge(window) {
        bridge.press.set(down.then(|| now(window)));
    }
}

/// Republish targets and present drag feedback on the next loop turn.
fn schedule(window: &MainWindow) {
    if let Some(bridge) = bridge(window) {
        bridge.schedule_turn();
    }
}

/// Every presenter refresh can change navigation, loading, modal or hover state.
pub(crate) fn refresh(window: &MainWindow, p: &Presenter) {
    let active = p.drag_drop.lifecycle.active();
    window.set_drag_active(active.is_some());
    if active.is_none() && p.drag_drop.hover.is_none() {
        window.set_drag_hover_pane(-1);
        window.set_drag_hover_row(-1);
    }
    schedule(window);
}

/// The shared software/native event observer runs after normal Slint dispatch.
/// Repeated row/global notifications are harmless because release consumes a token.
pub(crate) fn observe(window: &MainWindow, event: &WindowEvent) {
    match event {
        // The window's own press, not a list's possibly delayed one, times a drag.
        WindowEvent::PointerPressed {
            button: PointerEventButton::Left,
            ..
        } => pressed(window, true),
        WindowEvent::PointerMoved { position } if window.get_drag_active() => {
            window.invoke_drag_moved(position.x, position.y)
        }
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        } => {
            if window.get_drag_active() {
                window.invoke_drag_released(position.x, position.y);
            }
            pressed(window, false);
        }
        WindowEvent::PointerExited if window.get_drag_active() => {
            window.invoke_drag_moved(-1.0, -1.0)
        }
        WindowEvent::WindowActiveChanged(false) => window.invoke_drag_cancel(),
        _ => {}
    }
}

pub(crate) fn observe_native(
    window: &MainWindow,
    event: &slint::winit_030::winit::event::WindowEvent,
) {
    use slint::winit_030::winit::event::{ElementState, WindowEvent};
    match event {
        WindowEvent::ModifiersChanged(modifiers) => {
            let state = modifiers.state();
            window.invoke_drag_modifiers(
                state.control_key(),
                state.shift_key(),
                state.alt_key() || state.super_key(),
            );
        }
        WindowEvent::Resized(_)
        | WindowEvent::Moved(_)
        | WindowEvent::ScaleFactorChanged { .. }
        | WindowEvent::MouseInput {
            state: ElementState::Pressed,
            ..
        }
        | WindowEvent::KeyboardInput { .. } => window.invoke_drag_invalidate_targets(),
        // A popup closed on release, or a scroll started by wheel or touch, changes
        // the published regions without invalidating them.
        WindowEvent::MouseInput {
            state: ElementState::Released,
            ..
        }
        | WindowEvent::MouseWheel { .. }
        | WindowEvent::Touch(_) => schedule(window),
        WindowEvent::Focused(false) => window.invoke_drag_cancel(),
        WindowEvent::Destroyed => window.invoke_drag_host_closed(),
        _ => {}
    }
}

#[cfg(test)]
#[path = "drag_bridge_tests.rs"]
mod tests;
