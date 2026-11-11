//! Application-owned client areas use one font and action palette across hosts.
#[path = "support/window_input.rs"]
mod input;
#[path = "support/modeless.rs"]
#[allow(dead_code)]
mod modeless;

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ImageItem, WindowItem},
    window::WindowInner,
};
use midnight_ui::{
    AboutWindow, ChecksumWindow, DecisionWindow, FileListWindow, FindWindow, HelpWindow,
    HotPathsWindow, JobsWindow, MainWindow, NoticeWindow, OperationWindow, PanelFilterWindow,
    Presenter, Theme,
};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

fn platform() -> Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>> {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    windows
}

fn client_font(window: &slint::Window) -> (String, f32) {
    let root = ItemRc::new_root(WindowInner::from_pub(window).component());
    let item = root.downcast::<WindowItem>().unwrap();
    (
        item.as_pin_ref().default_font_family().to_string(),
        item.as_pin_ref().default_font_size().get(),
    )
}

#[test]
fn native_client_fonts_match_main_default_and_explicit_application_family() {
    let _windows = platform();
    let main = MainWindow::new().unwrap();
    for family in ["", "Noto Sans 文档"] {
        for density in [0, 1] {
            main.invoke_apply_fonts(family.into(), 13, 21);
            main.invoke_apply_preferences(3, density);
            let expected = client_font(main.window());
            assert!(!expected.0.is_empty());
            macro_rules! check {
                ($($kind:ty),+ $(,)?) => {$(
                    let child = <$kind>::new().unwrap();
                    child.invoke_apply_preferences(3, density, family.into(), 13, 21);
                    assert_eq!(client_font(child.window()), expected, stringify!($kind));
                )+};
            }
            check!(
                NoticeWindow,
                HelpWindow,
                AboutWindow,
                OperationWindow,
                JobsWindow,
                DecisionWindow,
                PanelFilterWindow,
                HotPathsWindow,
                FindWindow,
                ChecksumWindow,
                FileListWindow,
            );
        }
    }
}

#[test]
fn find_adopts_theme_and_font_on_the_same_refresh_without_recreating_its_draft() {
    let windows = platform();
    let main = MainWindow::new().unwrap();
    let path = std::env::temp_dir();
    let state = Rc::new(RefCell::new(Presenter::new(path.clone(), path)));
    midnight_ui::install(&main, state.clone(), Rc::new(|_| true));
    state
        .borrow_mut()
        .apply(midnight_core::Event::PreferencesLoaded(Ok(Box::default())));
    main.show().unwrap();
    midnight_ui::refresh(&main, &state.borrow());
    main.invoke_find_files(0);
    let child = windows.borrow()[1].clone();
    assert!(child.window().is_visible());
    modeless::draw(&child, 900, 700, "find-appearance-before");
    input::click(
        &child,
        &modeless::accessible_item(&child, AccessibleRole::TextInput, "File name masks").unwrap(),
    );
    input::chord(&child, &[Key::Control], "a");
    input::key(&child, "*.žlutý");
    main.invoke_preferences_changed(3, 0, 0);
    modeless::draw(&child, 900, 700, "find-first-theme-refresh");
    let background = main.global::<Theme>().get_window_background();
    let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 700];
    child.window().request_redraw();
    child.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 900);
    });
    assert_eq!(
        (pixels[0].r, pixels[0].g, pixels[0].b),
        (background.red(), background.green(), background.blue())
    );
    main.invoke_font_options_changed(0, 21, 15, 1);
    assert_eq!(
        windows.borrow().len(),
        2,
        "appearance changes keep the live form"
    );
    assert_eq!(client_font(child.window()), client_font(main.window()));
    modeless::draw(&child, 900, 700, "find-appearance-after");
    let entry =
        modeless::accessible_item(&child, AccessibleRole::TextInput, "File name masks").unwrap();
    assert_eq!(
        entry
            .accessible_string_property(
                i_slint_core::accessibility::AccessibleStringProperty::Value
            )
            .unwrap(),
        "*.žlutý"
    );
    main.invoke_close_find_windows();
    main.hide().unwrap();
}

#[test]
fn find_primary_and_close_actions_use_application_colors_and_keep_keyboard_input() {
    let windows = platform();
    let child = FindWindow::new().unwrap();
    child.set_roots(slint::ModelRc::new(slint::VecModel::from(vec![
        "C:/資料".into(),
    ])));
    child.invoke_apply_preferences(3, 0, "".into(), 13, 21);
    let drafts = Rc::new(RefCell::new(Vec::new()));
    let captured = drafts.clone();
    child.on_search(move |draft| captured.borrow_mut().push(draft));
    let stops = Rc::new(Cell::new(0));
    let captured = stops.clone();
    child.on_stop(move || captured.set(captured.get() + 1));
    let weak = child.as_weak();
    child.on_close_session(move || weak.upgrade().unwrap().hide().unwrap());
    child.show().unwrap();
    let native = windows.borrow()[0].clone();
    native
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    modeless::draw(&native, 900, 700, "find-themed-actions");
    let mut pixels = vec![slint::Rgb8Pixel::default(); 900 * 700];
    native.window().request_redraw();
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 900);
    });
    for (label, expected) in [
        ("Find", child.global::<Theme>().get_active_header()),
        ("Close", child.global::<Theme>().get_secondary_surface()),
    ] {
        let item = modeless::accessible_item(&native, AccessibleRole::Button, label).unwrap();
        let at = item.map_to_native_window(item.geometry().origin);
        let pixel = pixels[(at.y as usize + 5) * 900 + at.x as usize + 5];
        assert_eq!(
            (pixel.r, pixel.g, pixel.b),
            (expected.red(), expected.green(), expected.blue()),
            "{label}"
        );
    }
    let checkbox =
        modeless::accessible_item(&native, AccessibleRole::Checkbox, "Subfolders").unwrap();
    input::click(&native, &checkbox);
    modeless::click(&native, "Find");
    assert!(!drafts.borrow().last().unwrap().recursive);
    input::key(&native, Key::Return);
    assert_eq!(
        drafts.borrow().len(),
        2,
        "focused action still supports Enter"
    );
    child.set_running(true);
    child.set_stopping(true);
    modeless::draw(&native, 900, 700, "find-disabled-actions");
    checkbox.accessible_action(&i_slint_core::accessibility::AccessibilityAction::Default);
    let stop = modeless::accessible_item(&native, AccessibleRole::Button, "Stop").unwrap();
    stop.accessible_action(&i_slint_core::accessibility::AccessibilityAction::Default);
    assert_eq!(
        stops.get(),
        0,
        "disabled Stop ignores accessibility activation"
    );
    input::key(&native, Key::Return);
    assert_eq!(
        stops.get(),
        0,
        "disabled actions ignore keyboard and accessibility"
    );
    assert_eq!(drafts.borrow().len(), 2);
    child.set_running(false);
    child.set_stopping(false);
    modeless::draw(&native, 900, 700, "find-keyboard-checkboxes");
    input::click(&native, &checkbox);
    input::key(&native, Key::Tab);
    input::key(&native, Key::Space);
    modeless::click(&native, "Find");
    let draft = drafts.borrow().last().unwrap().clone();
    assert!(
        draft.recursive,
        "disabled checkbox invocation did not change its value"
    );
    assert!(
        draft.content_enabled,
        "Tab follows Subfolders to Containing text in the reformatted query and Space toggles it"
    );
    input::key(&native, Key::Escape);
    assert!(!child.window().is_visible());
}

fn icon_in(item: ItemRc) -> Option<ItemRc> {
    if item.is_visible() && item.downcast::<ImageItem>().is_some() {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = icon_in(item) {
            return Some(found);
        }
    }
    None
}

#[test]
fn error_notice_keeps_themed_compact_icons_and_close_at_large_client_font() {
    let windows = platform();
    let child = NoticeWindow::new().unwrap();
    child.invoke_apply_preferences(17, 1, "Segoe UI".into(), 24, 24);
    child.set_message_title("File operation failed".into());
    child.set_error(true);
    child.set_body("Access denied\nC:/資料/žluťoučký.txt".into());
    let weak = child.as_weak();
    child.on_request_dismiss(move || weak.upgrade().unwrap().hide().unwrap());
    child.show().unwrap();
    let native = windows.borrow()[0].clone();
    modeless::draw(&native, 520, 300, "notice-themed-font-icons");
    assert_eq!(client_font(child.window()), ("Segoe UI".to_owned(), 24.0));
    for label in ["Copy details", "Close"] {
        let button = modeless::accessible_item(&native, AccessibleRole::Button, label).unwrap();
        let icon = icon_in(button.clone()).expect("dialog actions have meaningful compact icons");
        let bounds = button.geometry();
        let at = button.map_to_native_window(bounds.origin);
        let icon_bounds = icon.geometry();
        let icon_at = icon.map_to_native_window(icon_bounds.origin);
        assert!(icon_bounds.width() <= 24.0 && icon_bounds.height() <= 24.0);
        assert!(
            (icon_at.y + icon_bounds.height() / 2.0 - at.y - bounds.height() / 2.0).abs() <= 1.0,
            "{label} icon is centered"
        );
    }
    modeless::draw(&native, 320, 200, "notice-themed-compact");
    modeless::click(&native, "Close");
    assert!(!child.window().is_visible());
}
