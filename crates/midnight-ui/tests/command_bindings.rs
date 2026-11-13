//! Exercise the installed command bridge, not just shortcut callback routing.
use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, install};
#[path = "support/bitmap.rs"]
mod bitmap;
#[path = "support/ui.rs"]
mod ui;
use slint::{
    ComponentHandle,
    platform::{
        Key, Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn control(window: &MainWindow, text: impl Into<slint::SharedString>) {
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
}

#[test]
fn extension_sort_header_and_ctrl_f4_target_each_panel_and_reverse_order() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let root = std::env::temp_dir();
    let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let sent = Rc::new(RefCell::new(Vec::new()));
    let captured = sent.clone();
    install(
        &window,
        state.clone(),
        Rc::new(move |command| {
            captured.borrow_mut().push(command);
            true
        }),
    );
    midnight_ui::refresh(&window, &state.borrow());
    native.set_size(slint::PhysicalSize::new(1340, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1340.0, 820.0),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); 1340 * 820];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    let headers = sort_headers(&window, "Extension");
    assert_eq!(
        headers.len(),
        2,
        "Each detailed panel must expose an Extension header"
    );
    for pane in [PaneId::Left, PaneId::Right] {
        sent.borrow_mut().clear();
        click_header(&window, &headers[pane.index()]);
        assert!(sent.borrow().iter().any(|command| matches!(command,
            Command::Resort { pane: target, sort: SortSpec { column: SortColumn::Type, descending: false }, .. }
                if *target == pane)));
        assert_eq!(window.get_active_pane(), pane.index() as i32);
        assert_eq!(
            headers[pane.index()]
                .accessible_string_property(AccessibleStringProperty::Value)
                .as_deref(),
            Some("Ascending")
        );
        sent.borrow_mut().clear();
        click_header(&window, &headers[pane.index()]);
        assert!(sent.borrow().iter().any(|command| matches!(command,
            Command::Resort { pane: target, sort: SortSpec { column: SortColumn::Type, descending: true }, .. }
                if *target == pane)), "Clicking Extension again reverses its own panel");
        assert_eq!(
            headers[pane.index()]
                .accessible_string_property(AccessibleStringProperty::Value)
                .as_deref(),
            Some("Descending")
        );
        sent.borrow_mut().clear();
        window.invoke_focus_files();
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1340);
        });
        bitmap::snapshot(
            &pixels,
            1340,
            820,
            &format!("extension-header-{}", pane.index()),
        );
        assert_eq!(window.get_focused_pane(), pane.index() as i32);
        control(&window, Key::F4);
        assert!(sent.borrow().iter().any(|command| matches!(command,
            Command::Resort { pane: target, sort: SortSpec { column: SortColumn::Type, descending: false }, .. }
                if *target == pane)), "Ctrl+F4 must reverse the {:?} panel", pane);
        for (label, column) in [
            ("Name", SortColumn::Name),
            ("Size", SortColumn::Size),
            ("Modified", SortColumn::Modified),
        ] {
            sent.borrow_mut().clear();
            click_header(&window, &sort_headers(&window, label)[pane.index()]);
            assert!(sent.borrow().iter().any(|command| matches!(command,
                Command::Resort { pane: target, sort, .. } if *target == pane && sort.column == column && !sort.descending
            )), "Existing {label} header must retain its sort meaning");
        }
    }
    for density in [0, 1] {
        window.invoke_apply_preferences(0, density);
        window.invoke_apply_fonts("".into(), 32, 32);
        for ratio in [0.0, 1.0] {
            window.set_panel_ratio(ratio);
            native.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, 1340);
            });
            for label in ["Name", "Extension"] {
                let headers = sort_headers(&window, label);
                assert_eq!(headers.len(), 2);
                for header in headers {
                    assert!(
                        header.geometry().width() >= 16.0,
                        "{label} must remain clickable in a narrow panel at font 32 (density {density}, ratio {ratio}); width {}",
                        header.geometry().width()
                    );
                }
            }
            bitmap::snapshot(
                &pixels,
                1340,
                820,
                &format!("extension-header-narrow-{density}-{ratio}"),
            );
        }
    }
    window.hide().unwrap();
}

fn sort_headers(window: &MainWindow, name: &str) -> Vec<ItemRc> {
    fn collect(item: ItemRc, label: &str, result: &mut Vec<ItemRc>) {
        if item.is_visible()
            && item.accessible_role() == AccessibleRole::Button
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|value| value == label)
        {
            result.push(item.clone());
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            collect(item, label, result);
        }
    }
    let mut result = Vec::new();
    collect(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        &format!("Sort by {name}"),
        &mut result,
    );
    result.sort_by(|a, b| {
        a.map_to_native_window(a.geometry().origin)
            .x
            .total_cmp(&b.map_to_native_window(b.geometry().origin).x)
    });
    result
}

fn click_header(window: &MainWindow, item: &ItemRc) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    ui::click(
        window,
        origin.x + rect.width() / 2.0,
        origin.y + rect.height() / 2.0,
    );
}

#[test]
fn control_enter_appends_literal_basename_and_control_p_uses_active_panel() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let left = PathBuf::from("left 文档");
    let right = PathBuf::from("right 👩🏽‍💻");
    let name = "Žlutý ' $(data) & 文档.txt";
    let presenter = Rc::new(RefCell::new(Presenter::new(left.clone(), right.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let sent = commands.clone();
    install(
        &window,
        presenter.clone(),
        Rc::new(move |command| {
            sent.borrow_mut().push(command);
            true
        }),
    );
    presenter.borrow_mut().apply(Event::DesktopContext {
        system_locations: Vec::new(),
        network_position: 0,
        shell: ShellFlavor::Posix,
        locations: vec![],
    });
    let entries: Arc<[Entry]> =
        vec![Entry::new(EntryId(1), left.join(name), EntryKind::File)].into();
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    presenter.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        entries,
        index,
    });
    midnight_ui::refresh(&window, &presenter.borrow());
    native.set_size(slint::PhysicalSize::new(1340, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1340.0, 820.0),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); 1340 * 820];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1340);
    });
    window.invoke_row_click(0, 0, false, false);
    commands.borrow_mut().clear();
    window.invoke_set_command("echo prefix".into(), 0);
    control(&window, Key::Return);
    assert_eq!(
        window.get_command_text().as_str(),
        format!("echo prefix{name}")
    );
    assert!(window.get_command_focused());
    assert!(
        commands.borrow().is_empty(),
        "insertion never opens a file or launches a terminal"
    );

    for (pane, path) in [(0, &left), (1, &right)] {
        window.invoke_activate(pane);
        window.invoke_set_command("echo ".into(), 5);
        control(&window, "p");
        assert_eq!(
            window.get_command_text().as_str(),
            format!(
                "echo {}{}",
                path.to_str().unwrap(),
                std::path::MAIN_SEPARATOR
            )
        );
        assert!(window.get_command_focused());
        assert!(commands.borrow().is_empty());
    }
    for (shortcut, pane, path) in [("[", 1, &left), ("]", 0, &right)] {
        window.invoke_activate(pane);
        window.invoke_set_command("echo  tail".into(), 5);
        control(&window, shortcut);
        assert_eq!(
            window.get_command_text().as_str(),
            format!(
                "echo {}{} tail",
                path.to_str().unwrap(),
                std::path::MAIN_SEPARATOR
            )
        );
        assert_eq!(
            window.get_command_cursor() as usize,
            5 + path.to_str().unwrap().len() + 1
        );
        assert!(window.get_command_focused());
        assert!(commands.borrow().is_empty());
    }
    for (key, pane, path) in [("BracketLeft", 1, &left), ("BracketRight", 0, &right)] {
        for shift in [false, true] {
            for editor_focused in [false, true] {
                window.invoke_activate(pane);
                window.invoke_set_command("echo  tail".into(), 5);
                if !editor_focused {
                    window.invoke_focus_files();
                }
                assert!(window.invoke_physical_shortcut(key.into(), true, false, shift, false));
                assert_eq!(
                    window.get_command_text().as_str(),
                    format!(
                        "echo {}{} tail",
                        path.to_str().unwrap(),
                        std::path::MAIN_SEPARATOR
                    )
                );
                assert!(window.get_command_focused());
                assert!(
                    commands.borrow().is_empty(),
                    "physical insertion must never run a command"
                );
            }
        }
    }
    window.invoke_activate(0);
    window.invoke_set_command("echo ".into(), 5);
    control(&window, "p");
    control(&window, Key::Return);
    assert_eq!(
        window.get_command_text().as_str(),
        format!(
            "echo {}{}{name}",
            left.to_str().unwrap(),
            std::path::MAIN_SEPARATOR
        )
    );
    assert!(commands.borrow().is_empty());
    window.hide().unwrap();
}
