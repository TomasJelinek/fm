//! Unit-only controls at real native commit/recovery boundaries.
use std::{cell::RefCell, path::Path};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Boundary {
    BeforeCapture,
    AfterCapture,
    BeforePublish,
    BeforeRestoreValidation,
    AfterCommit,
    BeforeCleanup,
}
type Action = Box<dyn FnOnce(&Path)>;
#[derive(Default)]
struct Hooks {
    actions: Vec<(Boundary, Action)>,
    fail_sync: bool,
    fail_rename: bool,
}
thread_local! {
    static HOOKS: RefCell<Hooks> = RefCell::default();
}
pub(crate) struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        HOOKS.with(|hooks| *hooks.borrow_mut() = Hooks::default());
    }
}
pub(crate) fn reset() -> Reset {
    HOOKS.with(|hooks| *hooks.borrow_mut() = Hooks::default());
    Reset
}
pub(crate) fn at(boundary: Boundary, action: impl FnOnce(&Path) + 'static) {
    HOOKS.with(|hooks| {
        hooks
            .borrow_mut()
            .actions
            .push((boundary, Box::new(action)))
    });
}
pub(crate) fn fire(boundary: Boundary, stage: &Path) {
    let action = HOOKS.with(|hooks| {
        let mut hooks = hooks.borrow_mut();
        let index = hooks
            .actions
            .iter()
            .position(|(point, _)| *point == boundary)?;
        Some(hooks.actions.remove(index).1)
    });
    if let Some(action) = action {
        action(stage);
    }
}
pub(crate) fn fail_next_sync() {
    HOOKS.with(|hooks| hooks.borrow_mut().fail_sync = true);
}
pub(crate) fn take_sync_failure() -> bool {
    HOOKS.with(|hooks| std::mem::take(&mut hooks.borrow_mut().fail_sync))
}
pub(crate) fn fail_after_next_rename() {
    HOOKS.with(|hooks| hooks.borrow_mut().fail_rename = true);
}
pub(crate) fn take_rename_failure() -> bool {
    HOOKS.with(|hooks| std::mem::take(&mut hooks.borrow_mut().fail_rename))
}
