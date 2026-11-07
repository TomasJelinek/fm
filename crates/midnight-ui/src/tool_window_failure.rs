//! Suppress failed native host creation; retry only bounded-rate backend release.
use crate::{MainWindow, Presenter};
use midnight_core::Command;
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    hash::Hash,
    rc::{Rc, Weak},
    time::Duration,
};

type Close<Id> = fn(&mut Presenter, Id, &dyn Fn(Command) -> bool) -> bool;

pub(crate) struct Failures<Id> {
    ids: Rc<RefCell<HashSet<Id>>>,
    context: Rc<Context<Id>>,
}
impl<Id> Clone for Failures<Id> {
    fn clone(&self) -> Self {
        Self {
            ids: self.ids.clone(),
            context: self.context.clone(),
        }
    }
}
impl<Id: Copy + Eq + Hash + 'static> Failures<Id> {
    pub(crate) fn new(
        main: &MainWindow,
        state: &Rc<RefCell<Presenter>>,
        send: &Rc<dyn Fn(Command) -> bool>,
        closing: &Rc<Cell<bool>>,
        title: &'static str,
        close: Close<Id>,
    ) -> Self {
        Self {
            ids: Rc::default(),
            context: Rc::new(Context {
                title,
                main: main.as_weak(),
                state: Rc::downgrade(state),
                send: send.clone(),
                closing: Rc::downgrade(closing),
                close,
            }),
        }
    }
    pub(crate) fn contains(&self, id: Id) -> bool {
        self.ids.borrow().contains(&id)
    }

    pub(crate) fn record(&self, id: Id, message: String) {
        if !self.ids.borrow_mut().insert(id) {
            return;
        }
        Rc::new(Retry {
            ids: Rc::downgrade(&self.ids),
            id,
            message: RefCell::new(Some(message)),
            context: self.context.clone(),
        })
        .schedule(Duration::ZERO);
    }
}
struct Retry<Id> {
    ids: Weak<RefCell<HashSet<Id>>>,
    id: Id,
    message: RefCell<Option<String>>,
    context: Rc<Context<Id>>,
}
struct Context<Id> {
    title: &'static str,
    main: slint::Weak<MainWindow>,
    state: Weak<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
    closing: Weak<Cell<bool>>,
    close: Close<Id>,
}
impl<Id: Copy + Eq + Hash + 'static> Retry<Id> {
    fn schedule(self: Rc<Self>, delay: Duration) {
        slint::Timer::single_shot(delay, move || self.attempt());
    }
    fn attempt(self: Rc<Self>) {
        let Some(ids) = self.ids.upgrade() else {
            return;
        };
        let (Some(main), Some(state)) = (self.context.main.upgrade(), self.context.state.upgrade())
        else {
            ids.borrow_mut().remove(&self.id);
            return;
        };
        if self
            .context
            .closing
            .upgrade()
            .is_none_or(|closing| closing.get())
        {
            ids.borrow_mut().remove(&self.id);
            return;
        }
        let Ok(mut p) = state.try_borrow_mut() else {
            self.schedule(Duration::from_millis(250));
            return;
        };
        if let Some(message) = self.message.borrow_mut().take() {
            p.desktop.report(self.context.title, message);
        }
        let closed = (self.context.close)(&mut p, self.id, self.context.send.as_ref());
        drop(p);
        if closed {
            ids.borrow_mut().remove(&self.id);
        } else {
            self.clone().schedule(Duration::from_millis(250));
        }
        // Both callbacks may synchronously evaluate model properties.
        crate::refresh(&main, &state.borrow());
    }
}
