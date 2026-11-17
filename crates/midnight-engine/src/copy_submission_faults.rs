use std::{cell::RefCell, sync::mpsc, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Point {
    AfterArmBeforeLock,
    AfterRegisterBeforePublish,
}

pub(super) enum Action {
    Panic,
    Pause {
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    },
}
struct Hook {
    point: Point,
    action: Action,
}
thread_local! { static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) }; }
struct Restore(Option<Hook>);
impl Drop for Restore {
    fn drop(&mut self) {
        let old = self.0.take();
        HOOK.with(|hook| {
            hook.replace(old);
        });
    }
}

/// One synchronous producer only; no global fault switch or cross-test state.
pub(super) fn with<R>(point: Point, action: Action, operation: impl FnOnce() -> R) -> R {
    let previous = HOOK.with(|hook| hook.replace(Some(Hook { point, action })));
    let _restore = Restore(previous);
    operation()
}

/// Parent calls this only behind cfg(test), at the two agreed ownership edges.
pub(super) fn at(point: Point) {
    let action = HOOK.with(|hook| {
        let mut hook = hook.borrow_mut();
        if hook.as_ref().is_some_and(|hook| hook.point == point) {
            hook.take().map(|hook| hook.action)
        } else {
            None
        }
    });
    // RefCell borrow is gone before either an unwind or a bounded pause.
    match action {
        Some(Action::Panic) => panic!("injected Copy submission fault"),
        Some(Action::Pause { entered, release }) => {
            entered.send(()).unwrap();
            release
                .recv_timeout(Duration::from_secs(10))
                .expect("test must release producer pause");
        }
        None => {}
    }
}
