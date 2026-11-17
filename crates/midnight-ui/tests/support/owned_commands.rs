//! Explicit ownership scopes for boolean UI fixtures, not Engine admission tests.
//!
//! The authored boolean sender keeps its existing acceptance/refusal behavior. A
//! retained test-only clone protects shared payload ownership before invoking it,
//! including an immediately completed or refused send. A bool cannot return the
//! original command: real Full/Closed custody and the fixed four-queued-plus-one-
//! pending bound belong to CommandReclaim tests.
//!
//! This fixture substitutes one shared collection for finite test-action retirements.
//! It releases that collection on one background thread at final fixture teardown,
//! without a runtime, production queue or task per callback/refusal.
use midnight_core::Command;
use midnight_ui::{BeginOwned, DeferredDrop, MainWindow, OwnedCommandScope, Presenter};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Default)]
struct FixtureOwner {
    claimed: Cell<bool>,
    retired: RefCell<Vec<DeferredDrop>>,
}

impl Drop for FixtureOwner {
    fn drop(&mut self) {
        let retired = std::mem::take(self.retired.get_mut());
        if retired.is_empty() {
            return;
        }
        let worker = std::thread::spawn(move || drop(retired));
        if let Err(panic) = worker.join()
            && !std::thread::panicking()
        {
            std::panic::resume_unwind(panic);
        }
    }
}

struct FixtureScope {
    owner: Rc<FixtureOwner>,
    send: Rc<dyn Fn(Command) -> bool>,
}

impl OwnedCommandScope for FixtureScope {
    fn send(&self, command: Command) -> bool {
        self.retire(Box::new(command.clone()));
        (self.send)(command)
    }

    fn retire(&self, value: DeferredDrop) {
        self.owner.retired.borrow_mut().push(value);
    }
}

impl Drop for FixtureScope {
    fn drop(&mut self) {
        self.owner.claimed.set(false);
    }
}

/// Return the same factory installed on the window for explicit backend-event calls.
pub fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) -> BeginOwned {
    let owner = Rc::new(FixtureOwner::default());
    let action_send = send.clone();
    let begin: BeginOwned = Rc::new(move || {
        if owner.claimed.replace(true) {
            return None;
        }
        Some(Box::new(FixtureScope {
            owner: owner.clone(),
            send: action_send.clone(),
        }))
    });
    midnight_ui::install_with_owned(window, presenter, send, begin.clone());
    begin
}

/// Scope a direct fixture event/action just as an installed owned callback does.
pub fn run<R>(
    presenter: &Rc<RefCell<Presenter>>,
    begin: &BeginOwned,
    action: impl FnOnce(&mut Presenter, &dyn Fn(Command) -> bool) -> R,
) -> R {
    midnight_ui::with_owned_commands(presenter, begin, action)
        .expect("a direct fixture action must obtain its non-reentrant scope")
}
