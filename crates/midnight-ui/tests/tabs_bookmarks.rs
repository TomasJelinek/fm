//! Real keyboard and pointer routing for tab identities and persistent bookmarks.
use crate::ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty, item_tree::ItemRc, items::AccessibleRole,
    window::WindowInner,
};
use midnight_ui::{BookmarkRow, MainWindow, NavigationChoiceRow, TabRow};
use slint::{
    ComponentHandle, ModelRc, SharedString, VecModel,
    platform::{
        Key, Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::RefCell, rc::Rc};

use crate::platform_keys;

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}
fn chord(window: &MainWindow, modifiers: &[Key], text: impl Into<SharedString>) {
    for modifier in modifiers {
        window.window().dispatch_event(WindowEvent::KeyPressed {
            text: (*modifier).into(),
        });
    }
    platform_keys::key(window, text.into());
    for modifier in modifiers.iter().rev() {
        window.window().dispatch_event(WindowEvent::KeyReleased {
            text: (*modifier).into(),
        });
    }
}
fn click(window: &MainWindow, x: f32, y: f32) {
    let position = slint::LogicalPosition::new(x, y);
    pointer_click(window, position, PointerEventButton::Left);
}
fn pointer_click(
    window: &MainWindow,
    position: slint::LogicalPosition,
    button: PointerEventButton,
) {
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
fn tab(identity: &str, label: &str, active: bool) -> TabRow {
    TabRow {
        identity: identity.into(),
        label: label.into(),
        path: format!("D:\\文档\\{label}").into(),
        active,
    }
}

fn close_tab_hint(label: &str) -> String {
    format!(
        "Close tab: {label}\nD:\\文档\\{label}\nCtrl+W closes the active tab. The last tab stays open."
    )
}

fn active_overflow_tab(window: &MainWindow) -> slint::LogicalPosition {
    fn find(item: ItemRc) -> Option<ItemRc> {
        if item.accessible_role() == AccessibleRole::Button
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|value| {
                    value == "Long folder title 文档\nD:\\文档\\Long folder title 文档"
                })
            && item
                .accessible_string_property(AccessibleStringProperty::Checked)
                .is_some_and(|value| value == "true")
        {
            return Some(item);
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            if let Some(found) = find(item) {
                return Some(found);
            }
        }
        None
    }
    let item = find(ItemRc::new_root(
        WindowInner::from_pub(window.window()).component(),
    ))
    .expect("the active folder tab remains accessible among equal captions");
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    assert!(origin.x >= window.get_left_list_left() - 1.0);
    assert!(
        origin.x + rect.size.width <= window.get_left_list_left() + window.get_left_list_width()
    );
    slint::LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    )
}

#[test]
fn tabs_and_bookmark_shortcuts_follow_actual_file_focus() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    midnight_ui::install_file_keys(&window);
    window.set_left_tabs(ModelRc::new(VecModel::from(vec![
        tab("11", "A", true),
        tab("12", "B", false),
    ])));
    window.set_right_tabs(ModelRc::new(VecModel::from(vec![
        tab("21", "C", true),
        tab("22", "D", false),
    ])));
    window.set_bookmarks(ModelRc::new(VecModel::from(vec![BookmarkRow {
        identity: "31".into(),
        label: "文档 [work]".into(),
        path: "D:\\文档\\work".into(),
        slot: 1,
    }])));
    window.set_left_path("D:\\文档".into());
    window.set_left_bookmark_name("文档".into());
    window.set_hot_path_choices(ModelRc::new(VecModel::from(vec![NavigationChoiceRow {
        identity: "manage-hot-paths".into(),
        label: "Manage Hot Paths…".into(),
        ..Default::default()
    }])));
    let events = Rc::new(RefCell::new(Vec::<String>::new()));
    let captured = events.clone();
    window.on_tab_new(move |pane| captured.borrow_mut().push(format!("new:{pane}")));
    let captured = events.clone();
    window.on_tab_cycle(move |pane, direction| {
        captured
            .borrow_mut()
            .push(format!("cycle:{pane}:{direction}"))
    });
    let captured = events.clone();
    window.on_tab_close_active(move |pane| {
        captured.borrow_mut().push(format!("close-active:{pane}"))
    });
    let captured = events.clone();
    window.on_tab_number(move |pane, number| {
        captured
            .borrow_mut()
            .push(format!("tab-number:{pane}:{number}"))
    });
    let captured = events.clone();
    window.on_bookmark_jump(move |pane, slot| {
        captured.borrow_mut().push(format!("jump:{pane}:{slot}"))
    });
    let captured = events.clone();
    window.on_bookmark_slot(move |pane, slot| {
        captured.borrow_mut().push(format!("slot:{pane}:{slot}"))
    });
    let captured = events.clone();
    window.on_bookmark_add(move |pane, name, slot| {
        captured
            .borrow_mut()
            .push(format!("add:{pane}:{name}:{slot}"))
    });
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    native.set_size(slint::PhysicalSize::new(1340, 820));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1340.0, 820.0),
    });
    window.invoke_focus_files();
    chord(&window, &[Key::Control], "t");
    chord(&window, &[Key::Control], "w");
    chord(&window, &[Key::Control], Key::Tab);
    assert!(window.get_command_focused());
    chord(&window, &[Key::Control], Key::Tab);
    chord(&window, &[Key::Control, Key::Shift], Key::Tab);
    chord(&window, &[Key::Control], "1");
    chord(&window, &[Key::Shift], "0");
    chord(&window, &[Key::Control, Key::Alt], "2");
    chord(&window, &[Key::Shift, Key::Alt], "3");
    chord(&window, &[Key::Control, Key::Shift], "0");
    chord(&window, &[Key::Control, Key::Shift], "1");
    chord(&window, &[Key::Control, Key::Shift], "9");
    chord(&window, &[Key::Control, Key::Shift], "+");
    assert_eq!(
        *events.borrow(),
        [
            "new:0",
            "close-active:0",
            "cycle:0:-1",
            "jump:0:1",
            "jump:0:10",
            "jump:1:2",
            "jump:1:3",
            "slot:0:10",
            "tab-number:0:1",
            "tab-number:0:9",
            "add:0:文档:0"
        ]
    );
    assert_eq!(window.get_focused_pane(), 0);
    window.set_active_pane(1);
    window.invoke_focus_files();
    chord(&window, &[Key::Control], "1");
    assert_eq!(events.borrow().last().unwrap(), "jump:1:1");
    window.set_native_shortcuts_authoritative(true);
    let native_count = events.borrow().len();
    chord(&window, &[Key::Control], "1");
    chord(&window, &[Key::Control, Key::Alt], "2");
    chord(&window, &[Key::Shift], "3");
    assert_eq!(
        events.borrow().len(),
        native_count,
        "native physical handling must disable logical fallback, including rejected AltGr chords"
    );
    window.set_native_shortcuts_authoritative(false);
    window.invoke_focus_command();
    let count = events.borrow().len();
    for (mods, text) in [
        (vec![Key::Control], "t"),
        (vec![Key::Control], "w"),
        (vec![Key::Control], "1"),
        (vec![Key::Control, Key::Shift], "1"),
    ] {
        chord(&window, &mods, text);
    }
    assert_eq!(
        events.borrow().len(),
        count,
        "navigation shortcuts must not steal command editing"
    );
    chord(&window, &[Key::Control], Key::Tab);
    assert_eq!(window.get_focused_pane(), 1);
    chord(&window, &[Key::Control], "g");
    chord(&window, &[Key::Control], "t");
    chord(&window, &[Key::Control], "1");
    assert_eq!(
        events.borrow().len(),
        count,
        "navigation shortcuts must not steal path editing"
    );
    let selected = Rc::new(RefCell::new(Vec::new()));
    let captured = selected.clone();
    window.on_tab_select(move |pane, identity| captured.borrow_mut().push((pane, identity)));
    let closed = Rc::new(RefCell::new(Vec::new()));
    let captured = closed.clone();
    window.on_tab_close(move |pane, identity| captured.borrow_mut().push((pane, identity)));
    let weak = window.as_weak();
    window.on_activate(move |pane| weak.upgrade().unwrap().set_active_pane(pane));
    let mut pixels = vec![slint::Rgb8Pixel::default(); 1340 * 820];
    let draw = |pixels: &mut [slint::Rgb8Pixel]| {
        native.draw_if_needed(|renderer| {
            renderer.render(pixels, 1340);
        });
    };
    draw(&mut pixels);
    ui::click_root(&window, "B\nD:\\文档\\B", AccessibleRole::Button);
    assert_eq!(*selected.borrow(), vec![(0, SharedString::from("12"))]);
    assert_eq!(window.get_focused_pane(), 0);
    draw(&mut pixels);
    let position = ui::root_position(&window, "B\nD:\\文档\\B", AccessibleRole::Button);
    pointer_click(&window, position, PointerEventButton::Middle);
    assert_eq!(*closed.borrow(), vec![(0, SharedString::from("12"))]);
    assert_eq!(
        selected.borrow().len(),
        1,
        "middle-click must only close its tab"
    );
    closed.borrow_mut().clear();
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Middle,
    });
    window.set_left_tabs(ModelRc::new(VecModel::from(vec![
        tab("12", "B", false),
        tab("11", "A", true),
    ])));
    draw(&mut pixels);
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Middle,
        });
    assert!(
        closed.borrow().is_empty(),
        "reordering during middle press must not close a tab"
    );
    window.set_left_tabs(ModelRc::new(VecModel::from(vec![
        tab("11", "A", true),
        tab("12", "B", false),
    ])));
    draw(&mut pixels);
    let middle_close = ui::root_position(&window, &close_tab_hint("B"), AccessibleRole::Button);
    pointer_click(&window, middle_close, PointerEventButton::Middle);
    assert_eq!(*closed.borrow(), vec![(0, SharedString::from("12"))]);
    closed.borrow_mut().clear();
    let position = ui::root_position(&window, &close_tab_hint("A"), AccessibleRole::Button);
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Middle,
    });
    window.set_left_tabs(ModelRc::new(VecModel::from(vec![
        tab("12", "B", false),
        tab("11", "A", true),
    ])));
    draw(&mut pixels);
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Middle,
        });
    assert!(
        closed.borrow().is_empty(),
        "reordering during middle press on X must not close a tab"
    );
    window.set_left_tabs(ModelRc::new(VecModel::from(vec![
        tab("11", "A", true),
        tab("12", "B", false),
    ])));
    draw(&mut pixels);
    let button = PointerEventButton::Left;
    window
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    window
        .window()
        .dispatch_event(WindowEvent::PointerPressed { position, button });
    window.set_left_tabs(ModelRc::new(VecModel::from(vec![
        tab("12", "B", false),
        tab("11", "A", true),
    ])));
    draw(&mut pixels);
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased { position, button });
    assert!(
        closed.borrow().is_empty(),
        "reordering tabs during a press must not close either identity"
    );
    ui::click_root(&window, &close_tab_hint("B"), AccessibleRole::Button);
    assert_eq!(*closed.borrow(), vec![(0, SharedString::from("12"))]);
    window.set_left_tabs(ModelRc::new(VecModel::from(vec![tab("11", "A", true)])));
    draw(&mut pixels);
    ui::click_root(&window, &close_tab_hint("A"), AccessibleRole::Button);
    let position = ui::root_position(&window, "A\nD:\\文档\\A", AccessibleRole::Button);
    pointer_click(&window, position, PointerEventButton::Middle);
    let close_position = ui::root_position(&window, &close_tab_hint("A"), AccessibleRole::Button);
    pointer_click(&window, close_position, PointerEventButton::Middle);
    assert_eq!(
        closed.borrow().len(),
        1,
        "the last tab must not expose a close action"
    );
    ui::click_root(
        &window,
        "Open a new folder tab (Ctrl+T).",
        AccessibleRole::Button,
    );
    assert_eq!(events.borrow().last().unwrap(), "new:0");
    window.set_left_tabs(ModelRc::new(VecModel::from(
        (0..32)
            .map(|index| {
                tab(
                    &format!("overflow-{index}"),
                    "Long folder title 文档",
                    index == 31,
                )
            })
            .collect::<Vec<_>>(),
    )));
    draw(&mut pixels);
    let position = active_overflow_tab(&window);
    click(&window, position.x, position.y);
    assert_eq!(
        selected.borrow().last(),
        Some(&(0, SharedString::from("overflow-31"))),
        "the last active tab must be revealed across an overflowing strip"
    );
    window.set_left_tabs(ModelRc::new(VecModel::from(
        (0..32)
            .map(|index| {
                tab(
                    &format!("overflow-{index}"),
                    "Long folder title 文档",
                    index == 0,
                )
            })
            .collect::<Vec<_>>(),
    )));
    draw(&mut pixels);
    let position = active_overflow_tab(&window);
    click(&window, position.x, position.y);
    assert_eq!(
        selected.borrow().last(),
        Some(&(0, SharedString::from("overflow-0"))),
        "cycling back must reveal the first tab"
    );
    window.set_left_tabs(ModelRc::new(VecModel::from(vec![tab("11", "A", true)])));
    draw(&mut pixels);
    let captured = events.clone();
    window.on_bookmark_remove(move |identity| {
        captured.borrow_mut().push(format!("remove:{identity}"))
    });
    let captured = events.clone();
    window.on_bookmark_open(move |pane, identity| {
        captured
            .borrow_mut()
            .push(format!("open:{pane}:{identity}"))
    });
    draw(&mut pixels);
    ui::click_root(&window, "Hot Paths", AccessibleRole::Combobox);
    chord(&window, &[], Key::Return); // The final dropdown item opens its manager.
    draw(&mut pixels);
    assert_eq!(
        window.get_focused_pane(),
        -1,
        "the popup must take real editor focus"
    );
    chord(&window, &[Key::Control], "a");
    chord(&window, &[], "Renamed [星] Café");
    chord(&window, &[], Key::Tab);
    chord(&window, &[], Key::DownArrow);
    chord(&window, &[], Key::Tab);
    chord(&window, &[], Key::Return);
    assert_eq!(events.borrow().last().unwrap(), "add:0:Renamed [星] Café:1");
    draw(&mut pixels);
    ui::click_popup(
        &window,
        "Remove bookmark: 文档 [work]\nD:\\文档\\work\nThe folder is kept.",
        AccessibleRole::Button,
    );
    assert_eq!(events.borrow().last().unwrap(), "remove:31");
    assert_eq!(
        window.get_focused_pane(),
        -1,
        "removing a bookmark must keep its popup open"
    );
    ui::click_popup(
        &window,
        "文档 [work]\nD:\\文档\\work\nHot Path Ctrl+1",
        AccessibleRole::Button,
    );
    assert_eq!(events.borrow().last().unwrap(), "open:0:31");
    assert_eq!(
        window.get_focused_pane(),
        0,
        "opening a bookmark restores its panel focus"
    );
    window.hide().unwrap();
}
