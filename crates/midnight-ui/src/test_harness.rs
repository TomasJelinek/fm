//! Software-rendered `MainWindow` harness for crate unit tests: real installed
//! presenter callbacks, accessibility-driven native input and deferred
//! `changed` handlers settled the way a native frame settles them.
use crate::{MainWindow, Presenter};
use i_slint_core::{
    accessibility::{AccessibilityAction, AccessibleStringProperty},
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use midnight_core::{Command, Event, UserPreferences};
use slint::{
    ComponentHandle, SharedString,
    platform::{
        Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

const WIDTH: usize = 1340;
const HEIGHT: usize = 820;

/// Real time until a test freezes it; afterwards Slint timers and animations
/// advance only through `Harness::advance`.
#[derive(Clone)]
struct Clock {
    started: Instant,
    frozen: Rc<Cell<Option<Duration>>>,
}

impl Clock {
    fn now(&self) -> Duration {
        self.frozen.get().unwrap_or_else(|| self.started.elapsed())
    }
}

/// The first adapter is the main window; later ones (native dialogs such as
/// Jobs or progress windows) are separate software windows kept for inspection.
struct TestPlatform {
    main: Rc<MinimalSoftwareWindow>,
    main_created: Cell<bool>,
    children: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    clock: Clock,
}

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        if !self.main_created.replace(true) {
            return Ok(self.main.clone());
        }
        let child = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        self.children.borrow_mut().push(child.clone());
        Ok(child)
    }

    fn duration_since_start(&self) -> Duration {
        self.clock.now()
    }
}

pub(crate) struct Harness {
    pub(crate) window: MainWindow,
    native: Rc<MinimalSoftwareWindow>,
    pixels: Vec<slint::Rgb8Pixel>,
    /// Current window width: the pixel stride of the fixed buffer.
    width: usize,
    clock: Clock,
    pub(crate) presenter: Rc<RefCell<Presenter>>,
    pub(crate) sent: Rc<RefCell<Vec<Command>>>,
    /// While set, the backend sender refuses commands as a full queue does.
    pub(crate) refuse: Rc<Cell<bool>>,
    /// Native windows created after the main window, in creation order.
    pub(crate) children: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
}

impl Harness {
    pub(crate) fn new(preferences: UserPreferences) -> Self {
        Self::with_location(preferences, "left".into())
    }

    pub(crate) fn with_location(
        preferences: UserPreferences,
        location: std::path::PathBuf,
    ) -> Self {
        Self::with_locations(preferences, location.clone(), location)
    }

    /// The left panel starts in `left`, the right one in `right`.
    pub(crate) fn with_locations(
        preferences: UserPreferences,
        left: std::path::PathBuf,
        right: std::path::PathBuf,
    ) -> Self {
        let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        let children = Rc::new(RefCell::new(Vec::new()));
        let clock = Clock {
            started: Instant::now(),
            frozen: Rc::default(),
        };
        slint::platform::set_platform(Box::new(TestPlatform {
            main: native.clone(),
            main_created: Cell::new(false),
            children: children.clone(),
            clock: clock.clone(),
        }))
        .unwrap();
        let window = MainWindow::new().unwrap();
        let presenter = Rc::new(RefCell::new(Presenter::new(left, right)));
        let sent = Rc::new(RefCell::new(Vec::new()));
        let captured = sent.clone();
        let refuse = Rc::new(Cell::new(false));
        let full = refuse.clone();
        crate::install(
            &window,
            presenter.clone(),
            Rc::new(move |command| {
                if full.get() {
                    return false;
                }
                captured.borrow_mut().push(command);
                true
            }),
        );
        presenter
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::new(
                midnight_core::AcceptedPreferences {
                    preferences,
                    comparators: Default::default(),
                },
            ))));
        crate::refresh(&window, &presenter.borrow());
        window.show().unwrap();
        window
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        native.set_size(slint::PhysicalSize::new(WIDTH as u32, HEIGHT as u32));
        window.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(WIDTH as f32, HEIGHT as f32),
        });
        let mut harness = Self {
            window,
            native,
            pixels: vec![slint::Rgb8Pixel::default(); WIDTH * HEIGHT],
            width: WIDTH,
            clock,
            presenter,
            sent,
            refuse,
            children,
        };
        harness.settle();
        harness
    }

    pub(crate) fn options(preferences: UserPreferences, topic: i32) -> Self {
        let mut harness = Self::new(preferences);
        harness.window.invoke_show_options();
        harness.window.set_options_topic_index(topic);
        harness.settle();
        harness
    }

    /// Run deferred `changed` handlers and render, as a native frame does.
    pub(crate) fn settle(&mut self) {
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
            self.native.draw_if_needed(|renderer| {
                renderer.render(&mut self.pixels, self.width);
            });
        }
    }

    pub(crate) fn refresh(&mut self) {
        crate::refresh(&self.window, &self.presenter.borrow());
        self.settle();
    }

    /// Stop real time: from now on Slint timers fire only through `advance`.
    pub(crate) fn freeze_time(&self) {
        if self.clock.frozen.get().is_none() {
            self.clock.frozen.set(Some(self.clock.started.elapsed()));
        }
    }

    /// Advance the frozen clock, then run the timers that became due.
    pub(crate) fn advance(&mut self, by: Duration) {
        self.freeze_time();
        self.clock.frozen.set(Some(self.clock.now() + by));
        self.settle();
    }

    /// Resize the window as a native resize does (never beyond the initial size).
    pub(crate) fn resize(&mut self, width: usize, height: usize) {
        assert!(
            width <= WIDTH && height <= HEIGHT,
            "the pixel buffer is fixed"
        );
        self.width = width;
        self.native
            .set_size(slint::PhysicalSize::new(width as u32, height as u32));
        self.window.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(width as f32, height as f32),
        });
        self.settle();
    }

    /// The label of the control that owns keyboard focus: the focused item or
    /// its nearest labelled ancestor (a text field's inner editor, say).
    pub(crate) fn focused_label(&self) -> Option<String> {
        let mut item = WindowInner::from_pub(self.window.window())
            .focus_item
            .borrow()
            .upgrade()?;
        loop {
            if let Some(label) = item
                .accessible_string_property(AccessibleStringProperty::Label)
                .filter(|label| !label.is_empty())
            {
                return Some(label.to_string());
            }
            item = item.parent_item(ParentItemTraversalMode::StopAtPopups)?;
        }
    }

    /// The topmost dialog surface: the newest non-tooltip popup, else the window.
    pub(crate) fn find(&self, matches: impl Fn(&ItemRc) -> bool) -> Option<ItemRc> {
        let inner = WindowInner::from_pub(self.window.window());
        let root = inner
            .active_popups()
            .iter()
            .rev()
            .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
            .map(|popup| ItemRc::new_root(popup.component.clone()))
            .unwrap_or_else(|| ItemRc::new_root(inner.component()));
        fn walk(item: ItemRc, matches: &dyn Fn(&ItemRc) -> bool) -> Option<ItemRc> {
            if matches(&item) {
                return Some(item);
            }
            let mut child = item.first_child();
            while let Some(current) = child {
                child = current.next_sibling();
                if let Some(found) = walk(current, matches) {
                    return Some(found);
                }
            }
            None
        }
        walk(root, &matches)
    }

    pub(crate) fn item(&self, label: &str, role: AccessibleRole) -> ItemRc {
        self.find(|item| {
            item.accessible_role() == role
                && item
                    .accessible_string_property(AccessibleStringProperty::Label)
                    .is_some_and(|value| value.as_str() == label)
        })
        .unwrap_or_else(|| panic!("no accessible control: {label}"))
    }

    pub(crate) fn value(&self, label: &str, role: AccessibleRole) -> String {
        self.item(label, role)
            .accessible_string_property(AccessibleStringProperty::Value)
            .unwrap_or_default()
            .to_string()
    }

    pub(crate) fn text(&self, label: &str) -> String {
        self.value(label, AccessibleRole::TextInput)
    }

    pub(crate) fn checked(&self, label: &str) -> bool {
        self.item(label, AccessibleRole::Checkbox)
            .accessible_string_property(AccessibleStringProperty::Checked)
            .is_some_and(|value| value.as_str() == "true")
    }

    /// Native input: the widget assigns its own value, as typing does.
    pub(crate) fn type_text(&mut self, label: &str, text: &str) {
        self.set_value(label, AccessibleRole::TextInput, text);
    }

    pub(crate) fn set_value(&mut self, label: &str, role: AccessibleRole, value: &str) {
        self.item(label, role)
            .accessible_action(&AccessibilityAction::SetValue(value.into()));
        self.settle();
    }

    pub(crate) fn activate(&mut self, label: &str, role: AccessibleRole) {
        self.item(label, role)
            .accessible_action(&AccessibilityAction::Default);
        self.settle();
    }

    pub(crate) fn activate_item(&mut self, item: ItemRc) {
        item.accessible_action(&AccessibilityAction::Default);
        self.settle();
    }

    pub(crate) fn key(&mut self, key: Key) {
        let text = SharedString::from(key);
        self.window
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        self.window
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text });
        self.settle();
    }

    /// A shortcut chord: modifiers pressed, the key typed, modifiers released.
    pub(crate) fn chord(&mut self, modifiers: &[Key], text: &str) {
        let window = self.window.window();
        for &modifier in modifiers {
            window.dispatch_event(WindowEvent::KeyPressed {
                text: modifier.into(),
            });
        }
        key_edges(window, text.into());
        for &modifier in modifiers.iter().rev() {
            window.dispatch_event(WindowEvent::KeyReleased {
                text: modifier.into(),
            });
        }
        self.settle();
    }

    /// Mouse-wheel input at a window position (negative `delta_y` scrolls down).
    pub(crate) fn wheel(&mut self, x: f32, y: f32, delta_y: f32) {
        self.window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(x, y),
                delta_x: 0.0,
                delta_y,
            });
        self.settle();
    }

    /// Left-button pointer input at a window position: 0 press, 1 move, 2 release.
    pub(crate) fn pointer(&self, phase: i32, x: f32, y: f32) {
        let position = slint::LogicalPosition::new(x, y);
        let button = PointerEventButton::Left;
        self.window.window().dispatch_event(match phase {
            0 => WindowEvent::PointerPressed { position, button },
            2 => WindowEvent::PointerReleased { position, button },
            _ => WindowEvent::PointerMoved { position },
        });
    }

    pub(crate) fn click(&mut self, label: &str, role: AccessibleRole) {
        let item = self.item(label, role);
        let rect = item.geometry();
        let origin = item.map_to_native_window(rect.origin);
        let position = slint::LogicalPosition::new(
            origin.x + rect.size.width / 2.0,
            origin.y + rect.size.height / 2.0,
        );
        let button = PointerEventButton::Left;
        let window = self.window.window();
        window.dispatch_event(WindowEvent::PointerMoved { position });
        window.dispatch_event(WindowEvent::PointerPressed { position, button });
        window.dispatch_event(WindowEvent::PointerReleased { position, button });
        self.settle();
    }

    /// Choose a combo box entry the way a keyboard user does in its popup.
    pub(crate) fn choose(&mut self, label: &str, steps: i32) {
        self.click(label, AccessibleRole::Combobox);
        for _ in 0..steps.unsigned_abs() {
            self.key(if steps > 0 {
                Key::DownArrow
            } else {
                Key::UpArrow
            });
        }
        self.key(Key::Return);
    }
}

/// Press and release one key. Windows' winit adapter also reports the text
/// without modifiers, which shortcut matching reads for Ctrl chords.
#[cfg(windows)]
fn key_edges(window: &slint::Window, text: SharedString) {
    use i_slint_core::input::{InternalKeyEvent, KeyEvent, KeyEventType};
    for event_type in [KeyEventType::KeyPressed, KeyEventType::KeyReleased] {
        let mut key_event = KeyEvent::default();
        key_event.text = text.clone();
        window.dispatch_event(WindowEvent::internal(InternalKeyEvent {
            key_event,
            text_without_modifiers: text.clone(),
            event_type,
            ..Default::default()
        }));
    }
}

#[cfg(not(windows))]
fn key_edges(window: &slint::Window, text: SharedString) {
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}
