//! Widgets that assign their own value must keep showing the state in force.
//! Drives the real controls through accessibility actions and keys.
use super::image_presentation_tests::present as present_outputs;
use super::*;
use i_slint_core::{
    accessibility::{AccessibilityAction, AccessibleStringProperty},
    item_tree::ItemRc,
    window::WindowInner,
};
use slint::platform::{Key, WindowEvent};

const ONLINE: &str = "Allow network for this document only";
const SCRIPTS: &str = "Allow scripts to read and transmit this document";

fn find(item: ItemRc, label: &str) -> Option<ItemRc> {
    if item
        .accessible_string_property(AccessibleStringProperty::Label)
        .is_some_and(|value| value == label)
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

fn control(open: &OpenWindow, label: &str) -> ItemRc {
    let inner = WindowInner::from_pub(open.window.window());
    // Instantiate conditional rows and run pending change handlers.
    inner.ensure_tree_instantiated();
    find(ItemRc::new_root(inner.component()), label).unwrap_or_else(|| panic!("missing {label}"))
}

fn shown(open: &OpenWindow, label: &str, property: AccessibleStringProperty) -> String {
    control(open, label)
        .accessible_string_property(property)
        .unwrap_or_default()
        .into()
}

fn text_frame(revision: u64, generation: u64, mode: u16, modes: &[u16]) -> ViewerOutput {
    let ViewerOutput::Frame(mut frame) = text_presentation_tests::frame(revision, 0, 0.2) else {
        unreachable!()
    };
    frame.generation = generation;
    frame.mode = mode;
    frame.modes = modes.to_vec();
    ViewerOutput::Frame(frame)
}

fn press(open: &OpenWindow, key: Key) {
    let text: slint::SharedString = key.into();
    open.window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    open.window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

/// Open the chooser's popup and pick the entry below the current one.
fn choose_next_mode(open: &OpenWindow) {
    control(open, "Viewer mode").accessible_action(&AccessibilityAction::Expand);
    press(open, Key::DownArrow);
    press(open, Key::Return);
    slint::platform::update_timers_and_animations();
}

type Inputs = Rc<RefCell<Vec<ViewerInput>>>;

/// Record inputs; the returned flag decides whether the sender accepts them.
fn capture(open: &OpenWindow) -> (Inputs, Rc<Cell<bool>>) {
    let inputs = Inputs::default();
    let accept = Rc::new(Cell::new(true));
    let (captured, accepting) = (inputs.clone(), accept.clone());
    *open.input.borrow_mut() = Rc::new(move |input| {
        if accepting.get() {
            captured.borrow_mut().push(input);
        }
        accepting.get()
    });
    (inputs, accept)
}

#[test]
fn mode_chooser_follows_presented_mode_after_model_changes_and_user_choice() {
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let (inputs, accept) = capture(&open);
    let present = |output| present_outputs(&open, &service, &|_| {}, vec![output]);
    let value = || shown(&open, "Viewer mode", AccessibleStringProperty::Value);

    // A one-mode file instantiates the chooser; the next file replaces its model.
    present(text_frame(10, 7, 1, &[1]));
    assert_eq!(value(), "Text");
    present(text_frame(11, 7, 1, &[1, 3]));
    assert_eq!(value(), "Text");
    // Same plugin, binary content: only the presented mode index changes.
    present(text_frame(12, 7, 3, &[1, 3]));
    assert_eq!(value(), "Binary", "a model change detached the chooser");

    // The user's own choice must not detach it either.
    present(text_frame(13, 7, 1, &[1, 3]));
    assert_eq!(value(), "Text");
    choose_next_mode(&open);
    assert!(matches!(inputs.borrow().last(), Some(ViewerInput::Mode(3))));
    assert_eq!(value(), "Binary");
    // The backend applies the choice under a new generation, then the next
    // file of the same plugin opens as text.
    present(text_frame(14, 8, 3, &[1, 3]));
    assert_eq!(value(), "Binary");
    present(text_frame(15, 9, 1, &[1, 3]));
    assert_eq!(value(), "Text", "a user choice detached the chooser");

    // A refused request restores the mode in force.
    accept.set(false);
    let sent = inputs.borrow().len();
    choose_next_mode(&open);
    assert_eq!(inputs.borrow().len(), sent);
    assert_eq!(open.window.get_mode_index(), 0);
    assert_eq!(value(), "Text");
}

#[test]
fn web_policy_boxes_show_policy_in_force_after_document_change_and_refusal() {
    let (_runtime, _manager, open, service) = image_presentation_tests::open();
    open.timer.stop();
    let (inputs, accept) = capture(&open);
    let present = |output| present_outputs(&open, &service, &|_| {}, vec![output]);
    let checked = |label| shown(&open, label, AccessibleStringProperty::Checked);
    let html = [VIEWER_MODE_HTML];
    let state = |generation| ViewerOutput::WebState {
        generation,
        policy: viewer_web::Policy::default(),
        policy_change_allowed: true,
        zoom: 1.0,
    };

    present(text_frame(10, 7, VIEWER_MODE_HTML, &html));
    present(state(7));
    control(&open, ONLINE).accessible_action(&AccessibilityAction::Default);
    control(&open, SCRIPTS).accessible_action(&AccessibilityAction::Default);
    assert_eq!([checked(ONLINE), checked(SCRIPTS)], ["true", "true"]);
    assert!(matches!(
        inputs.borrow().last(),
        Some(ViewerInput::Web {
            input: viewer_web::CommandKind::Policy(viewer_web::Policy {
                network: true,
                scripts: true
            }),
            ..
        })
    ));

    // The next HTML document starts offline with scripts disabled.
    present(ViewerOutput::LayoutSelected {
        token: 1,
        mode: VIEWER_MODE_HTML,
        content_generation: 8,
    });
    assert!(!open.window.get_web_network() && !open.window.get_web_scripts());
    assert_eq!([checked(ONLINE), checked(SCRIPTS)], ["false", "false"]);
    // Before its first frame no policy can be sent; the box must not claim one.
    control(&open, ONLINE).accessible_action(&AccessibilityAction::Default);
    assert!(!open.window.get_web_network());
    assert_eq!(checked(ONLINE), "false");

    present(text_frame(11, 8, VIEWER_MODE_HTML, &html));
    present(state(8));
    control(&open, ONLINE).accessible_action(&AccessibilityAction::Default);
    assert_eq!(checked(ONLINE), "true");
    // A full command queue refuses the change: network stays enabled and shown.
    accept.set(false);
    control(&open, ONLINE).accessible_action(&AccessibilityAction::Default);
    assert!(open.window.get_web_network());
    assert_eq!(checked(ONLINE), "true");
    // The accepted change recreates the browser context. Its next frame and the
    // backend's state reply remain authoritative for both boxes.
    present_outputs(
        &open,
        &service,
        &|_| {},
        vec![text_frame(12, 9, VIEWER_MODE_HTML, &html), state(9)],
    );
    assert_eq!([checked(ONLINE), checked(SCRIPTS)], ["false", "false"]);
}
