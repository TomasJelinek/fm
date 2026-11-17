use super::*;
use std::{
    cell::Cell,
    path::PathBuf,
    rc::Weak,
    sync::{Arc, Mutex},
    thread::ThreadId,
};

struct Scope {
    sends: Rc<Cell<usize>>,
    finished: Rc<Cell<usize>>,
}

impl OwnedCommandScope for Scope {
    fn send(&self, _command: Command) -> bool {
        self.sends.set(self.sends.get() + 1);
        true
    }

    fn retire(&self, _value: DeferredDrop) {
        // This fixture is for metadata-only actions. RetainingScope below covers
        // Presenter retirement; adapter ownership tests remain separate.
        panic!("unexpected retired payload in metadata-only fixture");
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        self.finished.set(self.finished.get() + 1);
    }
}

fn presenter() -> Rc<RefCell<Presenter>> {
    let mut presenter = Presenter::new(PathBuf::from("left"), PathBuf::from("right"));
    presenter.require_owned_command_scope();
    Rc::new(RefCell::new(presenter))
}

fn factory(sends: Rc<Cell<usize>>, finished: Rc<Cell<usize>>) -> BeginOwned {
    Rc::new(move || {
        Some(Box::new(Scope {
            sends: sends.clone(),
            finished: finished.clone(),
        }))
    })
}

#[test]
fn blocked_claim_does_not_invoke_action_or_construct_a_request() {
    let state = presenter();
    let attempts = Cell::new(0);
    let blocked: BeginOwned = Rc::new(|| None);
    for _ in 0..16 {
        assert!(
            with_owned_commands(&state, &blocked, |_, _| {
                attempts.set(attempts.get() + 1);
                Command::Shutdown
            })
            .is_none()
        );
    }
    assert_eq!(attempts.get(), 0);
    assert!(!state.borrow().owned_command_scope_available());
}

#[test]
fn blocked_claim_leaves_the_backend_event_in_its_bounded_channel() {
    let state = presenter();
    let (sender, mut events) = tokio::sync::mpsc::channel(1);
    sender.try_send(Command::Shutdown).unwrap();
    let blocked: BeginOwned = Rc::new(|| None);
    for _ in 0..16 {
        assert!(with_owned_commands(&state, &blocked, |_, _| events.try_recv().ok()).is_none());
    }
    assert_eq!(events.len(), 1);
    let begin = factory(Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    assert!(matches!(
        with_owned_commands(&state, &begin, |_, _| events.try_recv().ok()),
        Some(Some(Command::Shutdown))
    ));
    assert!(events.is_empty());
}

#[test]
fn factory_precedes_borrow_and_sender_uses_the_existing_borrow() {
    let state = presenter();
    let sends = Rc::new(Cell::new(0));
    let finished = Rc::new(Cell::new(0));
    let make_scope = factory(sends.clone(), finished.clone());
    let check_state = state.clone();
    let begin: BeginOwned = Rc::new(move || {
        assert!(check_state.try_borrow_mut().is_ok());
        make_scope()
    });
    assert_eq!(
        with_owned_commands(&state, &begin, |p, send| {
            assert!(p.owned_command_scope_available());
            assert!(state.try_borrow_mut().is_err());
            assert!(send(Command::Shutdown));
            7
        }),
        Some(7)
    );
    assert_eq!(sends.get(), 1);
    assert_eq!(finished.get(), 1);
    assert!(!state.borrow().owned_command_scope_available());
}

#[test]
fn reentrant_borrow_does_not_execute_owned_action_and_releases_empty_claim() {
    let state = presenter();
    let sends = Rc::new(Cell::new(0));
    let finished = Rc::new(Cell::new(0));
    let begin = factory(sends, finished.clone());
    let _borrow = state.borrow_mut();
    assert!(with_owned_commands(&state, &begin, |_, _| panic!("reentered")).is_none());
    assert_eq!(finished.get(), 1);
}

#[test]
fn full_reclamation_does_not_gate_provider_free_controls() {
    let state = presenter();
    let blocked: BeginOwned = Rc::new(|| None);
    let sends = Cell::new(0);
    let send = |command| {
        assert!(matches!(command, Command::Shutdown));
        sends.set(sends.get() + 1);
        true
    };
    assert_eq!(
        with_control_commands(&state, &blocked, &send, |p, send| {
            assert!(!p.owned_command_scope_available());
            send(Command::Shutdown)
        }),
        Some(true)
    );
    assert_eq!(sends.get(), 1);
}

#[test]
fn unwind_restores_scope_state_and_releases_the_claim() {
    let state = presenter();
    let finished = Rc::new(Cell::new(0));
    let begin = factory(Rc::new(Cell::new(0)), finished.clone());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_owned_commands(&state, &begin, |p, _| {
            assert!(p.owned_command_scope_available());
            panic!("finite action failed")
        })
    }));
    assert!(result.is_err());
    assert_eq!(finished.get(), 1);
    assert!(!state.borrow().owned_command_scope_available());
}

struct ObservedRetirement {
    _value: DeferredDrop,
    drops: Arc<Mutex<Vec<ThreadId>>>,
}

impl Drop for ObservedRetirement {
    fn drop(&mut self) {
        self.drops.lock().unwrap().push(std::thread::current().id());
    }
}

struct RetainingScope {
    // Test observation only: a successful borrow here would reveal that the
    // helper released its original mutable Presenter reference before retiring.
    presenter: Weak<RefCell<Presenter>>,
    values: RefCell<Vec<DeferredDrop>>,
    committed: Rc<RefCell<Option<Vec<DeferredDrop>>>>,
    received: Rc<Cell<usize>>,
    finished: Rc<Cell<usize>>,
    drops: Arc<Mutex<Vec<ThreadId>>>,
}

impl OwnedCommandScope for RetainingScope {
    fn send(&self, _command: Command) -> bool {
        panic!("this fixture exercises displaced Presenter values only")
    }

    fn retire(&self, value: DeferredDrop) {
        assert!(self.presenter.upgrade().unwrap().try_borrow_mut().is_err());
        self.received.set(self.received.get() + 1);
        self.values.borrow_mut().push(Box::new(ObservedRetirement {
            _value: value,
            drops: self.drops.clone(),
        }));
    }
}

impl Drop for RetainingScope {
    fn drop(&mut self) {
        *self.committed.borrow_mut() = Some(std::mem::take(self.values.get_mut()));
        self.finished.set(self.finished.get() + 1);
    }
}

fn assert_presenter_retirement_handoff(unwind: bool) {
    let caller = std::thread::current().id();
    let state = presenter();
    let begun = Rc::new(Cell::new(0));
    let finished = Rc::new(Cell::new(0));
    let received = Rc::new(Cell::new(0));
    let committed = Rc::new(RefCell::new(None));
    let drops = Arc::new(Mutex::new(Vec::new()));
    let begin: BeginOwned = Rc::new({
        let presenter = Rc::downgrade(&state);
        let begun = begun.clone();
        let finished = finished.clone();
        let received = received.clone();
        let committed = committed.clone();
        let drops = drops.clone();
        move || {
            begun.set(begun.get() + 1);
            Some(Box::new(RetainingScope {
                presenter: presenter.clone(),
                values: RefCell::new(Vec::new()),
                committed: committed.clone(),
                received: received.clone(),
                finished: finished.clone(),
                drops: drops.clone(),
            }))
        }
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_owned_commands(&state, &begin, |p, _| {
            // Use the real public detach operation, not a test-only retired list.
            p.retire_all();
            if unwind {
                panic!("finite action failed after detaching Presenter state");
            }
            7
        })
    }));
    if unwind {
        assert!(result.is_err());
    } else {
        assert_eq!(result.unwrap(), Some(7));
    }
    assert_eq!(begun.get(), 1);
    assert_eq!(finished.get(), 1);
    assert!(received.get() > 0);
    assert!(!state.borrow().owned_command_scope_available());
    assert!(state.borrow_mut().take_retired().is_empty());
    assert!(drops.lock().unwrap().is_empty());
    let values = committed.borrow_mut().take().unwrap();
    assert_eq!(values.len(), received.get());
    let worker = std::thread::spawn(move || {
        drop(values);
        std::thread::current().id()
    })
    .join()
    .unwrap();
    assert_ne!(worker, caller);
    let dropped = drops.lock().unwrap();
    assert_eq!(dropped.len(), received.get());
    assert!(dropped.iter().all(|thread| *thread == worker));
}

#[test]
fn normal_return_retires_presenter_values_into_the_same_scope_before_releasing_its_borrow() {
    assert_presenter_retirement_handoff(false);
}

#[test]
fn unwind_retires_presenter_values_into_the_same_scope_before_releasing_its_borrow() {
    assert_presenter_retirement_handoff(true);
}
