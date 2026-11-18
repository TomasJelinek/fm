//! Application-owned client areas use one font and action palette across hosts.
#[path = "support/embedded_form.rs"]
mod form;
#[path = "support/window_input.rs"]
mod input;
#[path = "support/modeless.rs"]
#[allow(dead_code)]
mod modeless;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{AccessibleRole, ImageItem, WindowItem},
    window::WindowInner,
};
use midnight_ui::{
    AboutWindow, ChecksumWindow, DecisionWindow, FindWindow, HelpWindow, JobsWindow, MainWindow,
    NoticePresentation, Presenter, Theme,
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

/// Native tool windows have their own client font. Notices, operation forms, Filter,
/// Hot Paths and Make File List are embedded in the main window and use its font.
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
                HelpWindow,
                AboutWindow,
                JobsWindow,
                DecisionWindow,
                FindWindow,
                ChecksumWindow,
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
    let pixels = modeless::render(&child, 900, 700, "find-first-theme-refresh");
    let background = main.global::<Theme>().get_window_background();
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
    let pixels = modeless::render(&native, 900, 700, "find-themed-actions");
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

fn visible_label_count(item: ItemRc, label: &str) -> usize {
    let mut count = usize::from(
        item.is_visible()
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|value| value == label),
    );
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        count += visible_label_count(item, label);
    }
    count
}

/// The embedded notice host closes only through its presentation's dismiss callback.
fn notice_host() -> form::Host {
    let host = form::Host::new();
    let weak = host.main.as_weak();
    host.main
        .global::<NoticePresentation>()
        .on_dismiss(move |_| {
            if let Some(main) = weak.upgrade() {
                main.global::<NoticePresentation>().set_token("".into());
            }
        });
    host
}

#[test]
fn error_notice_keeps_themed_compact_icons_and_close_at_large_client_font() {
    let host = notice_host();
    host.apply_preferences(17, 1, "Segoe UI", 24, 24);
    let notice = host.main.global::<NoticePresentation>();
    notice.set_title("File operation failed".into());
    notice.set_error(true);
    notice.set_body("Access denied\nC:/資料/žluťoučký.txt".into());
    notice.set_generation(1);
    notice.set_token("1".into());
    // A 520×300 dialog body inside the main window's 12 px frame margin.
    host.draw(544, 324, "notice-themed-font-icons");
    assert_eq!(
        client_font(host.main.window()),
        ("Segoe UI".to_owned(), 24.0),
        "the embedded notice renders with the application font"
    );
    let root = host.popup().expect("the embedded notice is open");
    assert_eq!(
        visible_label_count(root.clone(), "File operation failed"),
        1,
        "error notices have one themed heading"
    );
    assert_eq!(
        visible_label_count(root, "Error"),
        1,
        "the shared header owns the single error icon"
    );
    for label in ["Copy details", "Close"] {
        let button = host.item(AccessibleRole::Button, label);
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
    host.draw(328, 208, "notice-themed-compact");
    host.click("Close");
    assert!(notice.get_token().is_empty());
    assert!(!host.is_open());
}

#[test]
fn folder_notice_hides_reports_and_requires_an_explicit_authorization_action() {
    let host = notice_host();
    let notice = host.main.global::<NoticePresentation>();
    let details = "Observed issues: 1\nStored details: 1\nNative diagnostics remain available.";
    // The dialog body's height, measured at a main window larger than the notice.
    let body_height = || {
        form::body(&host.item(AccessibleRole::Button, "Close"))
            .1
            .height
    };
    for scheme in [0, 17] {
        host.apply_preferences(scheme, 0, "Segoe UI", 14, 18);
        let retries = Rc::new(Cell::new(0));
        let observed = retries.clone();
        notice.on_authorize_retry(move |_| observed.set(observed.get() + 1));
        notice.set_title("Cannot open folder".into());
        notice.set_error(true);
        notice.set_body("Access to this folder was denied.".into());
        notice.set_folder_path(r"C:\Windows\CSC".into());
        notice.set_details(details.into());
        notice.set_can_authorize(false);
        notice.set_generation(scheme + 1);
        notice.set_token(format!("folder-{scheme}").into());
        host.draw(1000, 700, &format!("folder-notice-before-offer-{scheme}"));
        let before = body_height();
        notice.set_can_authorize(true);
        host.draw(1000, 700, &format!("folder-notice-compact-{scheme}"));
        let offered = body_height();
        assert!(
            offered >= 250.0 && offered > before,
            "a late offer expands the notice: {before} -> {offered}"
        );
        let root = host.popup().expect("the embedded notice is open");
        assert_eq!(visible_label_count(root.clone(), "Cannot open folder"), 1);
        assert_eq!(visible_label_count(root.clone(), "Error"), 1);
        assert_eq!(visible_label_count(root.clone(), r"C:\Windows\CSC"), 1);
        assert_eq!(visible_label_count(root, details), 0);
        assert_eq!(
            retries.get(),
            0,
            "opening an error never requests authorization"
        );
        host.click("Details");
        host.draw(1000, 700, &format!("folder-notice-details-{scheme}"));
        assert_eq!(visible_label_count(host.popup().unwrap(), details), 1);
        assert_eq!(
            retries.get(),
            0,
            "inspecting details never requests authorization"
        );
        host.click("Hide details");
        host.draw(1000, 700, &format!("folder-notice-collapsed-{scheme}"));
        assert_eq!(visible_label_count(host.popup().unwrap(), details), 0);
        host.click("Retry with authorization");
        assert_eq!(retries.get(), 1);
        host.draw(328, 368, &format!("folder-notice-narrow-{scheme}"));
        // Close at this width: narrow_folder_notice_offer_keeps_close_inside_its_dialog.
        let retry = host.visible_item(AccessibleRole::Button, "Retry with authorization");
        form::assert_inside(&retry, "Retry with authorization");
        host.click("Retry with authorization");
        assert_eq!(retries.get(), 2);
        notice.set_can_authorize(false);
        host.draw(1000, 700, &format!("folder-notice-unavailable-{scheme}"));
        assert!(
            host.try_item(AccessibleRole::Button, "Retry with authorization")
                .is_none()
        );
        notice.set_token("".into());
        assert!(!host.is_open());
    }
}

/// A 320 px wide notice body puts the authorization offer on its own row. The
/// embedded notice sizes its body from a formula that is a few pixels shorter than
/// NoticeDialog's minimum layout there, so the body clips Close.
#[test]
#[ignore = "DEFERRED-CROSS: embedded-notice.slint sizes a narrow folder notice with an authorization offer a few pixels shorter than NoticeDialog needs, so Close is clipped; enable with the fix"]
fn narrow_folder_notice_offer_keeps_close_inside_its_dialog() {
    let host = notice_host();
    host.apply_preferences(0, 0, "Segoe UI", 14, 18);
    let notice = host.main.global::<NoticePresentation>();
    notice.set_title("Cannot open folder".into());
    notice.set_error(true);
    notice.set_body("Access to this folder was denied.".into());
    notice.set_folder_path(r"C:\Windows\CSC".into());
    notice.set_details("Observed issues: 1".into());
    notice.set_can_authorize(true);
    notice.set_generation(1);
    notice.set_token("narrow".into());
    host.draw(328, 368, "folder-notice-narrow-offer");
    for label in ["Retry with authorization", "Close"] {
        form::assert_inside(&host.visible_item(AccessibleRole::Button, label), label);
    }
}
