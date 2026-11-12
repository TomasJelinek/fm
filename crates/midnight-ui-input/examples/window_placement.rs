//! Native placement check; run on an interactive desktop with its normal backend.
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

fn bounds(window: &slint::Window) -> Option<(i32, i32, u32, u32)> {
    window
        .with_winit_window(|native| {
            let position = native.outer_position().ok()?;
            let size = native.inner_size();
            Some((position.x, position.y, size.width, size.height))
        })
        .flatten()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let parent = ProbeWindow::new()?;
    parent.set_caption("Midnight Explorer placement verification".into());
    parent
        .window()
        .set_position(slint::PhysicalPosition::new(180, 150));
    parent.window().set_size(slint::PhysicalSize::new(720, 460));
    parent.show()?;
    let child = ProbeWindow::new()?;
    child.set_caption("Viewer matches main window".into());
    let error = Rc::new(RefCell::new(None));
    let failure = error.clone();
    let first = parent.as_weak();
    let second = child.as_weak();
    slint::Timer::single_shot(Duration::from_millis(250), move || {
        let parent = first.upgrade().unwrap();
        let child = second.upgrade().unwrap();
        WindowPlacement::capture(parent.window()).apply(child.window());
        child.show().unwrap();
        slint::Timer::single_shot(Duration::from_millis(250), move || {
            let expected = bounds(parent.window());
            let observed = bounds(child.window());
            println!("normal parent={expected:?} child={observed:?}");
            if expected.is_none() || expected != observed {
                *failure.borrow_mut() = Some("Native child bounds differ from parent");
            }
            child.hide().unwrap();
            parent.window().set_maximized(true);
            slint::Timer::single_shot(Duration::from_millis(250), move || {
                WindowPlacement::capture(parent.window()).apply(child.window());
                child.show().unwrap();
                slint::Timer::single_shot(Duration::from_millis(250), move || {
                    let expected = bounds(parent.window());
                    let observed = bounds(child.window());
                    println!("maximized parent={expected:?} child={observed:?}");
                    if expected.is_none() || expected != observed || !child.window().is_maximized()
                    {
                        *failure.borrow_mut() =
                            Some("Native maximized child bounds differ from parent");
                    }
                    child.hide().unwrap();
                    parent.hide().unwrap();
                    slint::quit_event_loop().unwrap();
                });
            });
        });
    });
    slint::run_event_loop()?;
    if let Some(error) = *error.borrow() {
        return Err(error.into());
    }
    Ok(())
}
