//! The shipped embedded dialog hosts of a bare main window, driven through their
//! presentation globals exactly as the installed adapters drive them. Form-level
//! checks use this host; adapter lifetime, ownership and backend identity tests
//! install a Presenter instead.
#![allow(dead_code)]
use super::modeless;
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Clip, Flickable},
    lengths::{LogicalPoint, LogicalSize},
    window::{WindowInner, WindowKind},
};
use midnight_ui::MainWindow;
use slint::{
    ComponentHandle, LogicalPosition, SharedString,
    platform::{
        PointerEventButton, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow,
    },
};
use std::{cell::RefCell, rc::Rc};

pub struct Host {
    pub main: MainWindow,
    pub native: Rc<MinimalSoftwareWindow>,
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
}

impl Host {
    /// A bare, active main window; no Presenter or adapter is installed.
    pub fn new() -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
        let main = MainWindow::new().unwrap();
        main.set_preferences_ready(true);
        main.show().unwrap();
        main.window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        let native = windows.borrow()[0].clone();
        modeless::layout(&native, 1340, 820);
        Self {
            main,
            native,
            windows,
        }
    }

    /// Theme, density and fonts as Options applies them to the main window, which
    /// renders every embedded form.
    pub fn apply_preferences(
        &self,
        scheme: i32,
        density: i32,
        family: &str,
        desktop: i32,
        touch: i32,
    ) {
        self.main.invoke_apply_fonts(family.into(), desktop, touch);
        self.main.invoke_apply_preferences(scheme, density);
    }

    /// Run due timers and change handlers; these open and close the embedded popups.
    pub fn settle(&self) {
        slint::platform::update_timers_and_animations();
    }

    /// Size the main window and render it (see [`modeless::draw`]).
    pub fn draw(&self, width: u32, height: u32, name: &str) {
        self.settle();
        modeless::draw(&self.native, width, height, name);
        self.assert_main_adapter_only();
    }

    /// [`Self::draw`] that returns the frame for pixel assertions.
    pub fn render(&self, width: u32, height: u32, name: &str) -> Vec<slint::Rgb8Pixel> {
        self.settle();
        let pixels = modeless::render(&self.native, width, height, name);
        self.assert_main_adapter_only();
        pixels
    }

    /// Size the main window and lay it out without rendering.
    pub fn layout(&self, width: u32, height: u32) {
        self.settle();
        modeless::layout(&self.native, width, height);
    }

    /// Embedded forms never allocate a native window of their own.
    pub fn assert_main_adapter_only(&self) {
        assert_eq!(
            self.windows.borrow().len(),
            1,
            "an embedded form created a native window"
        );
    }

    /// The root of the topmost open form; tooltips are not forms.
    pub fn popup(&self) -> Option<ItemRc> {
        let inner = WindowInner::from_pub(self.main.window());
        let popups = inner.active_popups();
        let popup = popups
            .iter()
            .rev()
            .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))?;
        Some(ItemRc::new_root(popup.component.clone()))
    }

    pub fn is_open(&self) -> bool {
        self.settle();
        self.popup().is_some()
    }

    pub fn try_item(&self, role: AccessibleRole, label: &str) -> Option<ItemRc> {
        modeless::accessible_popup_item(&self.native, role, label)
    }

    pub fn item(&self, role: AccessibleRole, label: &str) -> ItemRc {
        self.try_item(role, label)
            .unwrap_or_else(|| panic!("the open form has no {label}"))
    }

    /// The visible control with this role and label; compact and normal layouts can
    /// both hold one, and only one of them is shown.
    pub fn visible_item(&self, role: AccessibleRole, label: &str) -> ItemRc {
        fn find(item: ItemRc, role: AccessibleRole, label: &str) -> Option<ItemRc> {
            if item.accessible_role() == role
                && item.is_visible()
                && item
                    .accessible_string_property(AccessibleStringProperty::Label)
                    .is_some_and(|value| value == label)
            {
                return Some(item);
            }
            let mut child = item.first_child();
            while let Some(item) = child {
                child = item.next_sibling();
                if let Some(found) = find(item, role, label) {
                    return Some(found);
                }
            }
            None
        }
        self.popup()
            .and_then(|root| find(root, role, label))
            .unwrap_or_else(|| panic!("the open form shows no {label}"))
    }

    /// Click a button of the open form at its center.
    pub fn click(&self, label: &str) {
        modeless::click(&self.native, label);
        self.settle();
    }

    pub fn key(&self, text: impl Into<SharedString>) {
        self.settle();
        let text = text.into();
        self.native
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        self.native
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text });
        self.settle();
    }

    /// A vertical pointer drag from `start`, e.g. on a resize edge of the open dialog.
    pub fn drag(&self, start: LogicalPosition, dy: f32) {
        let end = LogicalPosition::new(start.x, start.y + dy);
        let button = PointerEventButton::Left;
        for event in [
            WindowEvent::PointerMoved { position: start },
            WindowEvent::PointerPressed {
                position: start,
                button,
            },
            WindowEvent::PointerMoved { position: end },
            WindowEvent::PointerReleased {
                position: end,
                button,
            },
        ] {
            self.native.window().dispatch_event(event);
            self.settle();
        }
    }

    /// Enlarge the open dialog, which shows `inside`, with its top and then bottom
    /// resize edges up to the frame margins, as a user makes it taller.
    pub fn stretch_vertically(&self, inside: &ItemRc) {
        let (origin, size) = body(inside);
        self.drag(
            LogicalPosition::new(origin.x + size.width / 2.0, origin.y + 2.0),
            -10_000.0,
        );
        let (origin, size) = body(inside);
        self.drag(
            LogicalPosition::new(origin.x + size.width / 2.0, origin.y + size.height - 2.0),
            10_000.0,
        );
    }
}

/// The dialog body that shows `item`: its outermost clipping frame below the popup
/// root, in window coordinates.
pub fn body(item: &ItemRc) -> (LogicalPoint, LogicalSize) {
    let mut frame = None;
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
        if parent.is_some()
            && ancestor
                .downcast::<Clip>()
                .is_some_and(|clip| clip.as_pin_ref().clip())
        {
            frame = Some(ancestor);
        }
    }
    let frame = frame.expect("an embedded form is shown inside its clipped dialog body");
    let geometry = frame.geometry();
    (frame.map_to_native_window(geometry.origin), geometry.size)
}

/// Assert that `item` is visible and lies inside its dialog body.
pub fn assert_inside(item: &ItemRc, label: &str) {
    assert!(
        item.is_visible(),
        "{label} is hidden or scrolled out of view"
    );
    let (origin, size) = body(item);
    let rect = item.geometry();
    let at = item.map_to_native_window(rect.origin);
    assert!(
        rect.width() > 0.0
            && rect.height() > 0.0
            && at.x >= origin.x - 0.5
            && at.y >= origin.y - 0.5
            && at.x + rect.width() <= origin.x + size.width + 0.5
            && at.y + rect.height() <= origin.y + size.height + 0.5,
        "{label} is outside its dialog body {origin:?} {size:?}: {at:?} {rect:?}"
    );
}

/// [`assert_inside`] that also rejects a control hidden by any scroll viewport.
pub fn assert_unclipped(item: &ItemRc, label: &str) {
    assert_inside(item, label);
    let rect = item.geometry();
    let at = item.map_to_native_window(rect.origin);
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if ancestor.downcast::<Flickable>().is_some() {
            let clip = ancestor.geometry();
            let origin = ancestor.map_to_native_window(clip.origin);
            assert!(
                at.x >= origin.x - 1.0
                    && at.y >= origin.y - 1.0
                    && at.x + rect.width() <= origin.x + clip.width() + 1.0
                    && at.y + rect.height() <= origin.y + clip.height() + 1.0,
                "{label} is inside its dialog but hidden by a scroll viewport"
            );
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
}
