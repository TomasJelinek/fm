//! Find uses real input events and a lazy result model even for large result sets.
#[allow(dead_code)]
#[path = "support/modeless.rs"]
mod modeless;

use i_slint_core::{
    accessibility::{AccessibilityAction, AccessibleStringProperty},
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable},
};
use midnight_ui::{FindRow, FindWindow};
use slint::{
    ComponentHandle, Model, ModelNotify, ModelTracker,
    platform::{Key, PointerEventButton, WindowAdapter, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct LazyRows {
    count: usize,
    reads: Cell<usize>,
    notify: ModelNotify,
}

impl Model for LazyRows {
    type Data = FindRow;

    fn row_count(&self) -> usize {
        self.count
    }

    fn row_data(&self, row: usize) -> Option<FindRow> {
        if row >= self.count {
            return None;
        }
        self.reads.set(self.reads.get() + 1);
        Some(FindRow {
            token: format!("7:{}", row + 1).into(),
            name: format!("result-{row:05}.txt").into(),
            path: "D:\\fixtures\\文档".into(),
            ..Default::default()
        })
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}

fn key(native: &impl WindowAdapter, key: Key) {
    native
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
    native
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
}

fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (
        origin.x,
        origin.y,
        origin.x + rect.width(),
        origin.y + rect.height(),
    )
}

fn assert_visible_through_clips(item: &ItemRc) {
    assert!(item.is_visible());
    let area = bounds(item);
    let mut ancestor = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(parent) = ancestor {
        if parent.downcast::<Flickable>().is_some() {
            let clip = bounds(&parent);
            assert!(
                area.1 >= clip.1 - 0.5 && area.3 <= clip.3 + 0.5,
                "control must be wholly visible through its scroll viewport: {area:?}, {clip:?}"
            );
        }
        ancestor = parent.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
}

fn click_control(
    native: &slint::platform::software_renderer::MinimalSoftwareWindow,
    role: AccessibleRole,
    label: &str,
) -> ItemRc {
    let item =
        modeless::accessible_item(native, role, label).unwrap_or_else(|| panic!("missing {label}"));
    for _ in 0..40 {
        let area = bounds(&item);
        let mut ancestor = item.parent_item(ParentItemTraversalMode::StopAtPopups);
        let mut adjustment = None;
        while let Some(parent) = ancestor {
            if parent.downcast::<Flickable>().is_some() {
                let clip = bounds(&parent);
                if area.1 < clip.1 || area.3 > clip.3 {
                    adjustment = Some((clip, if area.1 < clip.1 { 70.0 } else { -70.0 }));
                    break;
                }
            }
            ancestor = parent.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        let Some((clip, delta_y)) = adjustment else {
            break;
        };
        native
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(clip.0 + 25.0, (clip.1 + clip.3) / 2.0),
                delta_x: 0.0,
                delta_y,
            });
        modeless::draw(native, 860, 620, "find-query-scroll");
    }
    assert_visible_through_clips(&item);
    let area = bounds(&item);
    let position = slint::LogicalPosition::new((area.0 + area.2) / 2.0, (area.1 + area.3) / 2.0);
    native.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    native
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
    item
}

fn type_input(
    native: &slint::platform::software_renderer::MinimalSoftwareWindow,
    label: &str,
    value: &str,
) {
    let input = click_control(native, AccessibleRole::TextInput, label);
    let length = input
        .accessible_string_property(AccessibleStringProperty::Value)
        .unwrap_or_default()
        .len();
    input.accessible_action(&AccessibilityAction::SetSelectionOffsets(0, length as i32));
    native
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: value.into() });
    native
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text: value.into() });
}

fn select_first(native: &impl WindowAdapter) {
    let root = i_slint_core::window::WindowInner::from_pub(native.window()).component();
    fn first_row(item: ItemRc) -> Option<ItemRc> {
        if item.accessible_role() == AccessibleRole::ListItem
            && item
                .accessible_string_property(AccessibleStringProperty::Value)
                .as_deref()
                == Some("7:1")
        {
            return Some(item);
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            if let Some(found) = first_row(item) {
                return Some(found);
            }
        }
        None
    }
    first_row(ItemRc::new_root(root))
        .unwrap()
        .accessible_action(&AccessibilityAction::Default);
}

#[test]
fn large_find_results_page_by_visible_rows_and_reveal_the_exact_last_token() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    window.invoke_apply_preferences(0, 0, "".into(), 13, 17);
    let rows = Rc::new(LazyRows {
        count: 10_001,
        reads: Cell::new(0),
        notify: ModelNotify::default(),
    });
    window.set_rows(rows.clone().into());
    window.set_running(true);
    let reveals = Rc::new(RefCell::new(Vec::new()));
    window.on_reveal({
        let reveals = reveals.clone();
        move |token| reveals.borrow_mut().push(token)
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    modeless::draw(&native, 860, 620, "find-lazy-initial");
    assert!(
        rows.reads.get() < 600,
        "initial render must not enumerate 10,001 rows"
    );
    select_first(native.as_ref());
    key(native.as_ref(), Key::PageDown);
    let next = window.get_selected_row();
    assert!(
        next > 1 && next < 100,
        "PageDown must move by a visible page: {next}"
    );
    key(native.as_ref(), Key::PageUp);
    assert_eq!(window.get_selected_row(), 0);
    key(native.as_ref(), Key::End);
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        modeless::draw(&native, 860, 620, "find-lazy-last");
    }
    assert_eq!(window.get_selected_row(), 10_000);
    let last = modeless::accessible_item(
        &native,
        AccessibleRole::ListItem,
        "result-10000.txt ·  · D:\\fixtures\\文档 ·  · ",
    )
    .expect("End must materialize the last row");
    assert_visible_through_clips(&last);
    key(native.as_ref(), Key::Return);
    assert_eq!(
        reveals.borrow().as_slice(),
        &[slint::SharedString::from("7:10001")]
    );
    assert!(
        rows.reads.get() < 2_000,
        "navigation must retain lazy row access"
    );

    // Explicit wheel motion releases cursor-following; otherwise an End-selected
    // result would pull every scroll back to the bottom.
    let list = modeless::accessible_item(&native, AccessibleRole::List, "Find results").unwrap();
    let area = bounds(&list);
    native
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(area.0 + 80.0, area.1 + 40.0),
            delta_x: 0.0,
            delta_y: 1000.0,
        });
    modeless::draw(&native, 860, 620, "find-lazy-wheel");
    assert_eq!(
        window.get_selected_row(),
        10_000,
        "scrolling must not retarget selection"
    );
    assert!(
        modeless::accessible_item(
            &native,
            AccessibleRole::ListItem,
            "result-10000.txt ·  · D:\\fixtures\\文档 ·  · ",
        )
        .is_none_or(|last| bounds(&last).1 >= area.3 - 1.0),
        "wheel motion must leave the End-selected row instead of snapping back to it"
    );
    window.hide().unwrap();
}

#[test]
fn pointer_selection_does_not_reveal_and_empty_results_ignore_navigation() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    let rows = Rc::new(LazyRows {
        count: 2,
        reads: Cell::new(0),
        notify: ModelNotify::default(),
    });
    window.set_rows(rows.into());
    let reveals = Rc::new(Cell::new(0));
    window.on_reveal({
        let reveals = reveals.clone();
        move |_| reveals.set(reveals.get() + 1)
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    modeless::draw(&native, 860, 620, "find-pointer");
    let row = modeless::accessible_item(
        &native,
        AccessibleRole::ListItem,
        "result-00000.txt ·  · D:\\fixtures\\文档 ·  · ",
    )
    .unwrap();
    let area = bounds(&row);
    let position = slint::LogicalPosition::new(area.0 + 60.0, (area.1 + area.3) / 2.0);
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
    assert_eq!(window.get_selected_row(), 0);
    assert_eq!(reveals.get(), 0);
    window.set_rows(slint::ModelRc::default());
    window.set_selected_row(-1);
    for event in [
        Key::DownArrow,
        Key::UpArrow,
        Key::PageDown,
        Key::PageUp,
        Key::Home,
        Key::End,
        Key::Return,
    ] {
        key(native.as_ref(), event);
    }
    assert_eq!(window.get_selected_row(), -1);
    assert_eq!(reveals.get(), 0);
    window.hide().unwrap();
}

#[test]
fn find_advanced_options_are_explicit_and_preserve_the_basic_query_when_collapsed() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    window.invoke_apply_preferences(0, 0, "".into(), 13, 17);
    let submitted = Rc::new(RefCell::new(Vec::new()));
    window.on_search({
        let submitted = submitted.clone();
        move |draft| submitted.borrow_mut().push(draft)
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    modeless::draw(&native, 860, 620, "find-query-basic");
    let mode = modeless::accessible_item(&native, AccessibleRole::Combobox, "Find match mode")
        .expect("matching mode stays in the basic query");
    assert_visible_through_clips(&mode);
    modeless::click(&native, "More options");
    modeless::draw(&native, 860, 620, "find-query-expanded");
    let age = modeless::accessible_item(&native, AccessibleRole::Checkbox, "Modified within")
        .expect("relative modified filter must be discoverable");
    age.accessible_action(&AccessibilityAction::Default);
    let duplicates =
        modeless::accessible_item(&native, AccessibleRole::Checkbox, "Identical files")
            .expect("verified duplicate search must be discoverable");
    duplicates.accessible_action(&AccessibilityAction::Default);
    modeless::click(&native, "Fewer options");
    modeless::draw(&native, 860, 620, "find-query-collapsed");
    window.invoke_submit();
    assert_eq!(submitted.borrow().len(), 1);
    assert_eq!(submitted.borrow()[0].masks, "*.*");
    modeless::click(&native, "More options");
    modeless::draw(&native, 860, 620, "find-query-restored");
    for label in ["Modified within", "Identical files"] {
        let item = modeless::accessible_item(&native, AccessibleRole::Checkbox, label).unwrap();
        assert_eq!(
            item.accessible_string_property(AccessibleStringProperty::Checkable)
                .as_deref(),
            Some("true")
        );
        assert_eq!(
            item.accessible_string_property(AccessibleStringProperty::Checked)
                .as_deref(),
            Some("true"),
            "collapsing must preserve {label}"
        );
    }
    window.hide().unwrap();
}

#[test]
fn invalid_relative_age_keeps_the_draft_and_does_not_submit_a_search() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    let submissions = Rc::new(Cell::new(0));
    window.on_search({
        let submissions = submissions.clone();
        move |_| submissions.set(submissions.get() + 1)
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    modeless::draw(&native, 860, 620, "find-age-before");
    modeless::click(&native, "More options");
    modeless::draw(&native, 860, 620, "find-age-options");
    modeless::accessible_item(&native, AccessibleRole::Checkbox, "Modified within")
        .unwrap()
        .accessible_action(&AccessibilityAction::Default);
    let value = modeless::accessible_item(&native, AccessibleRole::TextInput, "Modified age")
        .expect("relative age is an editable value");
    for text in ["", "0", "-1", "1.5", "not a number", "999999999"] {
        value.accessible_action(&AccessibilityAction::SetValue(text.into()));
        window.invoke_submit();
        assert_eq!(
            submissions.get(),
            0,
            "invalid interval {text:?} cannot submit"
        );
        assert_eq!(
            value
                .accessible_string_property(AccessibleStringProperty::Value)
                .as_deref(),
            Some(text),
            "validation must preserve the user's invalid draft"
        );
    }
    value.accessible_action(&AccessibilityAction::SetValue("5".into()));
    window.invoke_submit();
    assert_eq!(submissions.get(), 1);
    window.hide().unwrap();
}

#[test]
fn pointer_and_keyboard_query_survives_resize_stop_and_restart() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    window.invoke_apply_preferences(0, 0, "".into(), 13, 17);
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let stops = Rc::new(Cell::new(0));
    window.on_search({
        let submitted = submitted.clone();
        move |draft| submitted.borrow_mut().push(draft)
    });
    window.on_stop({
        let stops = stops.clone();
        move || stops.set(stops.get() + 1)
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    modeless::draw(&native, 860, 620, "find-query-input");
    type_input(&native, "File name masks", "*.txt|skip*");
    click_control(&native, AccessibleRole::Checkbox, "Containing text");
    type_input(&native, "Text to find", "Žluťoučký 日本語");
    click_control(&native, AccessibleRole::Checkbox, "Match case");
    click_control(&native, AccessibleRole::Checkbox, "Whole words");
    click_control(&native, AccessibleRole::Button, "More options");
    modeless::draw(&native, 860, 620, "find-query-more");
    click_control(&native, AccessibleRole::Checkbox, "Modified within");
    type_input(&native, "Modified age", "5");
    click_control(&native, AccessibleRole::Combobox, "Modified age unit");
    key(native.as_ref(), Key::UpArrow);
    key(native.as_ref(), Key::Return);
    modeless::draw(&native, 860, 620, "find-query-minutes");
    click_control(&native, AccessibleRole::Checkbox, "Identical files");
    click_control(&native, AccessibleRole::Button, "Fewer options");
    modeless::draw(&native, 860, 620, "find-query-active-summary");
    modeless::click(&native, "Find");
    assert_eq!(submitted.borrow().len(), 1);
    let query = submitted.borrow()[0].clone();
    assert_eq!(query.masks, "*.txt|skip*");
    assert_eq!(query.content, "Žluťoučký 日本語");
    assert!(query.content_enabled && query.content_case && query.whole_words);
    assert!(query.modified_enabled && query.duplicates);
    assert_eq!(query.modified_seconds, 300);

    for (scheme, density, font, width, height) in [(1, 1, 32, 320, 200), (0, 0, 13, 860, 620)] {
        window.invoke_apply_preferences(scheme, density, "".into(), font, font);
        modeless::draw(&native, width, height, "find-query-retained");
        window.invoke_submit();
        let drafts = submitted.borrow();
        let latest = drafts.last().unwrap();
        assert_eq!(latest.masks, query.masks);
        assert_eq!(latest.content, query.content);
        assert_eq!(latest.modified_seconds, query.modified_seconds);
        assert_eq!(latest.duplicates, query.duplicates);
    }
    window.set_running(true);
    window.invoke_submit();
    assert_eq!(
        submitted.borrow().len(),
        3,
        "running query cannot be resubmitted"
    );
    modeless::draw(&native, 860, 620, "find-query-running");
    modeless::click(&native, "Stop");
    assert_eq!(stops.get(), 1);
    window.set_stopping(true);
    modeless::click(&native, "Stop");
    assert_eq!(
        stops.get(),
        1,
        "pending Stop cannot enqueue another request"
    );
    window.set_running(false);
    window.set_stopping(false);
    modeless::draw(&native, 860, 620, "find-query-stopped");
    modeless::click(&native, "Find");
    assert_eq!(submitted.borrow().len(), 4);
    assert_eq!(submitted.borrow().last().unwrap().modified_seconds, 300);
    window.hide().unwrap();
}

#[test]
fn duplicate_groups_are_visible_aligned_and_keyboard_reachable_at_minimum_size() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    window.invoke_apply_preferences(0, 0, "".into(), 13, 17);
    window.set_duplicate_results(true);
    window.set_running(true);
    window.set_rows(
        Rc::new(slint::VecModel::from(vec![
            FindRow {
                token: "9:1".into(),
                name: "first.txt".into(),
                duplicate_group: "Group 1 · 2 files".into(),
                group_start: true,
                ..Default::default()
            },
            FindRow {
                token: "9:2".into(),
                name: "copy.txt".into(),
                duplicate_group: "Group 1 · 2 files".into(),
                ..Default::default()
            },
            FindRow {
                token: "9:3".into(),
                name: "second-group.txt".into(),
                duplicate_group: "Group 2 · 2 files".into(),
                group_start: true,
                group_shade: true,
                ..Default::default()
            },
            FindRow {
                token: "9:4".into(),
                name: "second-copy.txt".into(),
                duplicate_group: "Group 2 · 2 files".into(),
                group_shade: true,
                ..Default::default()
            },
        ]))
        .into(),
    );
    let sorts = Rc::new(RefCell::new(Vec::new()));
    window.on_sort({
        let sorts = sorts.clone();
        move |column| sorts.borrow_mut().push(column)
    });
    let revealed = Rc::new(RefCell::new(Vec::new()));
    window.on_reveal({
        let revealed = revealed.clone();
        move |token| revealed.borrow_mut().push(token)
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    modeless::draw(&native, 860, 620, "find-duplicate-groups");
    let heading =
        modeless::accessible_item(&native, AccessibleRole::Text, "Identical group").unwrap();
    let group =
        modeless::accessible_item(&native, AccessibleRole::Text, "Group 1 · 2 files").unwrap();
    assert!((bounds(&heading).0 - bounds(&group).0).abs() < 1.0);
    modeless::click(&native, "Sort by Name");
    assert_eq!(sorts.borrow().as_slice(), [0]);
    let first = modeless::accessible_item(
        &native,
        AccessibleRole::ListItem,
        "Group 1 · 2 files · first.txt ·  ·  ·  · ",
    )
    .unwrap();
    first.accessible_action(&AccessibilityAction::Default);
    window.invoke_apply_preferences(1, 1, "".into(), 32, 32);
    modeless::draw(&native, 320, 200, "find-duplicate-groups-minimum");
    key(native.as_ref(), Key::End);
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        modeless::draw(&native, 320, 200, "find-duplicate-groups-last-minimum");
    }
    assert_eq!(window.get_selected_row(), 3);
    let last = modeless::accessible_item(
        &native,
        AccessibleRole::ListItem,
        "Group 2 · 2 files · second-copy.txt ·  ·  ·  · ",
    )
    .unwrap();
    assert_visible_through_clips(&last);
    key(native.as_ref(), Key::RightArrow);
    key(native.as_ref(), Key::Return);
    assert_eq!(
        revealed.borrow().as_slice(),
        &[slint::SharedString::from("9:4")]
    );
    window.hide().unwrap();
}

#[test]
fn find_form_labels_align_and_basic_controls_fit_without_scrolling() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = FindWindow::new().unwrap();
    window.set_roots(
        Rc::new(slint::VecModel::from(vec![slint::SharedString::from(
            "D:\\fixtures\\資料",
        )]))
        .into(),
    );
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    for (scheme, density, font, width, height) in [
        (0, 0, 13, 780, 560),
        (1, 1, 17, 900, 700),
        (3, 0, 22, 1100, 800),
    ] {
        window.invoke_apply_preferences(scheme, density, "".into(), font, font);
        modeless::draw(
            &native,
            width,
            height,
            &format!("find-form-{scheme}-{density}-{font}"),
        );
        let item = |role, label| {
            modeless::accessible_item(&native, role, label)
                .unwrap_or_else(|| panic!("missing {label}"))
        };
        let name = item(AccessibleRole::TextInput, "File name masks");
        let label = item(AccessibleRole::Text, "File name");
        let folders = item(AccessibleRole::Combobox, "Search folders");
        let folder_label = item(AccessibleRole::Text, "Search folders");
        let mode = item(AccessibleRole::Combobox, "Find match mode");
        let text = item(AccessibleRole::TextInput, "Text to find");
        for control in [
            &name,
            &folders,
            &mode,
            &text,
            &item(AccessibleRole::Checkbox, "Subfolders"),
            &item(AccessibleRole::Checkbox, "Match name case"),
            &item(AccessibleRole::Checkbox, "Containing text"),
            &item(AccessibleRole::Button, "More options"),
        ] {
            assert_visible_through_clips(control);
        }
        let name_area = bounds(&name);
        let folders_area = bounds(&folders);
        let text_area = bounds(&text);
        assert!(
            bounds(&label).2 <= name_area.0,
            "name needs its own visible label"
        );
        assert!((bounds(&label).0 - bounds(&folder_label).0).abs() <= 1.0);
        assert!((name_area.0 - folders_area.0).abs() <= 1.0);
        assert!((name_area.0 - text_area.0).abs() <= 1.0);
        assert!(
            bounds(&mode).0 >= name_area.2,
            "matching mode belongs beside the name"
        );
        assert!(
            bounds(&mode).2 - bounds(&mode).0 < name_area.2 - name_area.0,
            "mode is compact, name gets remaining width"
        );
        assert!(name_area.3 < folders_area.1 && folders_area.3 < text_area.1);
        let results = bounds(&item(AccessibleRole::List, "Find results"));
        assert!(
            results.3 - results.1 >= height as f32 * 0.35,
            "results keep useful space at normal sizes: {results:?}"
        );
    }
    window.hide().unwrap();
}
