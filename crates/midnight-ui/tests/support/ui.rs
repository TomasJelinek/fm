//! Shared native-event helpers for the panel refinement integration tests.
#![allow(dead_code)]

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, SimpleText},
    window::{WindowInner, WindowKind},
};
use midnight_ui::MainWindow;
use slint::{
    ComponentHandle, SharedString,
    platform::{
        Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::Cell, rc::Rc, time::Duration};

#[cfg(windows)]
#[path = "windows_keys.rs"]
pub mod platform_keys;
#[cfg(not(windows))]
#[path = "default_keys.rs"]
pub mod platform_keys;

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}
struct ClockPlatform(Rc<MinimalSoftwareWindow>, Rc<Cell<Duration>>);
impl Platform for ClockPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
    fn duration_since_start(&self) -> Duration {
        self.1.get()
    }
}

pub struct TestUi {
    pub window: MainWindow,
    pub native: Rc<MinimalSoftwareWindow>,
    pub pixels: Vec<slint::Rgb8Pixel>,
    pub width: usize,
    pub height: usize,
}

impl TestUi {
    pub fn new() -> Self {
        Self::with_clock(None)
    }

    pub fn with_clock(clock: Option<Rc<Cell<Duration>>>) -> Self {
        Self::initialize(clock, true)
    }

    /// A fixture whose Slint timers and animations only advance through [`advance`].
    pub fn with_virtual_clock() -> (Self, Rc<Cell<Duration>>) {
        let clock = Rc::new(Cell::new(Duration::ZERO));
        (Self::with_clock(Some(clock.clone())), clock)
    }

    /// Exercise the production initializer without preinstalling widget callbacks.
    pub fn bare() -> Self {
        Self::initialize(None, false)
    }

    /// [`Self::bare`] whose Slint timers and animations only follow `clock`.
    pub fn bare_with_clock(clock: Rc<Cell<Duration>>) -> Self {
        Self::initialize(Some(clock), false)
    }

    fn initialize(clock: Option<Rc<Cell<Duration>>>, widget_callbacks: bool) -> Self {
        let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        let platform: Box<dyn Platform> = if let Some(clock) = clock {
            Box::new(ClockPlatform(native.clone(), clock))
        } else {
            Box::new(TestPlatform(native.clone()))
        };
        slint::platform::set_platform(platform).unwrap();
        let window = MainWindow::new().unwrap();
        // Bare widget tests represent a ready app; installing a real Presenter
        // replaces this with its actual asynchronous preference-load state.
        window.set_preferences_ready(true);
        if widget_callbacks {
            midnight_ui::install_file_keys(&window);
        }
        window.show().unwrap();
        window
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        let mut ui = Self {
            window,
            native,
            pixels: Vec::new(),
            width: 1340,
            height: 820,
        };
        ui.resize(1340, 820);
        ui
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        self.set_size(width, height);
        self.draw();
    }

    /// Resize the window and its frame buffer without rendering a frame.
    pub fn set_size(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.native
            .set_size(slint::PhysicalSize::new(width as u32, height as u32));
        self.window.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(width as f32, height as f32),
        });
        self.pixels
            .resize(width * height, slint::Rgb8Pixel::default());
    }

    pub fn draw(&mut self) {
        // Settle deferred property-change callbacks as a native frame does.
        slint::platform::update_timers_and_animations();
        self.native.draw_if_needed(|renderer| {
            renderer.render(&mut self.pixels, self.width);
        });
    }

    /// Settle timers, change handlers and lazily instantiated repeaters/conditionals as
    /// a frame does, without software-rendering it. Item geometry is then current for
    /// geometry-only assertions; [`Self::pixels`] keeps the last rendered frame, and the
    /// next [`Self::draw`] still repaints everything that changed in between.
    pub fn layout(&mut self) {
        settle_layout(self.window.window());
    }

    pub fn options_topic(&mut self, topic: i32) {
        let label = match topic {
            0 => "General",
            1 => "Appearance",
            3 => "Keyboard",
            4 => "Editors",
            10 => "Comparators",
            5 => "Plugins",
            6 => "Previews",
            7 => "Archives",
            8 => "Colors",
            9 => "Custom Commands",
            _ => panic!("unsupported Options topic: {topic}"),
        };
        if let Some(button) = try_popup_item(&self.window, label, AccessibleRole::Button) {
            click_item(&self.window, button);
        } else {
            let current_index = options_topic_position(self.window.get_options_topic_index());
            let target_index = options_topic_position(topic);
            click_popup(&self.window, "Options topic", AccessibleRole::Combobox);
            for _ in 0..target_index.abs_diff(current_index) {
                key(
                    &self.window,
                    if target_index > current_index {
                        Key::DownArrow
                    } else {
                        Key::UpArrow
                    },
                );
            }
            key(&self.window, Key::Return);
        }
        self.draw();
        assert_eq!(self.window.get_options_topic_index(), topic);
    }

    pub fn pixel(&self, x: f32, y: f32) -> slint::Rgb8Pixel {
        self.pixels[y as usize * self.width + x as usize]
    }
}

/// The topic identities of the Options pages in their listed order (`OptionsTopics` in
/// ui/options-navigation.slint).
pub const OPTIONS_TOPIC_ORDER: [i32; 10] = [0, 1, 8, 6, 7, 4, 10, 9, 3, 5];

/// The OptionsTopics picker position, with the removed Command topic mapped to General.
pub fn options_topic_position(topic: i32) -> usize {
    OPTIONS_TOPIC_ORDER
        .iter()
        .position(|&id| id == topic)
        .unwrap_or(0)
}

/// Advance a [`TestUi::with_virtual_clock`] clock and run the timers and animations
/// that became due, instead of sleeping for Slint's real-time delays.
pub fn advance(clock: &Cell<Duration>, by: Duration) {
    clock.set(clock.get() + by);
    slint::platform::update_timers_and_animations();
}

/// The layout-only part of a frame: timers, change handlers and lazy item-tree
/// instantiation (which a software render or an input event would otherwise run).
pub fn settle_layout(window: &slint::Window) {
    slint::platform::update_timers_and_animations();
    WindowInner::from_pub(window).ensure_tree_instantiated();
}

/// Whether a geometry sweep must render this combination: always for its first
/// (reference) case, and for every case when render artifacts were requested.
pub fn render_case(reference: bool) -> bool {
    reference || std::env::var_os("MIDNIGHT_UI_RENDER_DIR").is_some_and(|dir| !dir.is_empty())
}

pub fn click_popup(window: &MainWindow, label: &str, role: AccessibleRole) {
    click_item(window, popup_item(window, label, role));
}

pub fn popup_item(window: &MainWindow, label: &str, role: AccessibleRole) -> ItemRc {
    try_popup_item(window, label, role)
        .unwrap_or_else(|| panic!("no accessible popup control: {label}"))
}

pub fn try_popup_item(window: &MainWindow, label: &str, role: AccessibleRole) -> Option<ItemRc> {
    let inner = WindowInner::from_pub(window.window());
    let popups = inner.active_popups();
    let popup = popups
        .iter()
        .rev()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))?;
    find(ItemRc::new_root(popup.component.clone()), label, role)
}

pub fn click_root(window: &MainWindow, label: &str, role: AccessibleRole) {
    let position = root_position(window, label, role);
    click(window, position.x, position.y);
}

pub fn root_position(
    window: &MainWindow,
    label: &str,
    role: AccessibleRole,
) -> slint::LogicalPosition {
    try_root_position(window, label, role)
        .unwrap_or_else(|| panic!("no accessible main control: {label}"))
}

pub fn try_root_position(
    window: &MainWindow,
    label: &str,
    role: AccessibleRole,
) -> Option<slint::LogicalPosition> {
    let root = WindowInner::from_pub(window.window()).component();
    let item = find(ItemRc::new_root(root), label, role)?;
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    Some(slint::LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    ))
}

pub fn root_item(window: &MainWindow, label: &str, role: AccessibleRole) -> ItemRc {
    try_root_item(window, label, role)
        .unwrap_or_else(|| panic!("no accessible main control: {label}"))
}

/// The main window's control with this label and role, if the layout shows one.
pub fn try_root_item(window: &MainWindow, label: &str, role: AccessibleRole) -> Option<ItemRc> {
    let root = WindowInner::from_pub(window.window()).component();
    find(ItemRc::new_root(root), label, role)
}

/// Inspect actual text items: a control's accessible value may differ from its caption.
pub fn text_values(item: ItemRc) -> Vec<SharedString> {
    collect_text(item, false)
}

/// Ignore hidden caption metrics and bindings when asserting rendered text.
pub fn visible_text_values(item: ItemRc) -> Vec<SharedString> {
    collect_text(item, true)
}

fn collect_text(item: ItemRc, visible_only: bool) -> Vec<SharedString> {
    let mut values = Vec::new();
    if let Some(text) = item.downcast::<SimpleText>()
        && (!visible_only || item.is_visible())
    {
        values.push(text.as_pin_ref().text());
    }
    if let Some(text) = item.downcast::<ComplexText>()
        && (!visible_only || item.is_visible())
    {
        values.push(text.as_pin_ref().text());
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        values.extend(collect_text(item, visible_only));
    }
    values
}

pub fn has_popup(window: &MainWindow) -> bool {
    WindowInner::from_pub(window.window())
        .active_popups()
        .iter()
        .any(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
}

fn click_item(window: &MainWindow, item: ItemRc) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    click(
        window,
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    );
}

fn find(item: ItemRc, label: &str, role: AccessibleRole) -> Option<ItemRc> {
    if item.accessible_role() == role
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|value| value.as_str() == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = find(item, label, role) {
            return Some(found);
        }
    }
    None
}

pub fn key(window: &MainWindow, text: impl Into<SharedString>) {
    platform_keys::key(window, text.into());
}

pub fn chord(window: &MainWindow, modifiers: &[Key], text: impl Into<SharedString>) {
    for modifier in modifiers {
        window.window().dispatch_event(WindowEvent::KeyPressed {
            text: (*modifier).into(),
        });
    }
    key(window, text);
    for modifier in modifiers.iter().rev() {
        window.window().dispatch_event(WindowEvent::KeyReleased {
            text: (*modifier).into(),
        });
    }
}

pub fn click(window: &MainWindow, x: f32, y: f32) {
    let position = slint::LogicalPosition::new(x, y);
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

pub fn pointer_button(
    window: &MainWindow,
    position: slint::LogicalPosition,
    button: PointerEventButton,
    pressed: bool,
) {
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window.window().dispatch_event(if pressed {
        WindowEvent::PointerPressed { position, button }
    } else {
        WindowEvent::PointerReleased { position, button }
    });
}
