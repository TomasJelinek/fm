//! Shared software-native-window harness for independent tool windows.
#[path = "bitmap.rs"]
pub mod bitmap;
use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use slint::platform::{
    Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{cell::RefCell, rc::Rc};

pub struct Windows(pub Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>);
impl Platform for Windows {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        self.0.borrow_mut().push(window.clone());
        Ok(window)
    }
}

/// Size the native window and run what a frame runs before painting (change handlers
/// and lazy repeater/conditional instantiation) without rendering: enough for geometry
/// assertions and pointer input.
pub fn layout(window: &MinimalSoftwareWindow, width: u32, height: u32) {
    resize(window, width, height);
    WindowInner::from_pub(window.window()).ensure_tree_instantiated();
}

fn resize(window: &MinimalSoftwareWindow, width: u32, height: u32) {
    window.set_size(slint::PhysicalSize::new(width, height));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
}

/// Size the native window, software-render one full frame and keep it as an optional
/// artifact.
pub fn draw(window: &MinimalSoftwareWindow, width: u32, height: u32, name: &str) {
    render(window, width, height, name);
}

/// [`draw`] that returns the rendered frame for pixel assertions.
pub fn render(
    window: &MinimalSoftwareWindow,
    width: u32,
    height: u32,
    name: &str,
) -> Vec<slint::Rgb8Pixel> {
    resize(window, width, height);
    let mut pixels = vec![slint::Rgb8Pixel::default(); width as usize * height as usize];
    window.window().request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width as usize);
    });
    bitmap::snapshot(
        &pixels,
        width as usize,
        height as usize,
        &format!("{name}-{width}x{height}"),
    );
    pixels
}

/// Geometry sweeps render their reference case (and every case when artifacts were
/// requested); the other cases only lay out.
#[allow(dead_code)] // Used by the geometry sweeps only.
pub fn draw_or_layout(
    window: &MinimalSoftwareWindow,
    width: u32,
    height: u32,
    name: &str,
    reference: bool,
) {
    if reference || bitmap::enabled(&format!("{name}-{width}x{height}")) {
        draw(window, width, height, name);
    } else {
        layout(window, width, height);
    }
}
fn find(item: ItemRc, role: AccessibleRole, label: &str) -> Option<ItemRc> {
    if item.accessible_role() == role
        && item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|v| v == label)
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(item) = find(item, role, label) {
            return Some(item);
        }
    }
    None
}
pub fn accessible_item(
    window: &MinimalSoftwareWindow,
    role: AccessibleRole,
    label: &str,
) -> Option<ItemRc> {
    let root = WindowInner::from_pub(window.window()).component();
    find(ItemRc::new_root(root), role, label)
}
#[allow(dead_code)] // The Find and checksum harnesses share this module.
pub fn accessible_popup_item(
    window: &MinimalSoftwareWindow,
    role: AccessibleRole,
    label: &str,
) -> Option<ItemRc> {
    let inner = WindowInner::from_pub(window.window());
    inner
        .active_popups()
        .iter()
        .rev()
        .find_map(|popup| find(ItemRc::new_root(popup.component.clone()), role, label))
}
/// Every item with this role and label, in tree order.
#[allow(dead_code)] // Used where two controls share one label.
pub fn accessible_items(
    window: &MinimalSoftwareWindow,
    role: AccessibleRole,
    label: &str,
) -> Vec<ItemRc> {
    fn collect(item: ItemRc, role: AccessibleRole, label: &str, found: &mut Vec<ItemRc>) {
        if item.accessible_role() == role
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|v| v == label)
        {
            found.push(item.clone());
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            collect(item, role, label, found);
        }
    }
    let mut found = Vec::new();
    let root = WindowInner::from_pub(window.window()).component();
    collect(ItemRc::new_root(root), role, label, &mut found);
    found
}
pub fn click(window: &MinimalSoftwareWindow, label: &str) {
    let item = accessible_popup_item(window, AccessibleRole::Button, label)
        .or_else(|| accessible_item(window, AccessibleRole::Button, label))
        .unwrap_or_else(|| panic!("Missing {label}"));
    click_item(window, &item, label);
}
/// Press and release the left button at the centre of `item`.
pub fn click_item(window: &MinimalSoftwareWindow, item: &ItemRc, label: &str) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    let position = slint::LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    );
    assert!(position.x >= 0.0 && position.y >= 0.0);
    assert!(
        position.x < window.window().size().width as f32
            && position.y < window.window().size().height as f32,
        "{label} is outside the window: {position:?}"
    );
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
}

/// The window rectangle of `item`: left, top, right, bottom.
#[allow(dead_code)] // Used by the footer checks.
pub fn edges(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let at = item.map_to_native_window(rect.origin);
    (at.x, at.y, at.x + rect.width(), at.y + rect.height())
}

/// The part of `item` that its clipping ancestors and the window show.
#[allow(dead_code)] // Used by the footer checks.
pub fn shown_edges(window: &MinimalSoftwareWindow, item: &ItemRc) -> (f32, f32, f32, f32) {
    use i_slint_core::item_tree::ParentItemTraversalMode;
    let size = window
        .window()
        .size()
        .to_logical(window.window().scale_factor());
    let (left, top, right, bottom) = edges(item);
    let (mut left, mut top) = (left.max(0.0), top.max(0.0));
    let (mut right, mut bottom) = (right.min(size.width), bottom.min(size.height));
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if ancestor.borrow().as_ref().clips_children() {
            let clip = edges(&ancestor);
            (left, top) = (left.max(clip.0), top.max(clip.1));
            (right, bottom) = (right.min(clip.2), bottom.min(clip.3));
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    (left, top, right, bottom)
}

/// The visible buttons on a dialog footer's row (the row of its `dismiss` button), left to
/// right, with their labels.
#[allow(dead_code)] // Used by the footer checks.
pub fn footer_row(window: &MinimalSoftwareWindow, dismiss: &str) -> Vec<(String, ItemRc)> {
    fn buttons(item: ItemRc, found: &mut Vec<ItemRc>) {
        if item.accessible_role() == AccessibleRole::Button && item.is_visible() {
            found.push(item.clone());
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            buttons(item, found);
        }
    }
    let dismiss = accessible_item(window, AccessibleRole::Button, dismiss)
        .map(|item| edges(&item))
        .expect("the footer's dismiss button");
    let mut found = Vec::new();
    let root = WindowInner::from_pub(window.window()).component();
    buttons(ItemRc::new_root(root), &mut found);
    let mut row: Vec<(String, ItemRc)> = found
        .into_iter()
        .filter(|button| {
            let (_, top, _, bottom) = edges(button);
            (top - dismiss.1).abs() < 0.5 && (bottom - dismiss.3).abs() < 0.5
        })
        .map(|button| {
            let label = button
                .accessible_string_property(AccessibleStringProperty::Label)
                .unwrap_or_default()
                .to_string();
            (label, button)
        })
        .collect();
    row.sort_by(|a, b| edges(&a.1).0.total_cmp(&edges(&b.1).0));
    row
}

/// The labels of a dialog footer's row, after checking that every button on it is whole
/// and 8 px from the next one, and that `dismiss` ends it.
#[allow(dead_code)] // Used by the footer checks.
pub fn fitted_footer(window: &MinimalSoftwareWindow, dismiss: &str, case: &str) -> Vec<String> {
    let row = footer_row(window, dismiss);
    for (label, button) in &row {
        let (whole, shown) = (edges(button), shown_edges(window, button));
        assert!(
            (whole.0 - shown.0).abs() < 0.5 && (whole.2 - shown.2).abs() < 0.5,
            "{case}: {label} is cut: {whole:?} shows {shown:?}"
        );
    }
    for pair in row.windows(2) {
        let gap = edges(&pair[1].1).0 - edges(&pair[0].1).2;
        assert!(
            (gap - 8.0).abs() < 0.75,
            "{case}: {} | {} are {gap}px apart",
            pair[0].0,
            pair[1].0
        );
    }
    let labels: Vec<String> = row.into_iter().map(|(label, _)| label).collect();
    assert_eq!(
        labels.last().map(String::as_str),
        Some(dismiss),
        "{case}: {labels:?}"
    );
    labels
}
