//! Reveal the main window only after the asynchronous settings response has
//! reached its presentation. Native geometry is restored while it is hidden.
use crate::{startup_gate::Gate, window_state::WindowState};
use midnight_ui::{MainWindow, Presenter};
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};
use tokio::sync::oneshot;

pub(crate) struct Startup {
    ready: RefCell<Option<oneshot::Sender<()>>>,
    task: RefCell<Option<slint::JoinHandle<()>>>,
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
            ready: RefCell::new(Some(ready)),
            task: RefCell::new(Some(task)),
            error,
            revealed,
        })
    }

    /// Call after refreshing the initial response, including a failed read.
    /// A failed request also releases startup after the UI reports its failure;
    /// the presenter's separate successful-read guard still prohibits saving.
    pub(crate) fn preferences_applied(&self) {
        if let Some(ready) = self.ready.borrow_mut().take() {
            let _ = ready.send(());
        }
    }

    pub(crate) fn revealed(&self) -> bool {
        self.revealed.get()
    }

    pub(crate) fn finish(&self) -> Result<(), String> {
        if let Some(task) = self.task.borrow_mut().take() {
            task.abort();
        }
        self.error.borrow_mut().take().map_or(Ok(()), Err)
    }
}
