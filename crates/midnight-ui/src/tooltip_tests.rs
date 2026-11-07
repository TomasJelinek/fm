use super::*;
use crate::FindWindow;
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{AccessibleRole, WindowItem},
    window::PopupWindowLocation,
};
use slint::platform::{
    Platform, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{cell::Cell, rc::Rc, time::Duration};

type Natives = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;
struct PlatformState {
    natives: Natives,
    time: Rc<Cell<Duration>>,
}
impl Platform for PlatformState {
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
    let natives = Rc::new(RefCell::new(Vec::new()));
    let time = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(PlatformState {
        natives: natives.clone(),
        time: time.clone(),
    }))
    .unwrap();
    (natives, time)
}
fn resize(window: &slint::Window, native: &MinimalSoftwareWindow, width: f32, height: f32) {
    native.set_size(slint::PhysicalSize::new(width as u32, height as u32));
    window.dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width, height),
    });
}
fn input(item: ItemRc, label: &str) -> Option<ItemRc> {
    if item.accessible_role() == AccessibleRole::TextInput
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|value| value.as_str() == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(item) = input(item, label) {
            return Some(item);
        }
    }
    None
}

#[test]
fn find_host_tracks_its_own_cursor_and_client_bounds_without_updating_main() {
    let (natives, _) = setup();
    let main = MainWindow::new().unwrap();
    install(&main).unwrap();
    main.show().unwrap();
    resize(main.window(), &natives.borrow()[0], 1340., 900.);
    main.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(20., 30.),
    });
    let find = FindWindow::new().unwrap();
    install_find(&find).unwrap();
    find.show().unwrap();
    resize(find.window(), &natives.borrow()[1], 360., 620.);
    find.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(350., 610.),
    });
    let placement = find.global::<TooltipPlacement>();
    assert_eq!(placement.get_surface_width(), 360.);
    assert_eq!(placement.get_surface_height(), 620.);
    assert_eq!(placement.get_cursor_x(), 350.);
    assert_eq!(placement.get_cursor_y(), 610.);
    assert_eq!(main.global::<TooltipPlacement>().get_cursor_x(), 20.);
    assert_eq!(main.global::<TooltipPlacement>().get_surface_width(), 1340.);
    resize(find.window(), &natives.borrow()[1], 320., 200.);
    assert_eq!(placement.get_surface_width(), 320.);
    assert_eq!(placement.get_surface_height(), 200.);
    find.window().dispatch_event(WindowEvent::PointerExited);
    assert_eq!(placement.get_cursor_x(), -1.);
    assert_eq!(placement.get_cursor_y(), -1.);
    find.hide().unwrap();
    main.hide().unwrap();
}

#[test]
fn actual_find_hint_is_offset_and_clamped_to_the_find_host() {
    let (natives, time) = setup();
    let find = FindWindow::new().unwrap();
    install_find(&find).unwrap();
    find.invoke_apply_preferences(0, 0, "Segoe UI".into(), 13, 13);
    find.show().unwrap();
    let native = natives.borrow()[0].clone();
    resize(find.window(), &native, 360., 620.);
    let mut pixels = vec![slint::Rgb8Pixel::default(); 360 * 620];
    let mut draw = || {
        slint::platform::update_timers_and_animations();
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 360);
        });
    };
    draw();
    let item = input(
        ItemRc::new_root(WindowInner::from_pub(find.window()).component()),
        "File name masks",
    )
    .unwrap();
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    let cursor = slint::LogicalPosition::new(
        origin.x + rect.size.width - 4.,
        origin.y + rect.size.height / 2.,
    );
    assert!(cursor.x > 180. && cursor.x < 360. && cursor.y < 270.);
    find.window()
        .dispatch_event(WindowEvent::PointerMoved { position: cursor });
    draw();
    time.set(Duration::from_millis(600));
    draw();
    let inner = WindowInner::from_pub(find.window());
    let (origin, component) = {
        let popups = inner.active_popups();
        let tooltip = popups
            .iter()
            .find(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
            .expect("Real Find hint must show after timed hover");
        let PopupWindowLocation::ChildWindow(origin) = tooltip.location else {
            panic!("Expected embedded tooltip");
        };
        (origin, tooltip.component.clone())
    };
    let root = ItemRc::new_root(component);
    let popup = root.downcast::<WindowItem>().unwrap();
    let width = popup.as_pin_ref().width().get();
    let height = popup.as_pin_ref().height().get();
    assert!(width > 0. && height > 0.);
    assert!(
        origin.x >= 7.9
            && origin.y >= 7.9
            && origin.x + width <= 352.1
            && origin.y + height <= 612.1,
        "Find hint exceeds its 360x620 host: {origin:?}, {width}x{height}"
    );
    assert!(
        origin.x + width <= cursor.x - 13.9,
        "Right-edge tooltip must flip left with a 14px cursor gap"
    );
    assert!(
        cursor.y < origin.y || cursor.y >= origin.y + height,
        "Tooltip must not cover its cursor"
    );
    find.hide().unwrap();
}
