//! Non-consuming observers of events after Slint updates a window.
use slint::{PlatformError, Window, platform::WindowEvent};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

type Callback = dyn Fn(&Window, &WindowEvent);

struct Observer {
    active: Cell<bool>,
    callback: Rc<Callback>,
}

#[derive(Default)]
struct Registry {
    installed: bool,
    observers: Vec<Weak<Observer>>,
}

thread_local! { static REGISTRY: RefCell<Registry> = RefCell::default(); }

/// Keeps a UI-thread event observer registered until dropped.
///
/// Observers can register or remove observers during a callback. Removing one
/// prevents further calls, including later calls in the current event dispatch.
/// Capture component weak handles in callbacks to avoid retaining their windows.
#[must_use = "dropping the guard unregisters the event observer"]
pub struct PostWindowEventObserver {
    observer: Rc<Observer>,
}

impl Drop for PostWindowEventObserver {
    fn drop(&mut self) {
        self.observer.active.set(false);
        let _ = REGISTRY.try_with(|registry| {
            registry.borrow_mut().observers.retain(|observer| {
                observer
                    .upgrade()
                    .is_some_and(|observer| observer.active.get())
            });
        });
    }
}

/// Observe every window event on this UI thread after Slint has handled it.
///
/// This installs one hook, preserves any earlier hook, and never consumes input.
/// The caller filters the target window and owns the registration guard. A Slint
/// platform must already be installed. New observers start at the next event;
/// callbacks run without borrowing the observer registry.
pub fn observe_post_window_events(
    observer: impl Fn(&Window, &WindowEvent) + 'static,
) -> Result<PostWindowEventObserver, PlatformError> {
    if !REGISTRY.with(|registry| registry.borrow().installed) {
        let previous = i_slint_core::context::set_window_event_hook(None)?;
        i_slint_core::context::set_window_event_hook(Some(Box::new(
            move |adapter, event, result| {
                let observers = REGISTRY.with(|registry| {
                    let mut registry = registry.borrow_mut();
                    let mut observers = Vec::with_capacity(registry.observers.len());
                    registry.observers.retain(|observer| {
                        if let Some(observer) = observer.upgrade()
                            && observer.active.get()
                        {
                            observers.push(observer);
                            true
                        } else {
                            false
                        }
                    });
                    observers
                });
                for observer in observers {
                    if observer.active.get() {
                        (observer.callback)(adapter.window(), event);
                    }
                }
                if let Some(previous) = previous.as_ref() {
                    previous(adapter, event, result);
                }
            },
        )))?;
        REGISTRY.with(|registry| registry.borrow_mut().installed = true);
    }
    let observer = Rc::new(Observer {
        active: Cell::new(true),
        callback: Rc::new(observer),
    });
    REGISTRY.with(|registry| {
        registry
            .borrow_mut()
            .observers
            .push(Rc::downgrade(&observer))
    });
    Ok(PostWindowEventObserver { observer })
}
