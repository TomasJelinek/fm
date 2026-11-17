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

    #[test]
    fn the_first_of_read_deadline_or_backend_stop_releases_startup_exactly_once() {
        let (sender, mut receiver) = oneshot::channel();
        let ready: Ready = Rc::new(RefCell::new(Some(sender)));
        // The read, the deadline timer and the backend watcher share one release.
        release(&ready);
        release(&ready);
        assert!(receiver.try_recv().is_ok());
        assert!(ready.borrow().is_none());
        // A backend that stops without answering resolves its watcher by dropping the
        // sender, which happens while its task returns or unwinds.
        let (stopped, watcher) = oneshot::channel::<()>();
        let mut watcher = watcher;
        let engine = std::thread::spawn(move || {
            let _stopped = stopped;
            panic!("engine stopped unexpectedly");
        });
        assert!(engine.join().is_err());
        assert!(matches!(
            watcher.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
    }
}
