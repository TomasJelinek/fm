//! A closed comparator window is released, with its session and any output left in its
//! mailbox, on the next turn of the event loop: never inside its own callback, and
//! without waiting for another comparator to open.
use super::identity_tests::{platform, runtime, text_service};
use super::*;
use std::sync::Arc;

fn appearance() -> ViewerAppearance {
    ViewerAppearance {
        family: "monospace".into(),
        font_size: 12.0,
        foreground: [240, 240, 240, 255],
        background: [24, 28, 36, 255],
        selection: [60, 80, 120, 255],
        selection_text: [0; 4],
        accent: [100, 150, 220, 255],
    }
}

/// Open a comparator without files (it asks for them) and return the window it added.
fn open(manager: &ComparatorWindows, service: &Service) -> Result<Rc<OpenWindow>, String> {
    let snapshot = service.comparator_snapshot();
    manager.open(
        FileCompareLaunch {
            sources: Vec::new(),
            disabled_plugins: Arc::from([]),
            comparators: snapshot.preferences.clone(),
            comparator_policy: snapshot.id,
        },
        appearance(),
        slint::LogicalSize::new(640.0, 400.0),
        Rc::new(|_, _| None),
    )?;
    Ok(manager.windows.borrow().last().unwrap().clone())
}

#[test]
fn a_closed_comparator_is_released_on_the_next_loop_turn() {
    let _native = platform();
    let runtime = runtime();
    let (service, _) = text_service(&runtime);
    let manager = ComparatorWindows::new_polled(service.clone());
    for close in ["Close button", "title bar"] {
        let open = open(&manager, &service).unwrap();
        let (window, released) = (open.window.as_weak(), Rc::downgrade(&open));
        drop(open);
        // The worker's chooser waits unread in the mailbox: no timer has run.
        std::thread::sleep(Duration::from_millis(100));
        let shown = window.upgrade().unwrap();
        match close {
            "Close button" => shown.invoke_close_comparator(),
            _ => shown
                .window()
                .dispatch_event(slint::platform::WindowEvent::CloseRequested),
        }
        drop(shown);
        // Still alive right after its own callback returned.
        assert!(
            released.upgrade().is_some(),
            "{close}: kept through its callback"
        );
        slint::platform::update_timers_and_animations();
        assert!(
            released.upgrade().is_none(),
            "{close}: the window is released"
        );
        assert!(
            window.upgrade().is_none(),
            "{close}: the component is released"
        );
        assert!(manager.windows.borrow().is_empty(), "{close}");
    }
    // The two-window limit counts open windows only.
    let first = open(&manager, &service).unwrap();
    let _second = open(&manager, &service).unwrap();
    assert!(open(&manager, &service).is_err(), "a third is refused");
    first.window.invoke_close_comparator();
    drop(first);
    slint::platform::update_timers_and_animations();
    assert_eq!(manager.windows.borrow().len(), 1);
    let _third = open(&manager, &service).unwrap();
    manager.close_all();
}
