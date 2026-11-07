//! Dispatch real Slint keys against the whole window without a backend.
use midnight_ui::{MainWindow, Theme};
use slint::{
    ComponentHandle, ModelRc, SharedString, VecModel,
    platform::{
        Key, Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[cfg(windows)]
#[path = "support/windows_keys.rs"]
mod platform_keys;
#[cfg(not(windows))]
#[path = "support/default_keys.rs"]
mod platform_keys;

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}
fn key(window: &MainWindow, text: impl Into<SharedString>) {
    platform_keys::key(window, text.into());
}
fn hold(window: &MainWindow, modifier: Key, pressed: bool) {
    window.window().dispatch_event(if pressed {
        WindowEvent::KeyPressed {
            text: modifier.into(),
        }
    } else {
        WindowEvent::KeyReleased {
            text: modifier.into(),
        }
    });
}
fn chord(window: &MainWindow, modifiers: &[Key], text: impl Into<SharedString>) {
    for modifier in modifiers {
        hold(window, *modifier, true);
    }
    key(window, text);
    for modifier in modifiers.iter().rev() {
        hold(window, *modifier, false);
    }
}

#[test]
fn salamander_shortcuts_respect_actual_focus_and_modifiers() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    midnight_ui::install_file_keys(&window);
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    native.set_size(slint::PhysicalSize::new(1340, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1340.0, 820.0),
    });
    window.invoke_focus_files();
    let compared = Rc::new(RefCell::new(Vec::new()));
    let captured = compared.clone();
    window.on_compare_files(move |pane| captured.borrow_mut().push(pane));
    chord(&window, &[Key::Control, Key::Shift], "C");
    assert_eq!(
        *compared.borrow(),
        vec![0],
        "Ctrl+Shift+C opens file comparison"
    );
    let sorts = Rc::new(RefCell::new(vec![]));
    let captured = sorts.clone();
    window.on_sort(move |pane, column| captured.borrow_mut().push((pane, column)));
    chord(&window, &[Key::Control], Key::F3);
    assert_eq!(
        *sorts.borrow(),
        vec![(0, 0)],
        "Ctrl+F3 must sort the focused file panel by name"
    );
    for (shortcut, column) in [(Key::F4, 3), (Key::F5, 2), (Key::F6, 1)] {
        chord(&window, &[Key::Control], shortcut);
        assert_eq!(sorts.borrow().last(), Some(&(0, column)));
    }
    window.set_active_pane(1);
    window.invoke_focus_files();
    chord(&window, &[Key::Control], Key::F3);
    assert_eq!(sorts.borrow().last(), Some(&(1, 0)));
    let actions = Rc::new(RefCell::new(vec![]));
    let captured = actions.clone();
    window.on_action(move |pane, action| captured.borrow_mut().push((pane, action)));
    for (modifiers, shortcut, action) in [
        (vec![Key::Control], Key::F9, 3),
        (vec![Key::Control], Key::PageUp, 2),
        (vec![Key::Control], Key::Backspace, 4),
        (vec![Key::Shift], Key::Backspace, 4),
    ] {
        chord(&window, &modifiers, shortcut);
        assert_eq!(actions.borrow().last(), Some(&(1, action)));
    }
    chord(&window, &[Key::Control], "\\");
    assert_eq!(actions.borrow().last(), Some(&(1, 4)));
    let pane_keys = Rc::new(RefCell::new(vec![]));
    let captured = pane_keys.clone();
    window.on_key(move |pane, key| captured.borrow_mut().push((pane, key)));
    chord(&window, &[Key::Control], Key::PageDown);
    assert_eq!(*pane_keys.borrow(), vec![(1, 10)]);
    let sizes = Rc::new(Cell::new(0));
    let captured = sizes.clone();
    window.on_folder_sizes(move |pane| {
        assert_eq!(pane, 1);
        captured.set(captured.get() + 1);
    });
    chord(&window, &[Key::Control, Key::Shift], Key::F10);
    chord(&window, &[Key::Control, Key::Shift], "S");
    assert_eq!(sizes.get(), 2);
    let drive_info = Rc::new(RefCell::new(vec![]));
    let captured = drive_info.clone();
    window.on_drive_info(move |pane| captured.borrow_mut().push(pane));
    chord(&window, &[Key::Control], Key::F1);
    chord(&window, &[Key::Control], "l");
    assert_eq!(*drive_info.borrow(), vec![1, 1]);
    let paths = Rc::new(RefCell::new(vec![]));
    let captured = paths.clone();
    window.on_insert_path(move |pane| captured.borrow_mut().push(pane));
    chord(&window, &[Key::Control], Key::Space);
    assert!(
        paths.borrow().is_empty(),
        "Ctrl+Space belongs to native IME"
    );
    let names = Rc::new(Cell::new(0));
    let captured = names.clone();
    window.on_insert_name(move || captured.set(captured.get() + 1));
    chord(&window, &[Key::Control], Key::Return);
    assert_eq!(names.get(), 1);
    chord(&window, &[Key::Control], "p");
    assert_eq!(*paths.borrow(), vec![1]);
    window.set_active_pane(0);
    window.invoke_focus_files();
    chord(&window, &[Key::Control], "P");
    assert_eq!(*paths.borrow(), vec![1, 0]);
    window.set_active_pane(1);
    window.invoke_focus_files();
    let properties = Rc::new(Cell::new(0));
    let captured = properties.clone();
    window.on_properties(move |_| captured.set(captured.get() + 1));
    chord(&window, &[Key::Alt], Key::Return);
    chord(&window, &[Key::Control, Key::Alt], Key::Return);
    assert_eq!(properties.get(), 2, "AltGr must preserve Properties");
    let action_count = actions.borrow().len();
    for (modifiers, shortcut) in [
        (vec![Key::Control, Key::Shift], Key::Return),
        (vec![Key::Alt, Key::Shift], Key::Return),
        (vec![Key::Meta], Key::Return),
        (vec![Key::Meta], Key::Space),
        (vec![Key::Control, Key::Shift], Key::Backspace),
        (vec![Key::Alt], Key::Backspace),
        (vec![Key::Control, Key::Shift], Key::Space),
        (vec![Key::Alt], Key::Space),
        (vec![Key::Alt], Key::F10),
        (vec![Key::Control, Key::Shift], Key::PageDown),
        (vec![Key::Alt], Key::DownArrow),
        (vec![Key::Meta], Key::End),
    ] {
        chord(&window, &modifiers, shortcut);
    }
    chord(&window, &[Key::Control, Key::Alt], "r");
    chord(&window, &[Key::Control], "q");
    assert_eq!(actions.borrow().len(), action_count);
    assert_eq!(
        *pane_keys.borrow(),
        vec![(1, 10)],
        "unsupported modifiers must not fall through to navigation, open or selection"
    );
    assert_eq!(sizes.get(), 2, "selected-size shortcuts are reserved");
    assert_eq!(names.get(), 1);
    chord(&window, &[Key::Control], Key::Tab);
    assert!(window.get_command_focused());
    chord(&window, &[Key::Control], Key::Space);
    assert_eq!(
        *paths.borrow(),
        vec![1, 0],
        "Ctrl+Space in the editor remains available for IME"
    );
    chord(&window, &[Key::Control], "p");
    assert_eq!(*paths.borrow(), vec![1, 0, 1]);
    chord(&window, &[Key::Control], Key::Return);
    assert_eq!(
        names.get(),
        2,
        "Ctrl+Enter must insert the name from the editor"
    );
    assert_eq!(window.get_command_text().as_str(), "");
    chord(&window, &[Key::Control], Key::Tab);
    assert_eq!(window.get_focused_pane(), 1);
    chord(&window, &[Key::Control], "g");
    assert_eq!(window.get_focused_pane(), -1);
    assert!(!window.get_command_focused());
    chord(&window, &[Key::Control], Key::Tab);
    assert!(
        window.get_command_focused(),
        "Ctrl+Tab focuses the command line from the path editor too"
    );
    chord(&window, &[Key::Control], Key::Tab);
    chord(&window, &[Key::Control], "g");
    let navigated = Rc::new(RefCell::new(vec![]));
    let captured = navigated.clone();
    window.on_navigate(move |pane, path| captured.borrow_mut().push((pane, path)));
    key(&window, "文档");
    key(&window, Key::Return);
    assert_eq!(
        *navigated.borrow(),
        vec![(1, "文档".into())],
        "Ctrl+G must focus the actual path editor"
    );
    chord(&window, &[Key::Control], "[");
    chord(&window, &[Key::Control], "]");
    assert_eq!(*paths.borrow(), vec![1, 0, 1, 0, 1]);
    let launches = Rc::new(Cell::new(0));
    let captured = launches.clone();
    window.on_submit_command(move |_| {
        captured.set(captured.get() + 1);
        false
    });
    window.invoke_set_command("echo safe".into(), 9);
    for modifiers in [
        vec![Key::Control],
        vec![Key::Alt],
        vec![Key::Shift],
        vec![Key::Control, Key::Shift],
        vec![Key::Control, Key::Alt],
        vec![Key::Alt, Key::Shift],
        vec![Key::Meta],
    ] {
        chord(&window, &modifiers, Key::Return);
    }
    assert_eq!(
        launches.get(),
        0,
        "modified Enter must never execute a command"
    );
    assert_eq!(
        names.get(),
        3,
        "Ctrl+Enter in a draft must request filename insertion"
    );
    key(&window, Key::Return);
    assert_eq!(launches.get(), 1);
    window.set_volumes(ModelRc::new(VecModel::from(vec![
        "Drive C: 文档".into(),
        "Drive D: work".into(),
    ])));
    window.set_volume_short_labels(ModelRc::new(VecModel::from(vec!["C:".into(), "D:".into()])));
    window.set_volume_tooltips(ModelRc::new(VecModel::from(vec![
        "C: 文档".into(),
        "D: work".into(),
    ])));
    window.set_left_volume(0);
    window.set_right_volume(0);
    let weak = window.as_weak();
    window.on_activate(move |pane| weak.upgrade().unwrap().set_active_pane(pane));
    let volumes = Rc::new(RefCell::new(vec![]));
    let captured = volumes.clone();
    window.on_volume(move |pane, index| captured.borrow_mut().push((pane, index)));
    let mut pixels = vec![slint::Rgb8Pixel::default(); 1340 * 820];
    let draw = |pixels: &mut [slint::Rgb8Pixel]| {
        native.draw_if_needed(|renderer| {
            renderer.render(pixels, 1340);
        });
    };
    for (shortcut, pane) in [(Key::F1, 0), (Key::F2, 1)] {
        draw(&mut pixels);
        chord(&window, &[Key::Alt], shortcut);
        draw(&mut pixels);
        assert_eq!(window.get_active_pane(), pane);
        key(&window, Key::DownArrow);
        assert_eq!(
            volumes.borrow().len(),
            pane as usize,
            "browsing must not select a volume"
        );
        key(&window, Key::Return);
        assert_eq!(
            volumes.borrow().last(),
            Some(&(pane, 1)),
            "Alt+F1/F2 must open the real picker"
        );
        assert_eq!(window.get_focused_pane(), pane);
        chord(&window, &[Key::Alt], shortcut);
        draw(&mut pixels);
        key(&window, Key::DownArrow);
        window.set_volumes_revision(window.get_volumes_revision() + 1);
        draw(&mut pixels);
        assert_eq!(
            window.get_focused_pane(),
            pane,
            "snapshot invalidation restores actual pane focus"
        );
        key(&window, Key::Escape);
        assert_eq!(
            volumes.borrow().len(),
            pane as usize + 1,
            "revision invalidation must not commit a stale pending volume"
        );
    }
    let exits = Rc::new(Cell::new(0));
    let captured = exits.clone();
    window.on_exit(move || captured.set(captured.get() + 1));
    chord(&window, &[Key::Alt], Key::F4);
    assert_eq!(exits.get(), 1);
    let theme = window.global::<Theme>();
    assert_eq!(theme.get_bottom_height(), 22.0);
    window.invoke_apply_preferences(0, 1);
    assert_eq!(theme.get_bottom_height(), 40.0);
    window.hide().unwrap();
}
