//! Observe render submission without changing the renderer's flush path.
use slint::winit_030::{CustomApplicationHandler, EventResult, winit};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

pub type Callback = Rc<RefCell<Option<Box<dyn FnMut(Duration)>>>>;

pub struct Observer {
    pub callback: Callback,
    pub drawing: Option<Instant>,
}

impl Observer {
    fn finished(&mut self) {
        if let Some(started) = self.drawing.take()
            && let Some(callback) = self.callback.borrow_mut().as_mut()
        {
            callback(started.elapsed());
        }
    }
}

impl CustomApplicationHandler for Observer {
    fn window_event(
        &mut self,
        _: &winit::event_loop::ActiveEventLoop,
        _: winit::window::WindowId,
        _: Option<&winit::window::Window>,
        _: Option<&slint::Window>,
        event: &winit::event::WindowEvent,
    ) -> EventResult {
        self.finished();
        if matches!(event, winit::event::WindowEvent::RedrawRequested) {
            self.drawing = Some(Instant::now());
        }
        EventResult::Propagate
    }

    fn about_to_wait(&mut self, _: &winit::event_loop::ActiveEventLoop) -> EventResult {
        self.finished();
        EventResult::Propagate
    }
}
