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
use std::rc::Rc;

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
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
        let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
        let window = MainWindow::new().unwrap();
        midnight_ui::install_file_keys(&window);
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
        self.width = width;
        self.height = height;
        self.native
            .set_size(slint::PhysicalSize::new(width as u32, height as u32));
        self.window.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(width as f32, height as f32),
        });
        self.pixels
            .resize(width * height, slint::Rgb8Pixel::default());
        self.draw();
    }

    pub fn draw(&mut self) {
        self.native.draw_if_needed(|renderer| {
            renderer.render(&mut self.pixels, self.width);
        });
    }

    pub fn options_topic(&mut self, topic: i32) {
        click_popup(
            &self.window,
            ["Panel", "Appearance", "Command"][topic as usize],
            AccessibleRole::Button,
        );
        self.draw();
        assert_eq!(self.window.get_options_topic_index(), topic);
    }

    pub fn pixel(&self, x: f32, y: f32) -> slint::Rgb8Pixel {
        self.pixels[y as usize * self.width + x as usize]
    }
}

pub fn click_popup(window: &MainWindow, label: &str, role: AccessibleRole) {
    let inner = WindowInner::from_pub(window.window());
    let popups = inner.active_popups();
    let popup = popups
        .iter()
        .rev()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .expect("an open control popup");
    let item = find(ItemRc::new_root(popup.component.clone()), label, role)
        .unwrap_or_else(|| panic!("no accessible popup control: {label}"));
    drop(popups);
    click_item(window, item);
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
    let item = root_item(window, label, role);
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    slint::LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    )
}

pub fn root_item(window: &MainWindow, label: &str, role: AccessibleRole) -> ItemRc {
    let root = WindowInner::from_pub(window.window()).component();
    find(ItemRc::new_root(root), label, role)
        .unwrap_or_else(|| panic!("no accessible main control: {label}"))
}

/// Inspect actual text items: a control's accessible value may differ from its caption.
pub fn text_values(item: ItemRc) -> Vec<SharedString> {
    let mut values = Vec::new();
    if let Some(text) = item.downcast::<SimpleText>() {
        values.push(text.as_pin_ref().text());
    }
    if let Some(text) = item.downcast::<ComplexText>() {
        values.push(text.as_pin_ref().text());
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        values.extend(text_values(item));
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
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
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
