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
        details: None,
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

const FAILURE: &str = "This presentation is password-protected or saved in the older binary \
                       Office format (.doc, .xls or .ppt), which the preview cannot open.";
const DETAILS: &str = "Unsupported Office content: is password-protected or saved in the \
                       older binary Office format (.doc, .xls or .ppt), which the preview \
                       cannot open";

/// What the engine sends for a document that failed: its message for the status line, and
/// a notice only when the failure has diagnostic details.
fn failure(details: Option<&str>) -> Vec<ViewerOutput> {
    let mut outputs = vec![ViewerOutput::Status(FAILURE.into())];
    if let Some(details) = details {
        outputs.push(ViewerOutput::SandboxNotice(Some(ViewerSandboxNotice {
            message: FAILURE.into(),
            installable: false,
            installing: false,
            details: Some(details.into()),
        })));
    }
    outputs
}

/// Whether a control named `label` is shown.
fn shown(open: &OpenWindow, label: &str) -> bool {
    fn find(item: i_slint_core::item_tree::ItemRc, label: &str) -> bool {
        if item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|value| value == label)
        {
            return item.is_visible();
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            if find(item, label) {
                return true;
            }
        }
        false
    }
    let inner = WindowInner::from_pub(open.window.window());
    inner.ensure_tree_instantiated();
    find(
        i_slint_core::item_tree::ItemRc::new_root(inner.component()),
        label,
    )
}

/// Whether the keyboard focus is in the control named `label`.
fn focused(open: &OpenWindow, label: &str) -> bool {
    let inner = WindowInner::from_pub(open.window.window());
    let mut focused = inner.focus_item.borrow().upgrade();
    while let Some(item) = focused {
        if item
            .accessible_string_property(AccessibleStringProperty::Label)
            .as_deref()
            == Some(label)
        {
            return true;
        }
        focused = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    false
}

/// The texts the viewer's copy received.
type Copied = Rc<RefCell<Vec<String>>>;

/// A viewer copy that records what it receives.
fn copies() -> (Copied, Rc<dyn Fn(String)>) {
    let copied = Rc::new(RefCell::new(Vec::new()));
    let capture = copied.clone();
    (copied, Rc::new(move |text| capture.borrow_mut().push(text)))
}

#[test]
fn a_failure_with_details_takes_the_pages_place_and_one_without_stays_on_the_status_line() {
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    refused(&open);
    let window = &open.window;
    present_outputs(&open, &service, &|_| {}, failure(None));
    assert_eq!(window.get_status(), FAILURE);
    assert!(!window.get_sandbox_notice_visible());
    assert!(!shown(&open, "Copy details"));
    present_outputs(&open, &service, &|_| {}, failure(Some(DETAILS)));
    assert_eq!(
        window.get_status(),
        FAILURE,
        "the status line keeps the message"
    );
    assert!(window.get_sandbox_notice_visible());
    assert_eq!(window.get_sandbox_notice(), FAILURE);
    assert!(!window.get_sandbox_installable());
    assert!(shown(&open, "Copy details"));
    assert!(!shown(&open, "Install…"));
    // The sandbox's notice has no details to copy.
    present_outputs(&open, &service, &|_| {}, vec![notice(true, false)]);
    assert!(shown(&open, "Install…"));
    assert!(!shown(&open, "Copy details"));
    present_outputs(&open, &service, &|_| {}, failure(Some(DETAILS)));
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![ViewerOutput::SandboxNotice(None)],
    );
    assert!(!window.get_sandbox_notice_visible());
    assert!(!shown(&open, "Copy details"));
    close(&open);
}

#[test]
fn copy_details_copies_the_message_a_blank_line_and_the_details_through_the_viewer() {
    let (copied, copy) = copies();
    let (_runtime, _manager, open, service) = image_presentation_tests::open_with_copy(copy);
    refused(&open);
    present_outputs(&open, &service, &|_| {}, failure(Some(DETAILS)));
    open.window.invoke_copy_sandbox_details();
    assert_eq!(*copied.borrow(), [format!("{FAILURE}\n\n{DETAILS}")]);
    // Nothing to copy from a notice without details, nor once another document opened.
    for outputs in [
        vec![notice(true, false)],
        vec![ViewerOutput::SandboxNotice(None)],
    ] {
        present_outputs(&open, &service, &|_| {}, outputs);
        open.window.invoke_copy_sandbox_details();
        assert_eq!(copied.borrow().len(), 1);
    }
    close(&open);
}

#[test]
fn copy_details_takes_no_focus_follows_the_content_and_takes_return_and_space() {
    let (copied, copy) = copies();
    let (_runtime, _manager, open, service) = image_presentation_tests::open_with_copy(copy);
    refused(&open);
    WindowInner::from_pub(open.window.window()).ensure_tree_instantiated();
    open.window.invoke_focus_content();
    present_outputs(&open, &service, &|_| {}, failure(Some(DETAILS)));
    assert!(
        open.window.get_content_focused(),
        "the viewer's keys keep working while the notice is shown"
    );
    let mut tabs = 0;
    while !focused(&open, "Copy details") && tabs < 8 {
        key(&open, Key::Tab);
        tabs += 1;
    }
    assert!(
        focused(&open, "Copy details"),
        "Copy details is reachable with Tab"
    );
    assert_eq!(tabs, 1, "it follows the content in the focus order");
    key(&open, Key::Return);
    key(&open, Key::Space);
    let expected = format!("{FAILURE}\n\n{DETAILS}");
    assert_eq!(*copied.borrow(), [expected.clone(), expected]);
    close(&open);
}
