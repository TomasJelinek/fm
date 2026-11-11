//! Observe render submission without changing the renderer's flush path.
use midnight_viewer_ui::ViewerWindow;
use slint::{
    ComponentHandle,
    winit_030::{CustomApplicationHandler, EventResult, WinitWindowAccessor, winit},
};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

pub struct Callbacks {
    pub window: slint::Weak<ViewerWindow>,
    pub before: Box<dyn FnMut()>,
    pub after: Box<dyn FnMut(winit::window::WindowId, Duration)>,
}
pub type Callback = Rc<RefCell<Option<Callbacks>>>;

pub struct Observer {
    pub callback: Callback,
    pub drawing: Option<(winit::window::WindowId, Instant)>,
    pub injected_wheel: bool,
    pub native_events: Rc<RefCell<Vec<String>>>,
    pub timeline: super::timeline::Trace,
}

impl Observer {
    fn finished(&mut self) {
        if let Some((window_id, started)) = self.drawing.take()
            && let Some(callback) = self.callback.borrow_mut().as_mut()
        {
            let elapsed = started.elapsed();
            self.timeline
                .record("redraw_end", elapsed.as_secs_f64() * 1000.0);
            (callback.after)(window_id, elapsed);
        }
    }
}

impl CustomApplicationHandler for Observer {
    fn window_event(
        &mut self,
        _: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        _: Option<&winit::window::Window>,
        _: Option<&slint::Window>,
        event: &winit::event::WindowEvent,
    ) -> EventResult {
        self.finished();
        let target = self.callback.borrow().as_ref().and_then(|callback| {
            callback
                .window
                .upgrade()
                .and_then(|window| window.window().with_winit_window(|native| native.id()))
        });
        if target != Some(window_id) {
            return EventResult::Propagate;
        }
        if matches!(event, winit::event::WindowEvent::MouseWheel { .. }) {
            self.timeline.record("native_wheel", 0.0);
        }
        if self.injected_wheel
            && matches!(
                event,
                winit::event::WindowEvent::MouseWheel { .. }
                    | winit::event::WindowEvent::CursorMoved { .. }
                    | winit::event::WindowEvent::CursorLeft { .. }
                    | winit::event::WindowEvent::Focused(_)
                    | winit::event::WindowEvent::Resized(_)
            )
            && self.native_events.borrow().len() < 4096
        {
            self.native_events
                .borrow_mut()
                .push(format!("wheel_native {event:?}"));
        }
        // Synthetic native mouse messages leave the desktop cursor in place.
        // Its unrelated leave notification must not cancel this probe gesture.
        if self.injected_wheel && matches!(event, winit::event::WindowEvent::CursorLeft { .. }) {
            return EventResult::PreventDefault;
        }
        if matches!(event, winit::event::WindowEvent::RedrawRequested) {
            if let Some(callback) = self.callback.borrow_mut().as_mut() {
                (callback.before)();
            }
            self.timeline.record("redraw_begin", 0.0);
            self.drawing = Some((window_id, Instant::now()));
        }
        EventResult::Propagate
    }

    fn about_to_wait(&mut self, _: &winit::event_loop::ActiveEventLoop) -> EventResult {
        self.finished();
        EventResult::Propagate
    }
}
