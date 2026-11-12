//! Path-only paste requests bind a native editor snapshot to a backend clipboard reply.
use crate::{MainWindow, PathPasteView, Presenter, path_paste_input::Snapshot};
use midnight_core::{Command, DesktopRequestId, PaneId};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc, time::Duration};

struct Pending {
    id: DesktopRequestId,
    navigation: u64,
    epoch: Rc<()>,
    input: Snapshot,
}
struct Controller {
    epochs: [Rc<()>; 2],
    pending: [Option<Pending>; 2],
    scheduled: bool,
}
impl Default for Controller {
    fn default() -> Self {
        Self {
            epochs: std::array::from_fn(|_| Rc::new(())),
            pending: [None, None],
            scheduled: false,
        }
    }
}
pub(crate) fn install(
    window: &MainWindow,
    state: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let controller = Rc::new(RefCell::new(Controller::default()));
    let c = controller.clone();
    window.global::<PathPasteView>().on_invalidate(move |pane| {
        if let Some(epoch) = c.borrow_mut().epochs.get_mut(pane as usize) {
            *epoch = Rc::new(());
        }
    });
    let c = controller.clone();
    let weak = window.as_weak();
    window.global::<PathPasteView>().on_observe(move |pane| {
        let Ok(index) = usize::try_from(pane) else {
            return;
        };
        if index >= 2 {
            return;
        }
        // Layout and popup focus restoration can notify without changing the
        // editor. Release the controller borrow before reading Slint properties.
        let snapshot = c.borrow().pending[index]
            .as_ref()
            .map(|pending| pending.input.clone());
        if let (Some(snapshot), Some(window)) = (snapshot, weak.upgrade())
            && snapshot.current(&window)
        {
            return;
        }
        c.borrow_mut().epochs[index] = Rc::new(());
    });
    let weak = window.as_weak();
    window.global::<PathPasteView>().on_request(move |pane| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let pane = match pane {
            0 => PaneId::Left,
            1 => PaneId::Right,
            _ => return,
        };
        let Some(input) = Snapshot::capture(&window) else {
            return;
        };
        controller.borrow_mut().pending[pane.index()] = None;
        let Ok(mut p) = state.try_borrow_mut() else {
            return;
        };
        if p.active != pane {
            return;
        }
        let id = p.request_path_paste(pane, send.as_ref());
        let navigation = p.panes[pane.index()].generation;
        drop(p);
        if let Some(id) = id {
            let mut c = controller.borrow_mut();
            let epoch = c.epochs[pane.index()].clone();
            c.pending[pane.index()] = Some(Pending {
                id,
                navigation,
                epoch,
                input,
            });
            drop(c);
            schedule(&window, &state, &controller);
        }
        if let Ok(presenter) = state.try_borrow() {
            crate::refresh(&window, &presenter);
        }
    });
}
fn schedule(
    window: &MainWindow,
    state: &Rc<RefCell<Presenter>>,
    controller: &Rc<RefCell<Controller>>,
) {
    if controller.borrow().scheduled {
        return;
    }
    controller.borrow_mut().scheduled = true;
    let weak = window.as_weak();
    let state = Rc::downgrade(state);
    let controller = Rc::downgrade(controller);
    slint::Timer::single_shot(Duration::from_millis(25), move || {
        let (Some(window), Some(state), Some(controller)) =
            (weak.upgrade(), state.upgrade(), controller.upgrade())
        else {
            return;
        };
        controller.borrow_mut().scheduled = false;
        for pane in [PaneId::Left, PaneId::Right] {
            let Some(pending) = controller.borrow_mut().pending[pane.index()].take() else {
                continue;
            };
            let same_epoch = Rc::ptr_eq(&controller.borrow().epochs[pane.index()], &pending.epoch);
            let current = same_epoch && pending.input.current(&window);
            let Ok(mut p) = state.try_borrow_mut() else {
                controller.borrow_mut().pending[pane.index()] = Some(pending);
                continue;
            };
            if !current
                || p.panes[pane.index()].generation != pending.navigation
                || !p.path_paste_pending(pane, pending.id)
            {
                p.cancel_path_paste(pane);
                continue;
            }
            let result = p.take_path_paste(pane, pending.id);
            drop(p);
            match result {
                None => controller.borrow_mut().pending[pane.index()] = Some(pending),
                Some(Ok(text)) => {
                    if !text.is_empty() {
                        pending.input.insert(&window, &text);
                    }
                    state.borrow_mut().retire_pasted_text(text);
                }
                Some(Err(error)) => state
                    .borrow_mut()
                    .path_paste_failure(crate::error_presentation::describe(&error)),
            }
        }
        if let Ok(presenter) = state.try_borrow() {
            crate::refresh(&window, &presenter);
        }
        let again = controller.borrow().pending.iter().any(Option::is_some);
        if again {
            schedule(&window, &state, &controller);
        }
    });
}
