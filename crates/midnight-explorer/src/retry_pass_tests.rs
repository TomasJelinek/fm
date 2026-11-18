//! The retry pass reports work while a send is refused, an owned scope is unavailable
//! or the main window is hidden, and nothing once everything was delivered.
use super::*;
use midnight_core::{DirectoryIndex, Entry, EntryId, EntryKind, Event, VisibleDirectory};
use midnight_ui::{DeferredDrop, OwnedCommandScope};
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::sync::Arc;

struct Windows;
impl Platform for Windows {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

/// An owned scope over the fixture queue that reports refusals as the explorer's does.
struct Scope {
    queue: Rc<Queue>,
}

#[derive(Default)]
struct Queue {
    refuse: Cell<bool>,
    unavailable: Cell<bool>,
    claimed: Cell<bool>,
    sent: RefCell<Vec<Command>>,
    retired: RefCell<Vec<DeferredDrop>>,
    refusals: Rc<Refusals>,
}

impl OwnedCommandScope for Scope {
    fn send(&self, command: Command) -> bool {
        if self.queue.refuse.get() {
            self.queue.refusals.refused();
            self.retire(Box::new(command));
            return false;
        }
        self.queue.sent.borrow_mut().push(command);
        true
    }

    fn retire(&self, value: DeferredDrop) {
        self.queue.retired.borrow_mut().push(value);
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        self.queue.claimed.set(false);
    }
}

struct Fixture {
    window: MainWindow,
    pass: RetryPass,
    queue: Rc<Queue>,
    closed: Rc<Cell<usize>>,
}

fn fixture() -> Fixture {
    slint::platform::set_platform(Box::new(Windows)).unwrap();
    let window = MainWindow::new().unwrap();
    let root = std::env::temp_dir().join("retry pass 文档");
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    let navigations = Rc::new(RefCell::new(Vec::new()));
    let recorded = navigations.clone();
    midnight_ui::install(
        &window,
        presenter.clone(),
        Rc::new(move |command| {
            if let Command::Navigate {
                pane, navigation, ..
            } = command
            {
                recorded.borrow_mut().push((pane, navigation));
            }
            true
        }),
    );
    for (pane, navigation) in navigations.take() {
        let entries: Arc<[Entry]> = [Entry::new(
            EntryId(1),
            root.join("file.txt"),
            EntryKind::File,
        )]
        .into();
        presenter.borrow_mut().apply(Event::ListingFinished {
            pane,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
    }
    window.show().unwrap();
    let queue = Rc::new(Queue::default());
    let claims = queue.clone();
    let begin: BeginOwned = Rc::new(move || {
        if claims.unavailable.get() || claims.claimed.replace(true) {
            if claims.unavailable.get() {
                claims.refusals.refused();
            }
            return None;
        }
        Some(Box::new(Scope {
            queue: claims.clone(),
        }))
    });
    let closed = Rc::new(Cell::new(0));
    let counted = closed.clone();
    let pass = RetryPass {
        presenter,
        controls: Rc::new(|_| true),
        begin_owned: begin,
        window: window.as_weak(),
        close_viewers: Box::new(move || counted.set(counted.get() + 1)),
        preparation: Weak::new(),
        visible: Rc::default(),
        refusals: queue.refusals.clone(),
    };
    Fixture {
        window,
        pass,
        queue,
        closed,
    }
}

fn acknowledged(queue: &Queue) -> usize {
    queue
        .sent
        .borrow()
        .iter()
        .filter(|command| matches!(command, Command::PaneCommitted(VisibleDirectory { .. })))
        .count()
}

/// Run passes until one reports no work. A pass that sent something reports work once
/// more, so a few passes settle any fixture; more means the pass never goes quiet.
fn settle(pass: &RetryPass) -> usize {
    let passes = (1..=5).find(|_| !pass.run());
    passes.expect("the retry pass never went quiet")
}

#[test]
fn a_refused_acknowledgement_keeps_the_pass_running_until_it_is_accepted() {
    let fixture = fixture();
    fixture.queue.refuse.set(true);
    for _ in 0..5 {
        assert!(fixture.pass.run(), "refused sends leave work");
    }
    assert_eq!(acknowledged(&fixture.queue), 0);
    fixture.queue.refuse.set(false);
    settle(&fixture.pass);
    assert_eq!(acknowledged(&fixture.queue), 2);
    assert!(
        !fixture.pass.run(),
        "delivered acknowledgements leave no work"
    );
    assert_eq!(
        acknowledged(&fixture.queue),
        2,
        "each listing is acknowledged once"
    );
    assert_eq!(fixture.closed.get(), 0);
}

#[test]
fn an_unavailable_owned_scope_keeps_the_pass_running() {
    let fixture = fixture();
    fixture.queue.unavailable.set(true);
    for _ in 0..5 {
        assert!(fixture.pass.run());
    }
    assert_eq!(acknowledged(&fixture.queue), 0);
    fixture.queue.unavailable.set(false);
    settle(&fixture.pass);
    assert_eq!(acknowledged(&fixture.queue), 2);
}

#[test]
fn a_hidden_main_window_closes_viewers_and_keeps_the_pass_running() {
    let fixture = fixture();
    settle(&fixture.pass);
    assert_eq!(fixture.closed.get(), 0);
    fixture.window.hide().unwrap();
    assert!(fixture.pass.run());
    assert!(fixture.pass.run());
    assert_eq!(fixture.closed.get(), 2);
}
