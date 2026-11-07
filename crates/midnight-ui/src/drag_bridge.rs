//! UI input and geometry only; native payloads and filesystem work stay on workers.
use crate::{MainWindow, Presenter, drag_lifecycle::Phase, pane, presenter::drag_drop::invalidate};
use i_slint_core::window::{WindowInner, WindowKind};
use midnight_core::*;
use slint::{
    ComponentHandle,
    platform::{PointerEventButton, WindowEvent},
    winit_030::WinitWindowAccessor,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

#[path = "drag_input_native.rs"]
mod native;

type DragVisual = (Option<Phase>, usize, Option<PaneId>);

struct Bridge {
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
    targets: SharedDropTargets,
    cursor: Cell<slint::LogicalPosition>,
    handoff: Cell<bool>,
    visual: Cell<Option<DragVisual>>,
    lifetime: RefCell<Option<(u64, Cancellation)>>,
    timer: slint::Timer,
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

fn host(window: &MainWindow) -> Option<NativeDragWindow> {
    let mut host = None;
    window
        .window()
        .with_winit_window(|window| host = Some(native::host(window)));
    host
}

fn rectangles(window: &MainWindow) -> [Option<DropRect>; 2] {
    if !window.window().is_visible()
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
    std::array::from_fn(|pane| {
        if window.get_panel_zoomed() && window.get_active_pane() != pane as i32 {
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
    fn update(&self, window: &MainWindow, p: &Presenter) {
        let visual = (
            p.drag_drop.lifecycle.active.map(|g| g.phase),
            p.drag_drop.item_count,
            p.drag_drop.hover,
        );
        if self.visual.replace(Some(visual)) != Some(visual) {
            refresh(window, p);
        }
    }
    fn publish(&self, window: &MainWindow) {
        let Ok(mut p) = self.presenter.try_borrow_mut() else {
            return;
        };
        if let Some(host) = host(window) {
            p.attach_drag_host(host, self.send.as_ref());
        }
        self.remember_host(&p);
        p.publish_drag_targets(rectangles(window));
        p.flush_drag_commands(self.send.as_ref());
    }

    fn target(&self, window: &MainWindow) -> Option<(FileDropTarget, u64)> {
        let cursor = self.cursor.get();
        let scale = window.window().scale_factor();
        let targets = self.targets.lock().unwrap_or_else(|e| e.into_inner());
        targets
            .target_at(
                targets.host_generation,
                (cursor.x * scale).floor() as i32,
                (cursor.y * scale).floor() as i32,
            )
            .map(|target| (target, targets.revision))
    }

    fn moved(&self, window: &MainWindow, x: f32, y: f32) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        self.cursor.set(slint::LogicalPosition::new(x, y));
        self.publish(window);
        let mut p = self.presenter.borrow_mut();
        if p.drag_drop
            .lifecycle
            .active
            .is_some_and(|g| g.phase != Phase::Native)
        {
            p.drag_drop.hover = p
                .drag_drop
                .lifecycle
                .active
                .filter(|g| g.phase == Phase::Ready)
                .and_then(|_| self.target(window).map(|(target, _)| target.pane));
        }
        self.update(window, &p);
    }

    fn released(&self, window: &MainWindow, x: f32, y: f32) {
        if self.handoff.get() {
            return;
        }
        self.cursor.set(slint::LogicalPosition::new(x, y));
        self.publish(window);
        let target = self.target(window);
        let mut p = self.presenter.borrow_mut();
        p.release_file_drag(target, self.send.as_ref());
        self.update(window, &p);
    }

    fn cancel(&self, window: &MainWindow, force: bool) -> bool {
        if self.handoff.get() {
            return false;
        }
        let mut p = self.presenter.borrow_mut();
        let active = p
            .drag_drop
            .lifecycle
            .active
            .is_some_and(|g| force || g.phase != Phase::Native);
        if active {
            p.cancel_file_drag(self.send.as_ref());
        }
        self.update(window, &p);
        active
    }

    fn poll(&self, window: &MainWindow) {
        self.publish(window);
        let id = self.presenter.borrow().drag_native_ready();
        if let Some(id) = id {
            let armed = self.presenter.borrow_mut().arm_native_drag(id);
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
                    self.presenter
                        .borrow_mut()
                        .cancel_file_drag(self.send.as_ref());
                }
            }
        }
        self.update(window, &self.presenter.borrow());
    }
}

pub(crate) fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let targets = presenter.borrow().drag_drop.targets.clone();
    let bridge = Rc::new(Bridge {
        presenter,
        send,
        targets: targets.clone(),
        cursor: Cell::new(slint::LogicalPosition::new(-1.0, -1.0)),
        handoff: Cell::new(false),
        visual: Cell::new(None),
        lifetime: RefCell::new(None),
        timer: slint::Timer::default(),
    });
    window.on_drag_invalidate_targets(move || invalidate(&targets));
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
        let started = {
            let mut p = state.presenter.borrow_mut();
            p.attach_drag_host(
                host(&window).unwrap_or(NativeDragWindow::Unsupported),
                state.send.as_ref(),
            );
            state.remember_host(&p);
            if !p.drag_drop.attached {
                return false;
            }
            p.begin_file_drag(pane(id), row, identity.as_str(), state.send.as_ref())
        };
        crate::refresh(&window, &state.presenter.borrow());
        state.publish(&window);
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
        state
            .presenter
            .borrow_mut()
            .shutdown_drag_drop(state.send.as_ref())
    });
    let weak = window.as_weak();
    let state = Rc::downgrade(&bridge);
    bridge.timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(16),
        move || {
            if let (Some(window), Some(state)) = (weak.upgrade(), state.upgrade()) {
                state.poll(&window);
            }
        },
    );
}

pub(crate) fn refresh(window: &MainWindow, p: &Presenter) {
    let active = p.drag_drop.lifecycle.active;
    window.set_drag_active(active.is_some());
    window.set_drag_hover_pane(p.drag_drop.hover.map_or(-1, |pane| pane.index() as i32));
    window.set_drag_label(if active.is_some() && p.drag_drop.item_count > 0 {
        format!("Copy {} item(s) here", p.drag_drop.item_count).into()
    } else {
        "Copy here".into()
    });
}

/// The shared software/native event observer runs after normal Slint dispatch.
/// Repeated row/global notifications are harmless because release consumes a token.
pub(crate) fn observe(window: &MainWindow, event: &WindowEvent) {
    match event {
        WindowEvent::PointerMoved { position } if window.get_drag_active() => {
            window.invoke_drag_moved(position.x, position.y)
        }
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        } if window.get_drag_active() => window.invoke_drag_released(position.x, position.y),
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
        WindowEvent::Resized(_)
        | WindowEvent::Moved(_)
        | WindowEvent::ScaleFactorChanged { .. }
        | WindowEvent::MouseInput {
            state: ElementState::Pressed,
            ..
        }
        | WindowEvent::KeyboardInput { .. } => window.invoke_drag_invalidate_targets(),
        WindowEvent::Focused(false) => window.invoke_drag_cancel(),
        WindowEvent::Destroyed => window.invoke_drag_host_closed(),
        _ => {}
    }
}
