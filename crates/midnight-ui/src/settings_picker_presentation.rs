//! Weak, delayed cancellation retry while a native picker owns its bounded slot.
use crate::{MainWindow, Presenter};
use midnight_core::Command;
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    time::Duration,
};

pub(crate) fn wake(
    window: &MainWindow,
    state: &Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    {
        let mut p = state.borrow_mut();
        if p.settings_picker.poll_scheduled || !p.settings_picker.busy() || p.operations.closing {
            return;
        }
        p.settings_picker.poll_scheduled = true;
    }
    later(window.as_weak(), Rc::downgrade(state), send);
}
fn later(
    window: slint::Weak<MainWindow>,
    state: Weak<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    slint::Timer::single_shot(Duration::from_millis(25), move || {
        let (Some(w), Some(p)) = (window.upgrade(), state.upgrade()) else {
            return;
        };
        let Ok(mut draft) = p.try_borrow_mut() else {
            later(w.as_weak(), Rc::downgrade(&p), send);
            return;
        };
        draft.settings_picker.poll_scheduled = false;
        if draft.operations.closing {
            return;
        }
        let was_busy = draft.settings_picker.busy();
        draft.flush_settings_picker(send.as_ref());
        let changed = was_busy != draft.settings_picker.busy();
        drop(draft);
        if changed {
            crate::refresh(&w, &p.borrow());
        }
        wake(&w, &p, send);
    });
}
