//! Native touch capture must precede Flickable's delayed child pointer-down.
use super::*;
use i_slint_core::{
    accessibility::AccessibleStringProperty, input::TouchPhase as SlintTouchPhase,
    items::AccessibleRole, platform::InternalEvent,
};
use slint::winit_030::winit::event::Touch;

fn position(window: &MainWindow, label: &str) -> slint::LogicalPosition {
    fn find(item: ItemRc, label: &str) -> Option<ItemRc> {
        if item.accessible_role() == AccessibleRole::Button
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|text| text.as_str() == label)
        {
            return Some(item);
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            if let Some(found) = find(item, label) {
                return Some(found);
            }
        }
        None
    }
    let item = find(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        label,
    )
    .unwrap();
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    slint::LogicalPosition::new(
        origin.x + rect.width() / 2.0,
        origin.y + rect.height() / 2.0,
    )
}

fn touch(
    window: &MainWindow,
    state: &Rc<RefCell<InputState>>,
    timer: &slint::Timer,
    point: slint::LogicalPosition,
    phase: TouchPhase,
) {
    // Same ordering as Winit: native observer, then runtime touch translation.
    let scale = f64::from(window.window().scale_factor());
    observe(
        window,
        state,
        timer,
        &WindowEvent::Touch(Touch {
            device_id: DeviceId::dummy(),
            phase,
            location: PhysicalPosition::new(f64::from(point.x) * scale, f64::from(point.y) * scale),
            force: None,
            id: 1,
        }),
    );
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::internal(
            InternalEvent::Touch {
                id: 1,
                position: LogicalPoint::new(point.x, point.y),
                phase: match phase {
                    TouchPhase::Started => SlintTouchPhase::Started,
                    TouchPhase::Moved => SlintTouchPhase::Moved,
                    TouchPhase::Ended => SlintTouchPhase::Ended,
                    TouchPhase::Cancelled => SlintTouchPhase::Cancelled,
                },
            },
        ));
}

fn click(window: &MainWindow, position: slint::LogicalPosition) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let button = PointerEventButton::Left;
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window
        .window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
}

fn enter(window: &MainWindow) {
    use slint::platform::{Key, WindowEvent};
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Return.into(),
    });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Return.into(),
    });
}

#[test]
fn delayed_native_toolbar_touch_cannot_retarget_after_panel_or_location_changes() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    let time = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(TestPlatform(native.clone(), time.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    crate::install_file_keys(&window);
    window.set_preferences_ready(true);
    window.set_left_can_target_selection(true);
    window.set_right_can_target_selection(true);
    window.invoke_apply_preferences(0, 1);
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(700, 500));
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::Resized {
            size: slint::LogicalSize::new(700.0, 500.0),
        });
    let mut pixels = vec![slint::Rgb8Pixel::default(); 700 * 500];
    let state = Rc::new(RefCell::new(InputState::default()));
    let timer = slint::Timer::default();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let output = calls.clone();
    window.on_calculate_checksums(move |pane| output.borrow_mut().push(pane));
    for delayed in [false, true] {
        for change in 0..4 {
            window.set_active_pane(0);
            window.set_left_search_context("same".into());
            window.set_right_search_context("same".into());
            slint::platform::update_timers_and_animations();
            native.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, 700);
            });
            let point = position(&window, "Checksums");
            assert!(
                point.x > 40.0 && point.x < 660.0,
                "actual checksum button must be visible: {point:?}"
            );
            let send = |phase| touch(&window, &state, &timer, point, phase);
            calls.borrow_mut().clear();
            send(TouchPhase::Started);
            if change == 0 {
                window.set_active_pane(1);
            }
            if change == 1 {
                window.set_left_search_context("new location".into());
            }
            if delayed {
                time.set(time.get() + Duration::from_millis(120));
                slint::platform::update_timers_and_animations();
            }
            if change == 2 {
                send(TouchPhase::Cancelled);
            } else {
                send(TouchPhase::Ended);
            }
            slint::platform::update_timers_and_animations();
            if change == 3 {
                assert_eq!(*calls.borrow(), [0], "unchanged touch still dispatches");
            } else {
                assert!(
                    calls.borrow().is_empty(),
                    "stale/cancelled touch dispatched: delay={delayed}, change={change}, calls={:?}",
                    calls.borrow()
                );
            }
            if change == 2 {
                // The public mouse hook runs after child pointer-down. A previous
                // cancellation must neither execute nor poison that next click.
                click(&window, point);
                assert_eq!(*calls.borrow(), [0], "next mouse click after cancel");
                enter(&window);
                assert_eq!(*calls.borrow(), [0, 0], "keyboard after cancel");
                send(TouchPhase::Started);
                send(TouchPhase::Ended);
                slint::platform::update_timers_and_animations();
                assert_eq!(*calls.borrow(), [0, 0, 0], "next touch after cancel");
            }
        }
    }

    window.set_active_pane(0);
    window.set_left_can_target_cursor(true);
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 700);
    });
    let actions = Rc::new(RefCell::new(Vec::new()));
    let output = actions.clone();
    window.on_function_action(move |action, pane| output.borrow_mut().push((action, pane)));
    let point = position(&window, "F3 View");
    for delayed in [false, true] {
        actions.borrow_mut().clear();
        touch(&window, &state, &timer, point, TouchPhase::Started);
        if delayed {
            time.set(time.get() + Duration::from_millis(120));
            slint::platform::update_timers_and_animations();
        }
        touch(&window, &state, &timer, point, TouchPhase::Cancelled);
        slint::platform::update_timers_and_animations();
        assert!(
            actions.borrow().is_empty(),
            "footer canceled touch, delay={delayed}"
        );
        touch(&window, &state, &timer, point, TouchPhase::Started);
        touch(&window, &state, &timer, point, TouchPhase::Ended);
        slint::platform::update_timers_and_animations();
        assert_eq!(
            &*actions.borrow(),
            &[("files.view".into(), 0)],
            "next footer touch"
        );
    }
}
