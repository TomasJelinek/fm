//! The notice in a refused HTML page's place (`sandbox_notice.rs`) and its Install….
use super::image_presentation_tests::{self, present as present_outputs};
use super::*;
use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ParentItemTraversalMode,
    window::WindowInner,
};
use slint::platform::{Key, WindowEvent};
use slint::winit_030::winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, ElementState, MouseButton, WindowEvent as NativeEvent},
};

const MESSAGE: &str = "The HTML viewer needs permission for Chromium's sandbox on this system.";

fn notice(installable: bool, installing: bool) -> ViewerOutput {
    ViewerOutput::SandboxNotice(Some(ViewerSandboxNotice {
        message: MESSAGE.into(),
        installable,
        installing,
    }))
}

/// The window as a refused HTML document leaves it: in web mode and still waiting for a
/// first frame, which never comes. Returns the inputs it sends to the backend.
fn refused(open: &OpenWindow) -> Rc<RefCell<Vec<ViewerInput>>> {
    open.timer.stop();
    open.window.set_image_mode(false);
    open.window.set_tiled(false);
    open.window.set_web_mode(true);
    open.layout.borrow_mut().awaiting_frame = true;
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let capture = inputs.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        capture.borrow_mut().push(input);
        true
    });
    inputs
}

fn installs(inputs: &RefCell<Vec<ViewerInput>>) -> usize {
    inputs
        .borrow()
        .iter()
        .filter(|input| matches!(input, ViewerInput::InstallSandboxPermission))
        .count()
}

#[test]
fn the_notice_takes_the_pages_place_and_install_is_sent_only_when_offered() {
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    let inputs = refused(&open);
    present_outputs(&open, &service, &|_| {}, vec![notice(true, false)]);
    let window = &open.window;
    assert!(window.get_sandbox_notice_visible());
    assert_eq!(window.get_sandbox_notice(), MESSAGE);
    assert!(window.get_sandbox_installable() && !window.get_sandbox_installing());
    window.invoke_install_sandbox_permission();
    assert_eq!(installs(&inputs), 1, "admitted although no frame came");
    // Neither while an installation waits for authorization nor without an offer.
    for (installable, installing) in [(true, true), (false, false)] {
        present_outputs(
            &open,
            &service,
            &|_| {},
            vec![notice(installable, installing)],
        );
        window.invoke_install_sandbox_permission();
        assert_eq!(installs(&inputs), 1);
    }
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::SandboxNotice(None)],
    );
    assert!(!window.get_sandbox_notice_visible());
    assert!(!window.get_sandbox_installable());
    assert_eq!(window.get_sandbox_notice(), "");
    close(&open);
}

fn cursor(open: &OpenWindow, x: f32, y: f32) -> NativeEvent {
    let scale = f64::from(open.window.window().scale_factor());
    NativeEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(f64::from(x) * scale, f64::from(y) * scale),
    }
}

fn button(state: ElementState) -> NativeEvent {
    NativeEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state,
        button: MouseButton::Left,
    }
}

#[test]
fn pointer_input_over_the_notice_is_not_taken_for_the_browser() {
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    let inputs = refused(&open);
    let x = open.window.get_content_left() + 20.0;
    let y = open.window.get_content_top() + 20.0;
    present_outputs(&open, &service, &|_| {}, vec![notice(true, false)]);
    for event in [
        cursor(&open, x, y),
        button(ElementState::Pressed),
        button(ElementState::Released),
    ] {
        assert!(!web_input::observe(&open, &event, true), "{event:?}");
    }
    assert!(inputs.borrow().is_empty());
    // Without the notice the browser surface takes it again.
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::SandboxNotice(None)],
    );
    assert!(web_input::observe(&open, &cursor(&open, x, y), true));
    close(&open);
}

fn key(open: &OpenWindow, key: Key) {
    let window = open.window.window();
    window.dispatch_event(WindowEvent::KeyPressed { text: key.into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: key.into() });
}

#[test]
fn tab_from_the_content_reaches_install_and_return_activates_it() {
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    let inputs = refused(&open);
    present_outputs(&open, &service, &|_| {}, vec![notice(true, false)]);
    let inner = WindowInner::from_pub(open.window.window());
    inner.ensure_tree_instantiated();
    open.window.invoke_focus_content();
    let focused_install = || {
        let mut focused = inner.focus_item.borrow().upgrade();
        while let Some(item) = focused {
            if item
                .accessible_string_property(AccessibleStringProperty::Label)
                .as_deref()
                == Some("Install…")
            {
                return true;
            }
            focused = item.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        false
    };
    let mut tabs = 0;
    while !focused_install() && tabs < 8 {
        key(&open, Key::Tab);
        tabs += 1;
    }
    assert!(focused_install(), "Install… is reachable with Tab");
    assert_eq!(tabs, 1, "it follows the content in the focus order");
    key(&open, Key::Return);
    assert_eq!(installs(&inputs), 1);
    // The document opens again: the page, not the vanished button, has the keyboard.
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::SandboxNotice(None)],
    );
    assert!(open.window.get_content_focused());
    close(&open);
}
