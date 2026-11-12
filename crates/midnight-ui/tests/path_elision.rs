//! Path omissions are explicit while native targets and complete values survive.
#[path = "support/modeless.rs"]
mod ui;
use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, SimpleText},
};
use slint::{
    ComponentHandle, Model, ModelRc, VecModel,
    platform::{PointerEventButton, WindowAdapter, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

slint::slint! {
    import { PathNavigation } from "../ui/path-navigation.slint";
    import { PathPartRow } from "../ui/navigation-models.slint";
    import { ComparatorWindow } from "../../midnight-compare-ui/ui/comparator.slint";
    import { PathText } from "../ui/path-text.slint";
    import { CompactPicker } from "../ui/controls.slint";
    export { PathPartRow, ComparatorWindow }
    export component PathFixture inherits Window {
        preferred-width: 320px; preferred-height: 80px;
        in property <string> path;
        in property <[PathPartRow]> parts;
        out property <string> selected;
        out property <string> copied;
        out property <string> edit-text: navigation.text;
        out property <bool> editing: navigation.editing;
        navigation := PathNavigation {
            x: 12px; y: 12px; width: parent.width - 24px; height: 32px;
            path: root.path; parts: root.parts;
            selected-part(identity) => { root.selected = identity; }
            copy-part(identity) => { root.copied = identity; }
        }
    }
    export component PathLabelFixture inherits Window {
        preferred-width: 320px; preferred-height: 80px;
        in property <string> path;
        in property <length> text-size: 16px;
        label := PathText {
            x: 12px; y: 12px; width: parent.width - 24px; height: 48px;
            text: root.path; font-size: root.text-size;
        }
    }
    export component PathPickerFixture inherits Window {
        preferred-width: 320px; preferred-height: 200px;
        in property <[string]> paths;
        out property <int> selected: -1;
        public function open() { picker.open(); }
        picker := CompactPicker {
            x: 12px; y: 12px; width: 100px;
            path-labels: true; model: root.paths; label: root.paths[0];
            accessible-name: "Saved paths";
            popup-width: 280px;
            selected(index) => { root.selected = index; }
        }
    }
}

#[test]
fn shortened_path_keeps_the_root_clickable_and_edits_the_unshortened_path() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = PathFixture::new().unwrap();
    let labels = [
        "C:\\",
        "Users\\",
        "Tomas\\",
        "Documents\\",
        "long-project-directory\\",
        "日本語\\",
        "Sources",
    ];
    let mut prefix = String::new();
    let rows = labels
        .iter()
        .enumerate()
        .map(|(index, label)| {
            prefix.push_str(label);
            PathPartRow {
                identity: format!("native-{index}").into(),
                label: (*label).into(),
                prefix: prefix.as_str().into(),
            }
        })
        .collect::<Vec<_>>();
    window.set_path(prefix.as_str().into());
    window.set_parts(ModelRc::new(VecModel::from(rows)));
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    ui::draw(&native, 320, 80, "path-middle-elision");
    let marker = ui::accessible_item(&native, AccessibleRole::Button, "Edit full folder path")
        .expect("the shortened breadcrumb exposes its omission marker");
    let dots = visible_text(marker, "...")
        .expect("the omitted path must be marked by three visible literal dots");
    let bounds = dots.geometry();
    let origin = dots.map_to_native_window(bounds.origin);
    assert!(origin.x >= 12.0 && origin.x + bounds.size.width <= 308.0);
    assert_eq!(painted_dot_count(&native, origin, bounds.size), 3);
    fn visible_root(item: i_slint_core::item_tree::ItemRc) -> Option<slint::LogicalPosition> {
        if let Some(label) = item.accessible_string_property(
            i_slint_core::accessibility::AccessibleStringProperty::Label,
        ) && label == "Open C:\\"
        {
            let geometry = item.geometry();
            let origin = item.map_to_native_window(geometry.origin);
            if origin.x >= 12.0 {
                return Some(slint::LogicalPosition::new(
                    origin.x + geometry.size.width / 2.0,
                    origin.y + geometry.size.height / 2.0,
                ));
            }
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            if let Some(point) = visible_root(item) {
                return Some(point);
            }
        }
        None
    }
    let point = visible_root(i_slint_core::item_tree::ItemRc::new_root(
        i_slint_core::window::WindowInner::from_pub(window.window()).component(),
    ))
    .expect("visible leading root");
    for button in [PointerEventButton::Left, PointerEventButton::Right] {
        for event in [
            WindowEvent::PointerMoved { position: point },
            WindowEvent::PointerPressed {
                position: point,
                button,
            },
            WindowEvent::PointerReleased {
                position: point,
                button,
            },
        ] {
            window.window().dispatch_event(event);
        }
    }
    assert_eq!(window.get_selected(), "native-0");
    assert_eq!(window.get_copied(), "native-0");
    ui::click(&native, "Edit full folder path");
    assert!(window.get_editing());
    assert_eq!(window.get_edit_text(), prefix);
}

fn painted_dot_count(
    window: &slint::platform::software_renderer::MinimalSoftwareWindow,
    origin: i_slint_core::lengths::LogicalPoint,
    size: i_slint_core::lengths::LogicalSize,
) -> usize {
    let mut pixels = vec![slint::Rgb8Pixel::default(); 320 * 80];
    window.window().request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 320);
    });
    let x0 = origin.x.ceil() as usize;
    let y0 = origin.y.ceil() as usize;
    let x1 = (origin.x + size.width).floor() as usize;
    let y1 = (origin.y + size.height).floor() as usize;
    let background = pixels[y0 * 320 + x0];
    let mut previous = false;
    let mut count = 0;
    for x in x0..x1 {
        let ink = (y0..y1).any(|y| {
            let pixel = pixels[y * 320 + x];
            pixel.r.abs_diff(background.r) > 40
                || pixel.g.abs_diff(background.g) > 40
                || pixel.b.abs_diff(background.b) > 40
        });
        if ink && !previous {
            count += 1;
        }
        previous = ink;
    }
    count
}

#[test]
fn find_progress_and_result_paths_mark_omissions_without_changing_their_values() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = midnight_ui::FindWindow::new().unwrap();
    let directory = format!("C:\\{}\\文档", "long-directory\\".repeat(30));
    let path = format!("{directory}\\report.txt");
    window.set_current_directory(directory.as_str().into());
    window.set_rows(ModelRc::new(VecModel::from(vec![midnight_ui::FindRow {
        token: "captured-result".into(),
        name: "report.txt".into(),
        extension: "txt".into(),
        path: path.as_str().into(),
        size: "1 B".into(),
        modified: "2026-11-08".into(),
        directory: false,
        ..Default::default()
    }])));
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    ui::draw(&native, 860, 620, "find-path-elision");
    let progress = ui::accessible_item(&native, AccessibleRole::Text, &directory)
        .expect("the complete progress path remains accessible");
    assert!(visible_text(progress, "...").is_some());
    let row = ui::accessible_item(
        &native,
        AccessibleRole::ListItem,
        &format!("report.txt · txt · {path} · 1 B · 2026-11-08"),
    )
    .expect("the captured result retains its complete path");
    assert!(visible_text(row, "...").is_some());
    assert_eq!(window.get_current_directory(), directory);
    assert_eq!(window.get_rows().row_data(0).unwrap().path, path);

    window.set_current_directory("C:\\short".into());
    ui::draw(&native, 860, 620, "find-path-unshortened");
    let progress = ui::accessible_item(&native, AccessibleRole::Text, "C:\\short").unwrap();
    assert!(visible_text(progress, "...").is_none());
}

#[test]
fn comparator_marks_both_shortened_paths_and_keeps_the_full_names() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = ComparatorWindow::new().unwrap();
    let paths = [
        format!("C:\\{}\\left.txt", "a-long-directory\\".repeat(20)),
        format!("C:\\{}\\right.txt", "文档-directory\\".repeat(20)),
    ];
    window.set_left_name(paths[0].as_str().into());
    window.set_right_name(paths[1].as_str().into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    ui::draw(&native, 500, 300, "comparator-path-elision");
    for path in &paths {
        let label = ui::accessible_item(&native, AccessibleRole::Text, path)
            .expect("comparison paths stay complete for accessibility");
        assert!(visible_text(label, "...").is_some());
    }
    assert_eq!(window.get_left_name(), paths[0]);
    assert_eq!(window.get_right_name(), paths[1]);
}

#[test]
fn path_markers_survive_narrow_widths_and_font_changes_and_disappear_when_unneeded() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = PathLabelFixture::new().unwrap();
    let path = format!("C:\\{}\\👩🏽‍💻.txt", "文档 e\u{301}\\".repeat(20));
    window.set_path(path.as_str().into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    for font in [16.0, 30.0] {
        window.set_text_size(font);
        for width in [64, 120, 320] {
            ui::draw(&native, width, 80, "path-marker-narrow");
            let label = ui::accessible_item(&native, AccessibleRole::Text, &path).unwrap();
            let dots = visible_text(label, "...").expect("narrow paths retain the entire marker");
            let bounds = dots.geometry();
            let origin = dots.map_to_native_window(bounds.origin);
            assert!(origin.x >= 12.0 && origin.x + bounds.size.width <= width as f32 - 12.0);
            assert_eq!(window.get_path(), path);
        }
    }
    window.set_path("C:\\短".into());
    ui::draw(&native, 320, 80, "path-marker-unneeded");
    let label = ui::accessible_item(&native, AccessibleRole::Text, "C:\\短").unwrap();
    assert!(visible_text(label, "...").is_none());
}

#[test]
fn path_picker_marks_omissions_and_selects_the_original_full_value() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = PathPickerFixture::new().unwrap();
    let path = format!("C:\\{}\\文档", "long-directory\\".repeat(20));
    window.set_paths(ModelRc::new(VecModel::from(vec![path.as_str().into()])));
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    ui::draw(&native, 320, 200, "path-picker-closed");
    let picker = ui::accessible_item(&native, AccessibleRole::Combobox, "Saved paths").unwrap();
    assert!(visible_text(picker, "...").is_some());
    window.invoke_open();
    ui::draw(&native, 320, 200, "path-picker-open");
    let choice = ui::accessible_popup_item(&native, AccessibleRole::Button, &path).unwrap();
    assert!(visible_text(choice, "...").is_some());
    ui::click(&native, &path);
    assert_eq!(window.get_selected(), 0);
    assert_eq!(window.get_paths().row_data(0).unwrap(), path);
}

#[test]
fn find_root_picker_marks_omissions_and_removes_the_selected_original_root() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = midnight_ui::FindWindow::new().unwrap();
    let paths = [
        format!("C:\\{}\\first", "文档-long\\".repeat(30)),
        format!("D:\\{}\\second", "other-long\\".repeat(30)),
    ];
    let roots = ModelRc::new(VecModel::from(
        paths
            .iter()
            .map(|path| path.as_str().into())
            .collect::<Vec<_>>(),
    ));
    window.set_roots(roots);
    let removed = Rc::new(RefCell::new(Vec::new()));
    let captured = removed.clone();
    window.on_remove_root(move |index| captured.borrow_mut().push(index));
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    ui::draw(&native, 860, 620, "find-root-picker-closed");
    let picker = ui::accessible_item(&native, AccessibleRole::Combobox, "Search folders").unwrap();
    assert!(visible_text(picker.clone(), "...").is_some());
    let bounds = picker.geometry();
    let origin = picker.map_to_native_window(bounds.origin);
    let position = slint::LogicalPosition::new(
        origin.x + bounds.size.width / 2.0,
        origin.y + bounds.size.height / 2.0,
    );
    for event in [
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
        WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        },
    ] {
        native.window().dispatch_event(event);
    }
    ui::draw(&native, 860, 620, "find-root-picker-open");
    let choice = ui::accessible_popup_item(&native, AccessibleRole::Button, &paths[1]).unwrap();
    assert!(visible_text(choice, "...").is_some());
    ui::click(&native, &paths[1]);
    ui::click(&native, "Remove");
    assert_eq!(removed.borrow().as_slice(), [1]);
    assert_eq!(window.get_roots().row_data(1).unwrap(), paths[1]);

    window.set_roots(ModelRc::new(VecModel::from(vec![paths[0].as_str().into()])));
    ui::draw(&native, 860, 620, "find-root-picker-shrunk");
    ui::click(&native, "Remove");
    assert_eq!(removed.borrow().as_slice(), [1, 0]);
    window.set_running(true);
    ui::draw(&native, 860, 620, "find-root-picker-running");
    ui::click(&native, "Remove");
    assert_eq!(removed.borrow().as_slice(), [1, 0]);
}

fn visible_text(item: ItemRc, expected: &str) -> Option<ItemRc> {
    if item.is_visible()
        && (item
            .downcast::<SimpleText>()
            .is_some_and(|text| text.as_pin_ref().text() == expected)
            || item
                .downcast::<ComplexText>()
                .is_some_and(|text| text.as_pin_ref().text() == expected))
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = visible_text(item, expected) {
            return Some(found);
        }
    }
    None
}
