//! Popup and embedded browser input must keep their native ownership separate.
use super::*;
use i_slint_core::{
    accessibility::{AccessibilityAction, AccessibleStringProperty},
    item_tree::ItemRc,
    items::{AccessibleRole, Flickable},
    window::WindowInner,
};
use slint::winit_030::winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent},
};

fn find(item: ItemRc, predicate: &dyn Fn(&ItemRc) -> bool) -> Option<ItemRc> {
    if predicate(&item) {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = find(item, predicate) {
            return Some(found);
        }
    }
    None
}

fn popup(open: &OpenWindow) -> ItemRc {
    let inner = WindowInner::from_pub(open.window.window());
    let combo = find(ItemRc::new_root(inner.component()), &|item| {
        item.accessible_role() == AccessibleRole::Combobox
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .as_deref()
                == Some("Viewer mode")
    })
    .expect("viewer mode ComboBox");
    combo.accessible_action(&AccessibilityAction::Expand);
    let component = inner
        .active_popups()
        .last()
        .expect("expanded popup")
        .component
        .clone();
    find(ItemRc::new_root(component), &|item| {
        item.downcast::<Flickable>().is_some()
    })
    .expect("mode popup scroll viewport")
}

fn configure(open: &OpenWindow) -> Rc<RefCell<Vec<ViewerInput>>> {
    open.timer.stop();
    open.window.set_image_mode(false);
    open.window.set_tiled(false);
    open.window.set_web_mode(true);
    open.window
        .set_modes(slint::ModelRc::new(slint::VecModel::from(
            (0..100)
                .map(|index| slint::SharedString::from(format!("Mode {index}")))
                .collect::<Vec<_>>(),
        )));
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let capture = inputs.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        capture.borrow_mut().push(input);
        true
    });
    inputs
}

fn cursor(open: &OpenWindow, x: f32, y: f32) -> WindowEvent {
    let scale = f64::from(open.window.window().scale_factor());
    WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(f64::from(x) * scale, f64::from(y) * scale),
    }
}
fn button(state: ElementState) -> WindowEvent {
    WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state,
        button: MouseButton::Left,
    }
}
fn wheel(delta: MouseScrollDelta) -> WindowEvent {
    WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta,
        phase: TouchPhase::Moved,
    }
}
fn offset(viewport: &ItemRc) -> f32 {
    viewport
        .downcast::<Flickable>()
        .unwrap()
        .as_pin_ref()
        .content_y()
        .get()
}

#[test]
fn web_popup_owns_pointer_and_wheel_input_until_dismissed() {
    let (_runtime, _manager, open, _service) = image_presentation_tests::open();
    let inputs = configure(&open);
    let x = open.window.get_content_left() + 20.0;
    let y = open.window.get_content_top() + 20.0;
    assert!(web_input::observe(&open, &cursor(&open, x, y), true));
    let viewport = popup(&open);
    let area = viewport.geometry();
    let origin = viewport.map_to_native_window(area.origin);
    let point = (
        origin.x + area.width() / 2.0,
        origin.y + area.height() / 2.0,
    );
    assert!(point.0 >= open.window.get_content_left());
    assert!(point.1 >= open.window.get_content_top());
    assert!(point.0 < open.window.get_content_left() + open.window.get_content_width());
    assert!(point.1 < open.window.get_content_top() + open.window.get_content_height());
    inputs.borrow_mut().clear();
    for event in [
        cursor(&open, point.0, point.1),
        wheel(MouseScrollDelta::LineDelta(0.0, -1.0)),
        wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
            0.0, -3.25,
        ))),
        button(ElementState::Pressed),
        button(ElementState::Released),
    ] {
        assert!(
            !web_input::observe(&open, &event, true),
            "popup input was taken by Chromium: {event:?}"
        );
        assert!(inputs.borrow().is_empty(), "popup input reached Chromium");
    }
    let inner = WindowInner::from_pub(open.window.window());
    let popup_id = inner.active_popups().last().unwrap().popup_id;
    inner.close_popup(popup_id);
    assert!(web_input::observe(
        &open,
        &wheel(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
            0.0, -3.25
        ))),
        true
    ));
    let captured = inputs.borrow();
    let Some(ViewerInput::Web {
        input: viewer_web::CommandKind::Wheel(value),
        ..
    }) = captured.last()
    else {
        panic!("browser did not resume after popup dismissal");
    };
    let ratio = f64::from(open.raster_size.get().0) / f64::from(open.window.get_content_width());
    assert!(
        (value.x - f64::from(point.0 - open.window.get_content_left()) * ratio).abs() < 0.01,
        "popup cursor movement must update the resumed browser position"
    );
    drop(captured);
    close(&open);
}

#[test]
fn web_consumed_release_does_not_leave_auxiliary_scrollbars_pressed() {
    let (_runtime, _manager, open, _service) = image_presentation_tests::open();
    let _inputs = configure(&open);
    let mut ancillary = midnight_ui_input::NativeScroll::new(&open.window);
    let x = open.window.get_content_left() + 20.0;
    let top = open.window.get_content_top();
    for event in [
        cursor(&open, x, top - 2.0),
        button(ElementState::Pressed),
        cursor(&open, x, top + 20.0),
        button(ElementState::Released),
    ] {
        observe_native_content(&open, &event, true, &mut ancillary);
    }
    let viewport = popup(&open);
    let area = viewport.geometry();
    let origin = viewport.map_to_native_window(area.origin);
    let point = (
        origin.x + area.width() - 3.0,
        origin.y + area.height() / 2.0,
    );
    observe_native_content(
        &open,
        &cursor(&open, point.0, point.1),
        true,
        &mut ancillary,
    );
    let initial = offset(&viewport);
    assert!(
        viewport
            .downcast::<Flickable>()
            .unwrap()
            .as_pin_ref()
            .content_height()
            .get()
            - area.height()
            + initial
            > 60.0,
        "fixture must retain a full notch of popup scroll range",
    );
    assert!(observe_native_content(
        &open,
        &wheel(MouseScrollDelta::LineDelta(0.0, -1.0)),
        true,
        &mut ancillary
    ));
    for _ in 0..24 {
        image_presentation_tests::advance_time(Duration::from_millis(16));
        observe_native_content(&open, &WindowEvent::RedrawRequested, true, &mut ancillary);
    }
    assert!(
        (offset(&viewport) - initial + 60.0).abs() < 0.05,
        "release consumed by Chromium left the popup scrollbar pressed: {} -> {}",
        initial,
        offset(&viewport)
    );
    close(&open);
}

#[test]
fn raster_wheel_after_a_browser_consumed_release_is_not_stuck_in_pointer_drag_mode() {
    let (_runtime, manager, open, service) = image_presentation_tests::open();
    let _inputs = configure(&open);
    let mut ancillary = midnight_ui_input::NativeScroll::new(&open.window);
    let x = open.window.get_content_left() + 20.0;
    let top = open.window.get_content_top();
    for event in [
        cursor(&open, x, top - 2.0),
        button(ElementState::Pressed),
        cursor(&open, x, top + 20.0),
        button(ElementState::Released),
    ] {
        observe_native_content(&open, &event, true, &mut ancillary);
    }
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![text_presentation_tests::frame(10, 0, 0.2)],
    );
    observe_native_content(&open, &cursor(&open, x, top + 20.0), true, &mut ancillary);
    assert!(
        observe_native_content(
            &open,
            &wheel(MouseScrollDelta::LineDelta(0.0, -1.0)),
            true,
            &mut ancillary
        ),
        "browser-consumed release left the content scroller pressed"
    );
    manager.close_all();
}
