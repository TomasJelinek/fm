//! Polish batch 3, the U-D07 leftover (docs/developer/acceptance/ui-polish.md): the
//! Checksums window's Verify page path field selects with the theme's colours, at
//! least 4.5:1 in every theme, as the other name and path fields do, and keeps its
//! label, tooltip, height and behaviour.
#[path = "support/window_input.rs"]
mod input;
#[path = "support/window_polish.rs"]
mod polish;
use crate::support::modeless;

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, TextInput},
    window::{WindowInner, WindowKind},
};
use midnight_ui::{ChecksumWindow, Theme};
use slint::{
    Color, ComponentHandle,
    platform::{
        Key, Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

const FIELD: &str = "Checksum file to verify";

fn luminance(color: Color) -> f64 {
    let linear = |channel: u8| {
        let channel = f64::from(channel) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.red()) + 0.7152 * linear(color.green()) + 0.0722 * linear(color.blue())
}

fn contrast(first: Color, second: Color) -> f64 {
    let (first, second) = (luminance(first), luminance(second));
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

/// `top` drawn with `alpha` over `under`.
fn blend(top: Color, alpha: f64, under: Color) -> Color {
    let mix = |top: u8, under: u8| {
        (f64::from(top) * alpha + f64::from(under) * (1.0 - alpha)).round() as u8
    };
    Color::from_rgb_u8(
        mix(top.red(), under.red()),
        mix(top.green(), under.green()),
        mix(top.blue(), under.blue()),
    )
}

/// The first text input at or below `item`.
fn text_input(item: &ItemRc) -> Option<ItemRc> {
    if item.downcast::<TextInput>().is_some() {
        return Some(item.clone());
    }
    let mut child = item.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if let Some(found) = text_input(&current) {
            return Some(found);
        }
    }
    None
}

/// Software windows whose Slint timers (tooltip delays) follow a virtual clock.
struct Clocked {
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    clock: Rc<Cell<Duration>>,
}

impl Platform for Clocked {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        self.windows.borrow_mut().push(window.clone());
        Ok(window)
    }
    fn duration_since_start(&self) -> Duration {
        self.clock.get()
    }
}

/// The Checksums window on its Verify page, and its virtual clock.
fn verify_page() -> (
    ChecksumWindow,
    Rc<MinimalSoftwareWindow>,
    Rc<Cell<Duration>>,
) {
    let windows = Rc::new(RefCell::new(Vec::new()));
    let clock = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(Clocked {
        windows: windows.clone(),
        clock: clock.clone(),
    }))
    .unwrap();
    let window = ChecksumWindow::new().unwrap();
    window.set_manifest_path("C:\\checksums\\文档 list.sha256".into());
    window.set_page(1);
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    (window, native, clock)
}

#[test]
fn the_verify_path_field_selects_with_theme_colours_in_every_theme() {
    let (window, native, _) = verify_page();
    let mut findings = Vec::new();
    for scheme in midnight_core::PreferenceTheme::ALL.map(|theme| theme.index()) {
        window.invoke_apply_preferences(scheme, 0, "".into(), 12, 15);
        modeless::layout(&native, 860, 620);
        let field = modeless::accessible_item(&native, AccessibleRole::TextInput, FIELD)
            .expect("the checksum file field");
        let input = text_input(&field).expect("the field's text input");
        let input = input.downcast::<TextInput>().unwrap();
        let input = input.as_pin_ref();
        let theme = window.global::<Theme>();
        let (foreground, background) = (
            input.selection_foreground_color(),
            input.selection_background_color(),
        );
        // As the polish gate measures it: the selection foreground at the text's
        // opacity over the selection background.
        let alpha = f64::from(input.color().color().alpha()) / 255.0;
        let shown = blend(foreground, alpha, background);
        let ratio = contrast(shown, background);
        if (foreground, background)
            != (theme.get_selection_text(), theme.get_selection_background())
            || ratio < 4.5
        {
            findings.push(format!(
                "theme {scheme}: selected text {shown:?} on {background:?} ({ratio:.2}:1), the theme's {:?} on {:?}",
                theme.get_selection_text(),
                theme.get_selection_background()
            ));
        }
    }
    window.hide().unwrap();
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn the_verify_path_field_keeps_its_label_tooltip_height_and_behaviour() {
    let (window, native, clock) = verify_page();
    let verified = Rc::new(RefCell::new(Vec::<String>::new()));
    let sink = verified.clone();
    window.on_verify_manifest(move |path| sink.borrow_mut().push(path.to_string()));
    for (scheme, density, font) in [(0, 0, 12), (1, 1, 15), (2, 0, 20)] {
        window.invoke_apply_preferences(scheme, density, "".into(), font, font);
        modeless::draw(&native, 860, 620, "checksum-verify-field");
        let field = modeless::accessible_item(&native, AccessibleRole::TextInput, FIELD)
            .expect("the checksum file field keeps its accessible label");
        let control = window.global::<Theme>().get_control_height();
        let height = field.geometry().height();
        assert!(
            (height - control).abs() <= 0.5,
            "theme {scheme}, density {density}, font {font}: the field is {height}px high, a control {control}px"
        );
        // Browse stays beside it, to its right.
        let browse =
            modeless::accessible_item(&native, AccessibleRole::Button, "Browse checksum file")
                .expect("Browse");
        let field_end =
            field.map_to_native_window(field.geometry().origin).x + field.geometry().width();
        let browse_start = browse.map_to_native_window(browse.geometry().origin).x;
        assert!(
            browse_start >= field_end && browse_start - field_end <= 9.0,
            "Browse follows the field: {field_end} then {browse_start}"
        );
    }
    // Its tooltip still explains the formats.
    let field = modeless::accessible_item(&native, AccessibleRole::TextInput, FIELD).unwrap();
    let rect = field.geometry();
    let origin = field.map_to_native_window(rect.origin);
    let centre = slint::LogicalPosition::new(
        origin.x + rect.width() / 2.0,
        origin.y + rect.height() / 2.0,
    );
    native
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position: centre });
    clock.set(clock.get() + Duration::from_millis(1200));
    slint::platform::update_timers_and_animations();
    modeless::draw(&native, 860, 620, "checksum-verify-field-tooltip");
    let mut tips = Vec::new();
    for popup in WindowInner::from_pub(native.window())
        .active_popups()
        .iter()
    {
        if matches!(popup.window_kind, WindowKind::ToolTip) {
            fn texts(item: ItemRc, found: &mut Vec<String>) {
                if let Some(text) = item.downcast::<i_slint_core::items::ComplexText>() {
                    found.push(text.as_pin_ref().text().to_string());
                }
                let mut child = item.first_child();
                while let Some(current) = child {
                    child = current.next_sibling();
                    texts(current, found);
                }
            }
            texts(ItemRc::new_root(popup.component.clone()), &mut tips);
        }
    }
    assert!(
        tips.iter()
            .any(|tip| tip.contains("SFV") && tip.contains("relative to this file")),
        "the field's tooltip: {tips:?}"
    );
    native.window().dispatch_event(WindowEvent::PointerExited);
    // Typing replaces the path, and Enter in the field verifies it.
    input::click(&native, &field);
    input::chord(&native, &[Key::Control], "a");
    input::key(&native, "other 文档.md5");
    input::key(&native, Key::Return);
    assert_eq!(window.get_manifest_path(), "other 文档.md5");
    assert_eq!(verified.borrow().as_slice(), ["other 文档.md5"]);
    // A running verification leaves the field disabled.
    window.set_running(true);
    modeless::draw(&native, 860, 620, "checksum-verify-field-running");
    let field = modeless::accessible_item(&native, AccessibleRole::TextInput, FIELD).unwrap();
    assert_eq!(
        field.accessible_string_property(
            i_slint_core::accessibility::AccessibleStringProperty::Enabled
        ),
        Some("false".into()),
        "a running verification disables the field"
    );
    window.hide().unwrap();
}

/// The owner's report of 24 November: on the Verify page the "…" browse button stood
/// taller than the path field beside it (32 px beside 24 px with the desktop density),
/// reached below it, and drew its dots near its bottom edge. The alignment checks of
/// the polish gate (U-K32 row alignment, U-K33 caption centring, U-K35 companion
/// button) flag it; the page passes every layout and alignment check of the gate in
/// both densities, with the default and a large font, in a dark and a light theme.
#[test]
fn the_verify_page_lines_up_its_field_and_browse_button() {
    let (window, native, _) = verify_page();
    let mut findings = Vec::new();
    for (scheme, density, font) in [(1, 0, 12), (2, 1, 15), (1, 0, 24), (2, 1, 24), (0, 0, 32)] {
        window.invoke_apply_preferences(scheme, density, "".into(), font, font);
        let pixels = modeless::render(&native, 860, 620, "checksum-verify-alignment");
        let field = modeless::accessible_item(&native, AccessibleRole::TextInput, FIELD)
            .expect("the checksum file field");
        let browse =
            modeless::accessible_item(&native, AccessibleRole::Button, "Browse checksum file")
                .expect("Browse");
        let (field, browse) = (modeless::edges(&field), modeless::edges(&browse));
        let case = format!("theme {scheme}, density {density}, font {font}");
        if (field.1 - browse.1).abs() > 1.0 || (field.3 - browse.3).abs() > 1.0 {
            findings.push(format!("{case}: field {field:?}, Browse {browse:?}"));
        }
        if ((browse.2 - browse.0) - (browse.3 - browse.1)).abs() > 1.0 {
            findings.push(format!("{case}: Browse is not square: {browse:?}"));
        }
        findings.extend(
            polish::layout_findings(native.window())
                .into_iter()
                .chain(polish::alignment_findings(native.window(), &pixels))
                .map(|finding| format!("{case}: {finding}")),
        );
    }
    window.hide().unwrap();
    findings.sort();
    findings.dedup();
    assert!(findings.is_empty(), "{findings:#?}");
}
