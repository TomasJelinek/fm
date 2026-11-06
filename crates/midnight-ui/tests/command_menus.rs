//! Shared command-menu delegates retain callbacks and gestures during native icon delivery.
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use midnight_ui::{MainWindow, Presenter, install};
use slint::{
    ComponentHandle, Image, Rgb8Pixel, SharedPixelBuffer,
    platform::{
        Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

const WIDTH: usize = 1340;
const HEIGHT: usize = 900;

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn key(window: &MainWindow, key: Key) {
    let text: slint::SharedString = key.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn draw(native: &MinimalSoftwareWindow, pixels: &mut [Rgb8Pixel]) {
    slint::platform::update_timers_and_animations();
    native.draw_if_needed(|renderer| {
        renderer.render(pixels, WIDTH);
    });
}

fn find_button(item: ItemRc, label: &str) -> Option<ItemRc> {
    if item.accessible_role() == AccessibleRole::Button
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|value| value.as_str() == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(button) = find_button(item, label) {
            return Some(button);
        }
    }
    None
}

fn menu_button(window: &MainWindow, label: &str) -> ItemRc {
    let inner = WindowInner::from_pub(window.window());
    let popups = inner.active_popups();
    let popup = popups
        .iter()
        .rev()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .unwrap();
    find_button(ItemRc::new_root(popup.component.clone()), label).unwrap()
}

fn center(item: &ItemRc) -> slint::LogicalPosition {
    let rect = item.geometry();
    let point = item.map_to_native_window(rect.origin);
    slint::LogicalPosition::new(
        point.x + rect.size.width / 2.0,
        point.y + rect.size.height / 2.0,
    )
}

#[test]
fn native_icon_delivery_during_menu_press_keeps_its_command_and_command_focus_never_executes() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    let path = std::env::temp_dir();
    install(
        &window,
        Rc::new(RefCell::new(Presenter::new(path.clone(), path))),
        Rc::new(|_| true),
    );
    let properties = Rc::new(RefCell::new(Vec::new()));
    let captured = properties.clone();
    window.on_properties(move |pane| captured.borrow_mut().push(pane));
    let runs = Rc::new(Cell::new(0));
    let captured = runs.clone();
    window.on_submit_command(move |_| {
        captured.set(captured.get() + 1);
        false
    });
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(WIDTH as u32, HEIGHT as u32));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(WIDTH as f32, HEIGHT as f32),
    });
    let mut pixels = vec![Rgb8Pixel::default(); WIDTH * HEIGHT];
    for (profile, (density, font)) in [(0, 11), (1, 15), (0, 32), (1, 32)].into_iter().enumerate() {
        window.invoke_apply_fonts("".into(), font, font);
        window.invoke_apply_preferences(0, density);
        window.invoke_focus_files();
        key(&window, Key::F9);
        draw(&native, &mut pixels);
        let position = center(&menu_button(&window, "Properties · Alt+Enter"));
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        window.window().dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        let before = properties.borrow().len();
        let mut icons = window.get_action_icons();
        let bytes: Vec<_> = (0..16 * 16)
            .flat_map(|_| [43 + 17 * profile as u8, 190, 119])
            .collect();
        icons.properties = Image::from_rgb8(SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(
            &bytes, 16, 16,
        ));
        window.set_action_icons(icons);
        draw(&native, &mut pixels);
        assert_eq!(
            center(&menu_button(&window, "Properties · Alt+Enter")),
            position
        );
        window
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
        assert_eq!(
            properties.borrow().get(before),
            Some(&0),
            "icon arrival must retain the pressed command delegate"
        );
        assert_eq!(properties.borrow().len(), before + 1);
        assert!(
            WindowInner::from_pub(window.window())
                .active_popups()
                .is_empty()
        );
        key(&window, Key::F9);
        draw(&native, &mut pixels);
        let position = center(&menu_button(&window, "Command line"));
        for event in [
            WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            },
            WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            },
        ] {
            window.window().dispatch_event(event);
        }
        assert!(window.get_command_focused());
        assert_eq!(runs.get(), 0, "focusing the editor cannot run a command");
        assert!(
            WindowInner::from_pub(window.window())
                .active_popups()
                .is_empty()
        );
    }
    window.hide().unwrap();
}
