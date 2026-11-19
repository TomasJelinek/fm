use i_slint_core::{
    items::TooltipArea,
    platform::WindowEventDispatchResult,
    window::{WindowInner, WindowKind},
};
use midnight_ui_input::{
    PostWindowEventObserver, observe_post_window_events, synchronize_tooltip_anchors,
};
use slint::{
    ComponentHandle, LogicalPosition,
    platform::{
        Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

slint::slint! {
    export component TestWindow inherits Window {
        width: 240px; height: 160px;
        Rectangle {
            x: 40px; y: 30px; width: 100px; height: 50px;
            Tooltip { Rectangle { width: 80px; height: 20px; background: blue; } }
        }
    }
}

type Natives = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;

struct TestPlatform {
    natives: Natives,
    time: Rc<Cell<Duration>>,
}

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        self.natives.borrow_mut().push(native.clone());
        Ok(native)
    }

    fn duration_since_start(&self) -> Duration {
        self.time.get()
    }
}

fn setup() -> (Natives, Rc<Cell<Duration>>) {
    let natives = Natives::default();
    let time = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform {
        natives: natives.clone(),
        time: time.clone(),
    }))
    .unwrap();
    (natives, time)
}

#[test]
fn observers_see_updated_window_state_chain_previous_hook_and_unregister_on_drop() {
    setup();
    let window = TestWindow::new().unwrap();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let out = calls.clone();
    i_slint_core::context::set_window_event_hook(Some(Box::new(move |_, event, result| {
        if matches!(event, WindowEvent::ScaleFactorChanged { .. }) {
            assert!(matches!(result, WindowEventDispatchResult::Accepted));
            out.borrow_mut().push("previous");
        }
    })))
    .unwrap();
    let out = calls.clone();
    let weak = window.as_weak();
    let first = observe_post_window_events(move |host, event| {
        if let WindowEvent::ScaleFactorChanged { scale_factor } = event {
            assert!(std::ptr::eq(host, weak.upgrade().unwrap().window()));
            assert_eq!(host.scale_factor(), *scale_factor);
            out.borrow_mut().push("first");
        }
    })
    .unwrap();
    let out = calls.clone();
    let second = observe_post_window_events(move |_, event| {
        if matches!(event, WindowEvent::ScaleFactorChanged { .. }) {
            out.borrow_mut().push("second");
        }
    })
    .unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.5 });
    assert_eq!(*calls.borrow(), ["first", "second", "previous"]);
    calls.borrow_mut().clear();
    drop(first);
    window
        .window()
        .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
    assert_eq!(*calls.borrow(), ["second", "previous"]);
    calls.borrow_mut().clear();
    drop(second);
    window
        .window()
        .dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
    assert_eq!(*calls.borrow(), ["previous"]);
}

#[test]
fn callbacks_can_remove_pending_observers_register_and_dispatch_reentrantly() {
    setup();
    let window = TestWindow::new().unwrap();
    let pending = Rc::new(RefCell::new(None::<PostWindowEventObserver>));
    let added = Rc::new(RefCell::new(None::<PostWindowEventObserver>));
    let pending_calls = Rc::new(Cell::new(0));
    let added_calls = Rc::new(Cell::new(0));
    let handled = Cell::new(false);
    let pending_out = pending.clone();
    let added_out = added.clone();
    let calls = added_calls.clone();
    let _first = observe_post_window_events(move |host, event| {
        if matches!(event, WindowEvent::PointerExited) && !handled.replace(true) {
            drop(pending_out.borrow_mut().take());
            let calls = calls.clone();
            *added_out.borrow_mut() =
                Some(observe_post_window_events(move |_, _| calls.set(calls.get() + 1)).unwrap());
            host.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.5 });
        }
    })
    .unwrap();
    let calls = pending_calls.clone();
    *pending.borrow_mut() =
        Some(observe_post_window_events(move |_, _| calls.set(calls.get() + 1)).unwrap());
    window.window().dispatch_event(WindowEvent::PointerExited);
    assert_eq!(
        pending_calls.get(),
        0,
        "a dropped snapshot entry must not run"
    );
    assert_eq!(
        added_calls.get(),
        1,
        "new observer sees the nested event only"
    );
    window.window().dispatch_event(WindowEvent::PointerExited);
    assert_eq!(added_calls.get(), 2);
    drop(added.borrow_mut().take());
    window.window().dispatch_event(WindowEvent::PointerExited);
    assert_eq!(added_calls.get(), 2);
}

#[test]
fn weak_callbacks_do_not_retain_components_and_guard_drop_releases_the_callback() {
    setup();
    let window = TestWindow::new().unwrap();
    let weak = window.as_weak();
    let observed = weak.clone();
    let token = Rc::new(());
    let released = Rc::downgrade(&token);
    let guard = observe_post_window_events(move |_, _| {
        let _token = &token;
        let _ = observed.upgrade();
    })
    .unwrap();
    drop(window);
    assert!(
        weak.upgrade().is_none(),
        "registry must not retain the component"
    );
    assert!(released.upgrade().is_some());
    drop(guard);
    assert!(
        released.upgrade().is_none(),
        "registry must not retain callbacks"
    );
}

#[test]
fn tooltip_anchor_updates_use_host_coordinates_and_ignore_negative_cursors() {
    let (natives, time) = setup();
    let window = TestWindow::new().unwrap();
    window.show().unwrap();
    let native = natives.borrow()[0].clone();
    native.set_size(slint::PhysicalSize::new(240, 160));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(240.0, 160.0),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); 240 * 160];
    let mut draw = || {
        slint::platform::update_timers_and_animations();
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 240);
        });
    };
    draw();
    window.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(70.0, 55.0),
    });
    draw();
    time.set(Duration::from_millis(600));
    draw();
    let parent = WindowInner::from_pub(window.window())
        .active_popups()
        .iter()
        .find(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
        .expect("the real tooltip must open after timed hover")
        .parent_item
        .upgrade()
        .unwrap();
    let origin = parent.map_to_native_window(parent.geometry().origin);
    let area = parent.downcast::<TooltipArea>().unwrap();
    let cursor = LogicalPosition::new(83.0, 61.0);
    assert!(synchronize_tooltip_anchors(window.window(), cursor));
    assert_eq!(area.as_pin_ref().mouse_x().get(), cursor.x - origin.x);
    assert_eq!(area.as_pin_ref().mouse_y().get(), cursor.y - origin.y);
    assert!(!synchronize_tooltip_anchors(window.window(), cursor));
    for outside in [
        LogicalPosition::new(-1.0, 61.0),
        LogicalPosition::new(83.0, -1.0),
    ] {
        assert!(!synchronize_tooltip_anchors(window.window(), outside));
        assert_eq!(area.as_pin_ref().mouse_x().get(), cursor.x - origin.x);
        assert_eq!(area.as_pin_ref().mouse_y().get(), cursor.y - origin.y);
    }
    let other = TestWindow::new().unwrap();
    assert!(!synchronize_tooltip_anchors(other.window(), cursor));
    window.hide().unwrap();
}
