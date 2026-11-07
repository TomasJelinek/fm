//! New panel controls preserve display values and dispatch stable identities.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ClippedImage, ComplexText, ImageItem, SimpleText},
};
use midnight_ui::{
    DetailRow, FileRow, LocationRow, MainWindow, NavigationChoiceRow, PathPartRow, Theme,
};
use slint::{
    ComponentHandle, ModelRc, SharedString, VecModel,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use ui::{TestUi, chord, click, key};

fn parts(root: &str, folder: &str) -> ModelRc<PathPartRow> {
    ModelRc::new(VecModel::from(vec![
        PathPartRow {
            identity: "root".into(),
            label: root.into(),
            prefix: root.into(),
        },
        PathPartRow {
            identity: "folder".into(),
            label: folder.into(),
            prefix: format!("{root}{folder}").into(),
        },
    ]))
}

fn choose_history(window: &MainWindow) {
    click(
        window,
        window.get_command_history_left() + window.get_command_history_width() / 2.0,
        window.get_command_history_top() + window.get_command_history_height() / 2.0,
    );
    key(window, Key::Return);
}

#[test]
fn breadcrumbs_details_and_command_history_never_reinterpret_display_text() {
    let mut ui = TestUi::new();
    ui.window.set_left_path("C:\\文档".into());
    ui.window.set_left_path_parts(parts("C:\\", "文档"));
    ui.window.set_right_path("D:\\right".into());
    let icon = slint::Image::from_rgb8(slint::SharedPixelBuffer::clone_from_slice(
        &[253, 3, 7],
        1,
        1,
    ));
    ui.window
        .set_left_locations(ModelRc::new(VecModel::from(vec![
            LocationRow {
                identity: "drive".into(),
                compact_label: "C:".into(),
                is_drive: true,
                tooltip: "Test drive".into(),
                icon: icon.clone(),
                ..Default::default()
            },
            LocationRow {
                identity: "documents".into(),
                kind: midnight_ui::LocationRole::Documents,
                tooltip: "Test documents".into(),
                icon: icon.clone(),
                ..Default::default()
            },
        ])));
    ui.window
        .set_navigation_choices(ModelRc::new(VecModel::from(vec![NavigationChoiceRow {
            identity: "drive".into(),
            label: "Test drive".into(),
            compact_label: "C:".into(),
            icon: icon.clone(),
            ..Default::default()
        }])));
    ui.window.set_left_navigation_index(0);
    ui.window.set_left_details(ModelRc::new(VecModel::from(vec![
        DetailRow {
            identity: "entry-1:name".into(),
            label: "Name".into(),
            value: "文档 $(literal).rs".into(),
        },
        DetailRow {
            identity: "entry-1:size".into(),
            label: "Size in bytes".into(),
            value: "123456789 bytes".into(),
        },
    ])));
    ui.window
        .set_left_rows(ModelRc::new(VecModel::from(vec![FileRow {
            identity: "entry-1".into(),
            name: "selected.rs".into(),
            selected: true,
            icon,
            ..Default::default()
        }])));
    let paths = Rc::new(RefCell::new(Vec::new()));
    let captured = paths.clone();
    ui.window
        .on_path_part(move |pane, identity| captured.borrow_mut().push((pane, identity)));
    let copied = Rc::new(RefCell::new(Vec::new()));
    let captured = copied.clone();
    ui.window
        .on_detail_copy(move |pane, identity| captured.borrow_mut().push((pane, identity)));
    let launched = Rc::new(Cell::new(0));
    let captured = launched.clone();
    ui.window.on_submit_command(move |_| {
        captured.set(captured.get() + 1);
        false
    });
    let command = SharedString::from("echo '文档 & $(literal)' > result.txt");
    ui.window
        .set_command_history(ModelRc::new(VecModel::from(vec![NavigationChoiceRow {
            identity: "command-42".into(),
            label: command.clone(),
            ..Default::default()
        }])));
    let weak = ui.window.as_weak();
    let recalled = Rc::new(RefCell::new(Vec::new()));
    let captured = recalled.clone();
    ui.window.on_command_history_choice(move |identity| {
        captured.borrow_mut().push(identity);
        weak.upgrade()
            .unwrap()
            .invoke_set_command(command.clone(), command.len() as i32);
    });
    ui.draw();
    assert_compact_geometry(&mut ui);
    assert_eq!(ui.window.get_command_prompt(), "C:\\文档>");
    let x = ui.window.get_left_path_left() + 12.0;
    let y = ui.window.get_left_path_top() + ui.window.get_left_path_control_height() / 2.0;
    ui.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(x, y),
        });
    ui.draw();
    assert_eq!(ui.window.get_left_highlighted_path(), "C:\\");
    click(&ui.window, x, y);
    assert_eq!(*paths.borrow(), vec![(0, SharedString::from("root"))]);
    click(
        &ui.window,
        ui.window.get_left_details_left() + 30.0,
        ui.window.get_left_details_top() + 10.0,
    );
    assert_eq!(
        *copied.borrow(),
        vec![(0, SharedString::from("entry-1:name"))]
    );
    choose_history(&ui.window);
    assert_eq!(*recalled.borrow(), vec![SharedString::from("command-42")]);
    assert_eq!(
        ui.window.get_command_text(),
        "echo '文档 & $(literal)' > result.txt"
    );
    assert_eq!(launched.get(), 0, "recall must not execute");
    assert!(ui.window.get_command_focused());
    ui.window.invoke_focus_files();
    chord(&ui.window, &[Key::Control], "g");
    chord(&ui.window, &[Key::Control], "a");
    key(&ui.window, "discarded edit");
    assert_eq!(ui.window.get_left_path_draft(), "discarded edit");
    ui.window.set_left_path("C:\\changed".into());
    ui.window.set_left_path_parts(parts("C:\\", "changed"));
    ui.draw();
    assert_eq!(
        ui.window.get_left_path_draft(),
        "C:\\changed",
        "directory changes replace stale path drafts"
    );
    assert_eq!(ui.window.get_command_prompt(), "C:\\changed>");
    ui.window.invoke_focus_files();
    let steps = Rc::new(RefCell::new(Vec::new()));
    let captured = steps.clone();
    ui.window
        .on_action(move |pane, action| captured.borrow_mut().push((pane, action)));
    ui.window
        .set_left_back_history(ModelRc::new(VecModel::from(vec![NavigationChoiceRow {
            identity: "old-history".into(),
            label: "Old folder".into(),
            ..Default::default()
        }])));
    ui.draw();
    assert_history_chevron_centered(&ui.window);
    let back_position = ui::root_position(&ui.window, "Back (Alt+Left)", AccessibleRole::Button);
    ui.window
        .window()
        .dispatch_event(WindowEvent::PointerPressed {
            position: back_position,
            button: PointerEventButton::Left,
        });
    ui.window.set_history_revision(1);
    ui.window
        .set_left_back_history(ModelRc::new(VecModel::from(vec![NavigationChoiceRow {
            identity: "new-history".into(),
            label: "Another tab".into(),
            ..Default::default()
        }])));
    ui.draw();
    ui.window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: back_position,
            button: PointerEventButton::Left,
        });
    assert!(
        steps.borrow().is_empty(),
        "held Back cannot navigate a changed history"
    );
    click(&ui.window, back_position.x, back_position.y);
    assert_eq!(*steps.borrow(), vec![(0, 0)]);
    let history_picks = Rc::new(RefCell::new(Vec::new()));
    let captured = history_picks.clone();
    ui.window.on_history_choice(move |pane, forward, identity| {
        captured.borrow_mut().push((pane, forward, identity));
    });
    ui::click_root(
        &ui.window,
        "Back (Alt+Left) history",
        AccessibleRole::Combobox,
    );
    key(&ui.window, Key::Return);
    assert_eq!(
        *history_picks.borrow(),
        vec![(0, false, SharedString::from("new-history"))]
    );
    let position = slint::LogicalPosition::new(x, y);
    ui.window
        .window()
        .dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
    ui.window
        .set_left_path_parts(ModelRc::new(VecModel::from(vec![PathPartRow {
            identity: "new-root".into(),
            label: "Z:\\".into(),
            prefix: "Z:\\".into(),
        }])));
    ui.draw();
    ui.window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
    assert_eq!(
        paths.borrow().len(),
        1,
        "a changed breadcrumb cannot redirect a held click"
    );
    for scheme in 0..3 {
        ui.window.invoke_apply_preferences(scheme, 0);
        ui.draw();
        let color = ui.window.global::<Theme>().get_selected_edge();
        let expected = slint::Rgb8Pixel {
            r: color.red(),
            g: color.green(),
            b: color.blue(),
        };
        assert!(
            ui.pixels.iter().filter(|pixel| **pixel == expected).count() >= 40,
            "selection has a visible edge in theme {scheme}"
        );
    }
    ui.window.hide().unwrap();
}

fn descendants(item: ItemRc) -> Vec<ItemRc> {
    let mut found = vec![item.clone()];
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        found.extend(descendants(item));
    }
    found
}

fn image_items(item: ItemRc) -> Vec<ItemRc> {
    descendants(item)
        .into_iter()
        .filter(|item| {
            item.downcast::<ImageItem>().is_some() || item.downcast::<ClippedImage>().is_some()
        })
        .collect()
}

fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (origin.x, origin.y, rect.size.width, rect.size.height)
}

fn caption(item: ItemRc) -> ItemRc {
    descendants(item)
        .into_iter()
        .find(|item| {
            item.downcast::<SimpleText>().is_some() || item.downcast::<ComplexText>().is_some()
        })
        .expect("a shaped breadcrumb caption")
}

fn assert_compact_geometry(ui: &mut TestUi) {
    for (density, original_icon, row_height, control_height, toolbar_height) in
        [(0, 14.0, 22.0, 24.0, 34.0), (1, 20.0, 40.0, 40.0, 48.0)]
    {
        ui.window.invoke_apply_preferences(0, density);
        ui.draw();
        let theme = ui.window.global::<Theme>();
        assert_eq!(theme.get_row_height(), row_height);
        assert_eq!(theme.get_control_height(), control_height);
        assert_eq!(theme.get_toolbar_height(), toolbar_height);
        for (label, role, expected) in [
            ("selected.rs", AccessibleRole::ListItem, original_icon * 1.1),
            (
                "Test drive",
                AccessibleRole::Button,
                if density == 0 { 20.0 } else { 26.0 },
            ),
            (
                "Test documents",
                AccessibleRole::Button,
                if density == 0 { 20.0 } else { 26.0 },
            ),
            ("Drive or volume", AccessibleRole::Combobox, 18.0),
        ] {
            let parent = ui::root_item(&ui.window, label, role);
            let image = image_items(parent.clone())
                .into_iter()
                .find(|item| {
                    let rect = item.geometry();
                    rect.size.width > 10.0 && rect.size.height > 10.0
                })
                .expect("the visible host icon");
            let (x, y, width, height) = bounds(&image);
            let (parent_x, parent_y, parent_width, parent_height) = bounds(&parent);
            assert!(
                (width - expected).abs() < 0.05 && (height - expected).abs() < 0.05,
                "{label}: artwork uses its expected size without enlarging its control"
            );
            assert!(x >= parent_x && x + width <= parent_x + parent_width);
            assert!(
                (y + height / 2.0 - parent_y - parent_height / 2.0).abs() < 0.05,
                "{label}: larger artwork remains vertically centered"
            );
        }
        let first = caption(ui::root_item(
            &ui.window,
            "Open C:\\",
            AccessibleRole::Button,
        ));
        let second = caption(ui::root_item(
            &ui.window,
            "Open C:\\文档",
            AccessibleRole::Button,
        ));
        let (first_x, _, first_width, _) = bounds(&first);
        let (second_x, _, _, _) = bounds(&second);
        assert!(
            (first_x + first_width - second_x).abs() < 0.05,
            "Unicode breadcrumb components must meet without inserted spaces or padding"
        );
        let (x, y, width, height) = bounds(&second);
        ui.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(x + width / 2.0, y + height / 2.0),
            });
        ui.draw();
        assert_eq!(ui.window.get_left_highlighted_path(), "C:\\文档");
        ui.window
            .window()
            .dispatch_event(WindowEvent::PointerExited);
        ui.draw();
        assert_history_chevron_centered(&ui.window);
    }
    ui.window.invoke_apply_preferences(0, 0);
    ui.draw();
}

fn assert_history_chevron_centered(window: &MainWindow) {
    let picker = ui::root_item(window, "Back (Alt+Left) history", AccessibleRole::Combobox);
    let chevron = image_items(picker.clone())
        .into_iter()
        .find(|item| {
            let rect = item.geometry();
            (rect.size.width - 9.0).abs() < 0.05 && (rect.size.height - 6.0).abs() < 0.05
        })
        .expect("the history dropdown chevron");
    let (x, y, width, height) = bounds(&chevron);
    let (parent_x, parent_y, parent_width, parent_height) = bounds(&picker);
    assert!(
        (x + width / 2.0 - parent_x - parent_width / 2.0).abs() < 0.05,
        "history chevron must stay centered inside its narrow clickable area"
    );
    assert!((y + height / 2.0 - parent_y - parent_height / 2.0).abs() < 0.05);
}
