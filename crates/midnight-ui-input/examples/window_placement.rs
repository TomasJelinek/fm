//! Native placement check; run on an interactive desktop with its normal backend.
//!
//! Opens a parent, then a child placed over it as viewers and comparators are, twice
//! (the second child uses any offset learned from the first), then both maximized.
//! It compares the native client origins and sizes, which are exact on X11 and
//! Windows. Wayland reports no origins: only sizes and states are compared there,
//! and the compositor chooses the child's position.
use midnight_ui_input::WindowPlacement;
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::{cell::RefCell, rc::Rc, time::Duration};

slint::slint! {
    export component ProbeWindow inherits Window {
        in property <string> caption;
        title: caption;
        min-width: 320px; min-height: 200px;
        background: #19324c;
        Text { text: root.caption; color: white; horizontal-alignment: center; vertical-alignment: center; }
    }
}

/// Client origin (None on Wayland), client size and maximized state.
type Bounds = (Option<(i32, i32)>, (u32, u32), bool);

fn bounds(window: &slint::Window) -> Option<Bounds> {
    window.with_winit_window(|native| {
        let origin = native
            .inner_position()
            .ok()
            .map(|origin| (origin.x, origin.y));
        let size = native.inner_size();
        (origin, (size.width, size.height), native.is_maximized())
    })
}

/// Settle time for the window manager's placement and the child's one alignment move.
const SETTLE: Duration = Duration::from_millis(1500);

fn check(
    label: &'static str,
    parent: slint::Weak<ProbeWindow>,
    failure: Rc<RefCell<Vec<String>>>,
    maximized: bool,
    then: impl FnOnce() + 'static,
) {
    let parent_window = parent.upgrade().unwrap();
    let child = ProbeWindow::new().unwrap();
    child.set_caption(format!("{label} child: should cover the parent exactly").into());
    WindowPlacement::capture(parent_window.window()).apply(child.window());
    child.show().unwrap();
    slint::Timer::single_shot(SETTLE, move || {
        let parent = parent.upgrade().unwrap();
        let expected = bounds(parent.window());
        let observed = bounds(child.window());
        println!("{label}: parent={expected:?} child={observed:?}");
        if expected.is_none()
            || expected != observed
            || expected.is_some_and(|bounds| bounds.2 != maximized)
        {
            failure.borrow_mut().push(format!(
                "{label}: child {observed:?} differs from parent {expected:?}"
            ));
        }
        child.hide().unwrap();
        then();
    });
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let parent = ProbeWindow::new()?;
    parent.set_caption("Midnight Explorer placement verification".into());
    parent
        .window()
        .set_position(slint::PhysicalPosition::new(180, 150));
    parent.window().set_size(slint::PhysicalSize::new(720, 460));
    parent.show()?;
    let failure = Rc::new(RefCell::new(Vec::new()));
    let errors = failure.clone();
    let weak = parent.as_weak();
    slint::Timer::single_shot(Duration::from_millis(500), move || {
        let second = weak.clone();
        let failures = failure.clone();
        check("first", weak, failure, false, move || {
            let third = second.clone();
            check("second", second, failures.clone(), false, move || {
                third.upgrade().unwrap().window().set_maximized(true);
                slint::Timer::single_shot(Duration::from_millis(500), move || {
                    let last = third.clone();
                    check("maximized", third, failures, true, move || {
                        last.upgrade().unwrap().hide().unwrap();
                        slint::quit_event_loop().unwrap();
                    });
                });
            });
        });
    });
    slint::run_event_loop()?;
    let errors = errors.borrow();
    if errors.is_empty() {
        println!("Placement matches.");
        Ok(())
    } else {
        Err(errors.join("\n").into())
    }
}
