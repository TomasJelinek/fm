//! Reveal the main window only after the asynchronous settings response has
//! reached its presentation. Native geometry is restored while it is hidden.
use crate::{startup_gate::Gate, window_state::WindowState};
use midnight_ui::{MainWindow, Presenter};
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Duration,
};
use tokio::sync::oneshot;

/// A settings read slower than this no longer keeps the window hidden. The presenter
/// still applies the read when it arrives and saves nothing until a read succeeds.
pub(crate) const DEADLINE: Duration = Duration::from_secs(10);

type Ready = Rc<RefCell<Option<oneshot::Sender<()>>>>;

pub(crate) struct Startup {
    ready: Ready,
    task: RefCell<Option<slint::JoinHandle<()>>>,
    watcher: RefCell<Option<slint::JoinHandle<()>>>,
    error: Rc<RefCell<Option<String>>>,
    revealed: Rc<Cell<bool>>,
}

impl Startup {
    pub(crate) fn install(
        window: &MainWindow,
        presenter: Rc<RefCell<Presenter>>,
        geometry: Rc<WindowState>,
        gate: Arc<Gate>,
    ) -> Result<Self, String> {
        let (ready, response) = oneshot::channel();
        let error = Rc::new(RefCell::new(None));
        let failure = error.clone();
        let revealed = Rc::new(Cell::new(false));
        let first_reveal = revealed.clone();
        let weak = window.as_weak();
        let task = slint::spawn_local(async move {
            if response.await.is_err() {
                return;
            }
            let Some(window) = weak.upgrade() else { return };
            // Slint 1.18 creates the native window hidden when its event loop
            // resumes. Await its existence, never show it merely to enumerate
            // monitors. Release the native Arc so normal hide/close still works.
            let result = match window.window().winit_window().await {
                Ok(native) => {
                    drop(native);
                    geometry.restore(&window, &presenter.borrow());
                    window
                        .show()
                        .map(|()| {
                            first_reveal.set(true);
                            gate.reveal();
                            window.invoke_refresh_information_windows();
                        })
                        .map_err(|error| error.to_string())
                }
                Err(error) => Err(error.to_string()),
            };
            if let Err(error) = result {
                gate.abort();
                *failure.borrow_mut() = Some(format!("Cannot show window: {error}"));
                let _ = slint::quit_event_loop();
            }
        })
        .map_err(|error| error.to_string())?;
        Ok(Self {
            ready: Rc::new(RefCell::new(Some(ready))),
            task: RefCell::new(Some(task)),
            watcher: RefCell::new(None),
            error,
            revealed,
        })
    }

    /// Never stay hidden indefinitely: reveal after `deadline`, or as soon as the backend
    /// stops (its sender is dropped when the engine returns or unwinds) without answering.
    pub(crate) fn arm(
        &self,
        deadline: Duration,
        engine_stopped: oneshot::Receiver<()>,
    ) -> Result<(), String> {
        let ready = self.ready.clone();
        slint::Timer::single_shot(deadline, move || release(&ready));
        let ready = self.ready.clone();
        let watcher = slint::spawn_local(async move {
            let _ = engine_stopped.await;
            release(&ready);
        })
        .map_err(|error| error.to_string())?;
        *self.watcher.borrow_mut() = Some(watcher);
        Ok(())
    }

    /// Call after refreshing the initial response, including a failed read.
    /// A failed request also releases startup after the UI reports its failure;
    /// the presenter's separate successful-read guard still prohibits saving.
    pub(crate) fn preferences_applied(&self) {
        release(&self.ready);
    }

    pub(crate) fn revealed(&self) -> bool {
        self.revealed.get()
    }

    pub(crate) fn finish(&self) -> Result<(), String> {
        for task in [&self.task, &self.watcher] {
            if let Some(task) = task.borrow_mut().take() {
                task.abort();
            }
        }
        self.error.borrow_mut().take().map_or(Ok(()), Err)
    }
}

fn release(ready: &Ready) {
    if let Some(ready) = ready.borrow_mut().take() {
        let _ = ready.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::platform::{EventLoopProxy, Platform, PlatformError, WindowAdapter};
    use std::sync::Mutex;

    type Queue = Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>;

    /// Timers follow an explicit clock; the polls of `spawn_local` futures wait in a queue
    /// that the test runs on its own thread. No proxy is offered while `set_platform`
    /// runs: Slint keeps that one process-wide for `invoke_from_event_loop`, and other
    /// tests in this binary rely on there being none.
    struct LoopPlatform {
        clock: Rc<Cell<Duration>>,
        queue: Queue,
        proxies: Rc<Cell<bool>>,
    }
    struct Proxy(Queue);
    impl EventLoopProxy for Proxy {
        fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> {
            Ok(())
        }
        fn invoke_from_event_loop(
            &self,
            event: Box<dyn FnOnce() + Send>,
        ) -> Result<(), slint::EventLoopError> {
            self.0.lock().unwrap().push(event);
            Ok(())
        }
    }
    impl Platform for LoopPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Err(PlatformError::NoPlatform)
        }
        fn duration_since_start(&self) -> Duration {
            self.clock.get()
        }
        fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
            self.proxies
                .get()
                .then(|| Box::new(Proxy(self.queue.clone())) as Box<dyn EventLoopProxy>)
        }
    }

    fn run_queued(queue: &Queue) {
        loop {
            let events = std::mem::take(&mut *queue.lock().unwrap());
            if events.is_empty() {
                return;
            }
            for event in events {
                event();
            }
        }
    }

    /// A startup waiting hidden for its settings read, as `install` leaves it.
    fn hidden() -> (Startup, oneshot::Receiver<()>) {
        let (ready, response) = oneshot::channel();
        let startup = Startup {
            ready: Rc::new(RefCell::new(Some(ready))),
            task: RefCell::new(None),
            watcher: RefCell::new(None),
            error: Rc::new(RefCell::new(None)),
            revealed: Rc::new(Cell::new(false)),
        };
        (startup, response)
    }

    #[test]
    fn an_armed_startup_is_released_by_its_deadline_or_a_stopped_backend() {
        let clock = Rc::new(Cell::new(Duration::ZERO));
        let queue = Queue::default();
        let proxies = Rc::new(Cell::new(false));
        slint::platform::set_platform(Box::new(LoopPlatform {
            clock: clock.clone(),
            queue: queue.clone(),
            proxies: proxies.clone(),
        }))
        .unwrap();
        proxies.set(true);
        let advance = |to: Duration| {
            clock.set(to);
            slint::platform::update_timers_and_animations();
            run_queued(&queue);
        };

        // The settings read never answers and the backend keeps running.
        let (unanswered, mut response) = hidden();
        let (_running, engine_stop) = oneshot::channel::<()>();
        unanswered.arm(DEADLINE, engine_stop).unwrap();
        advance(DEADLINE - Duration::from_millis(1));
        assert!(
            matches!(
                response.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ),
            "startup was released before its deadline"
        );
        advance(DEADLINE);
        assert!(
            response.try_recv().is_ok(),
            "the deadline must reveal a startup whose read never answers"
        );
        // A read that arrives late still applies, but releases nothing twice.
        unanswered.preferences_applied();
        unanswered.finish().unwrap();

        // The backend stops (here: unwinds) before answering; its dropped sender
        // releases startup at once, long before the deadline.
        let (stopping, mut response) = hidden();
        let (stopped, engine_stop) = oneshot::channel::<()>();
        stopping.arm(DEADLINE, engine_stop).unwrap();
        run_queued(&queue);
        assert!(matches!(
            response.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        let engine = std::thread::spawn(move || {
            let _stopped = stopped;
            panic!("engine stopped unexpectedly");
        });
        assert!(engine.join().is_err());
        run_queued(&queue);
        assert!(
            response.try_recv().is_ok(),
            "a stopped backend must reveal a startup that is still hidden"
        );
        stopping.finish().unwrap();
    }
}
