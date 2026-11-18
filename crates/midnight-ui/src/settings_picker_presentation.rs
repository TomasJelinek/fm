//! Weak, delayed cancellation retry while a native picker owns its bounded slot.
//!
//! One restartable check per window: once a second while a picker is open with nothing
//! to cancel, every 25 ms while a cancellation waits for the queue, and not at all
//! otherwise. Callbacks that invalidate a picker call [`wake`], which re-arms a pending
//! slow check at the cancellation cadence.
use crate::{MainWindow, Presenter};
use midnight_core::Command;
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    time::Duration,
};

/// Retry cadence while a cancellation is pending or the presenter is borrowed.
const CANCEL_RETRY: Duration = Duration::from_millis(25);
/// Safety check while a picker is open and nothing needs cancelling.
const OPEN_CHECK: Duration = Duration::from_secs(1);

thread_local! {
    /// This UI thread's checks, each owned by its window.
    static CHECKS: RefCell<Vec<(slint::Weak<MainWindow>, Rc<slint::Timer>)>> =
        const { RefCell::new(Vec::new()) };
}

fn check_timer(window: &MainWindow) -> Rc<slint::Timer> {
    CHECKS.with(|checks| {
        let mut checks = checks.borrow_mut();
        checks.retain(|(owner, _)| owner.upgrade().is_some());
        let owned = checks.iter().find(|(owner, _)| {
            owner
                .upgrade()
                .is_some_and(|owner| std::ptr::eq(owner.window(), window.window()))
        });
        if let Some((_, timer)) = owned {
            return timer.clone();
        }
        let timer = Rc::new(slint::Timer::default());
        checks.push((window.as_weak(), timer.clone()));
        timer
    })
}

pub(crate) fn wake(
    window: &MainWindow,
    state: &Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let delay = match state.try_borrow() {
        Ok(p) if !p.settings_picker.busy() || p.operations.closing => return,
        Ok(p) if !p.settings_picker.cancellation_pending() => OPEN_CHECK,
        _ => CANCEL_RETRY,
    };
    let timer = check_timer(window);
    // Never postpone a due check; a pending cancellation pre-empts a slow one.
    if timer.running() && timer.interval() <= delay {
        return;
    }
    let (window, state) = (window.as_weak(), Rc::downgrade(state));
    timer.start(slint::TimerMode::SingleShot, delay, move || {
        check(&window, &state, &send);
    });
}

fn check(
    window: &slint::Weak<MainWindow>,
    state: &Weak<RefCell<Presenter>>,
    send: &Rc<dyn Fn(Command) -> bool>,
) {
    let (Some(w), Some(p)) = (window.upgrade(), state.upgrade()) else {
        return;
    };
    let changed = {
        let Ok(mut draft) = p.try_borrow_mut() else {
            wake(&w, &p, send.clone());
            return;
        };
        if draft.operations.closing {
            return;
        }
        let was_busy = draft.settings_picker.busy();
        draft.flush_settings_picker(send.as_ref());
        was_busy != draft.settings_picker.busy()
    };
    if changed {
        crate::refresh(&w, &p.borrow());
    }
    wake(&w, &p, send.clone());
}

#[cfg(test)]
#[path = "settings_picker_presentation_tests.rs"]
mod tests;
