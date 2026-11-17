//! Acquire reclamation ownership before a callback can acquire captured native values.
use crate::{DeferredDrop, Presenter};
use midnight_core::Command;
use std::{cell::RefCell, cell::RefMut, rc::Rc};

/// A claim on the host's existing bounded retirement batch for one finite UI action.
///
/// Implementations must retain refused commands and displaced values without
/// destroying their payloads on the UI thread. They must never borrow Presenter.
pub trait OwnedCommandScope {
    /// Publish a command, retaining its exact original value on refusal.
    fn send(&self, command: Command) -> bool;

    /// Transfer a value displaced by this action to the claimed retirement batch.
    fn retire(&self, value: DeferredDrop);
}

/// Claims reclamation ownership without waiting, before constructing an owned action.
/// `None` leaves the draft, capture or queued event in its current owner slot.
pub type BeginOwned = Rc<dyn Fn() -> Option<Box<dyn OwnedCommandScope>>>;

struct ActivePresenter<'a> {
    presenter: RefMut<'a, Presenter>,
    scope: &'a dyn OwnedCommandScope,
    previous_active: bool,
}

impl Drop for ActivePresenter<'_> {
    fn drop(&mut self) {
        // This is the already-held borrow, including during unwinding. The
        // sender/scope does not reenter Presenter to recover a refused payload.
        self.presenter
            .set_owned_command_scope_active(self.previous_active);
        for value in self.presenter.take_retired() {
            self.scope.retire(value);
        }
    }
}

fn run_owned<R>(
    presenter: &Rc<RefCell<Presenter>>,
    scope: &dyn OwnedCommandScope,
    action: impl FnOnce(&mut Presenter, &dyn Fn(Command) -> bool) -> R,
) -> Option<R> {
    let mut presenter = presenter.try_borrow_mut().ok()?;
    let previous_active = presenter.set_owned_command_scope_active(true);
    let mut active = ActivePresenter {
        presenter,
        scope,
        previous_active,
    };
    active.presenter.flush_deferred_copy_draft();
    active.presenter.flush_deferred_drag_owners();
    Some(action(&mut active.presenter, &|command| {
        scope.send(command)
    }))
}

/// Execute one finite ownership-bearing action with a reserved reclamation batch.
///
/// The factory runs before the Presenter borrow and before `action`. Put event
/// dequeue, request builders and capture clones inside `action`; on `None` none
/// of those operations run. The scope is released before this function returns.
/// Return metadata only; displaced captured owners must go through retirement.
pub fn with_owned_commands<R>(
    presenter: &Rc<RefCell<Presenter>>,
    begin: &BeginOwned,
    action: impl FnOnce(&mut Presenter, &dyn Fn(Command) -> bool) -> R,
) -> Option<R> {
    let scope = begin()?;
    run_owned(presenter, scope.as_ref(), action)
}

/// Controls may still run when reclamation is full. Captured-owner methods check
/// owned_command_scope_available before taking/cloning their existing owner slot.
pub(crate) fn with_control_commands<R>(
    presenter: &Rc<RefCell<Presenter>>,
    begin: &BeginOwned,
    control_send: &dyn Fn(Command) -> bool,
    action: impl FnOnce(&mut Presenter, &dyn Fn(Command) -> bool) -> R,
) -> Option<R> {
    if let Some(scope) = begin() {
        run_owned(presenter, scope.as_ref(), action)
    } else {
        let mut presenter = presenter.try_borrow_mut().ok()?;
        Some(action(&mut presenter, control_send))
    }
}

#[cfg(test)]
#[path = "owned_commands_tests.rs"]
mod tests;
