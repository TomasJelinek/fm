//! Real comparator controls expose literal drafts without matching or launching files.
//! Authored before the component; NOT RUN pending the coordinated UI build lease.
#[path = "support/render.rs"]
mod render;
use crate::support::ui;

use i_slint_core::{
    accessibility::AccessibilityAction,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable},
    window::WindowInner,
};
use midnight_ui::{ComparatorOptionsDraft, ComparatorOptionsView};
use slint::{
    ComponentHandle, LogicalPosition, Model, ModelRc, SharedString, VecModel,
    platform::{Key, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

fn strings(values: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        values
            .iter()
            .map(|value| (*value).into())
            .collect::<Vec<_>>(),
    ))
}

fn settle(test: &mut ui::TestUi) {
    for _ in 0..3 {
        test.draw();
    }
}

fn open(test: &mut ui::TestUi, external: bool) {
    let window = test.window.clone_strong();
    let view = window.global::<ComparatorOptionsView>();
    view.set_ready(true);
    view.set_labels(strings(&[
        "On · *.c;*.cpp → External program",
        "Other regular files → Automatic (fixed)",
    ]));
    view.set_rule_count(1);
    view.set_selected(0);
    view.set_draft(ComparatorOptionsDraft {
        enabled: true,
        expression: "*.c;*.cpp".into(),
        target: if external { 3 } else { 0 },
        executable: "ABC".into(),
        ..Default::default()
    });
    view.set_argument_rows(strings(&["{left}", "{right}"]));
    test.window.invoke_show_options();
    test.window.set_options_topic_index(10);
    settle(test);
}

fn activate(test: &mut ui::TestUi, label: &str, role: AccessibleRole) {
    ui::popup_item(&test.window, label, role).accessible_action(&AccessibilityAction::Default);
    settle(test);
}

fn set_value(test: &mut ui::TestUi, label: &str, value: &str) {
    ui::popup_item(&test.window, label, AccessibleRole::TextInput)
        .accessible_action(&AccessibilityAction::SetValue(value.into()));
    settle(test);
}

fn choose(test: &mut ui::TestUi, label: &str, keys: &[Key]) {
    ui::popup_item(&test.window, label, AccessibleRole::Combobox)
        .accessible_action(&AccessibilityAction::Expand);
    settle(test);
    for key in keys {
        ui::key(&test.window, *key);
    }
    ui::key(&test.window, Key::Return);
    settle(test);
}

fn text(test: &ui::TestUi, label: &str) -> String {
    ui::text_values(ui::popup_item(&test.window, label, AccessibleRole::Text))
        .iter()
        .map(SharedString::as_str)
        .collect::<Vec<_>>()
        .join("\n")
}

fn focused_within(test: &ui::TestUi, control: &ItemRc) -> bool {
    let inner = WindowInner::from_pub(test.window.window());
    let mut focused = inner.focus_item.borrow().upgrade();
    while let Some(item) = focused {
        if &item == control {
            return true;
        }
        focused = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    false
}

fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let geometry = item.geometry();
    let position = item.map_to_native_window(geometry.origin);
    (
        position.x,
        position.y,
        position.x + geometry.width(),
        position.y + geometry.height(),
    )
}

fn focus_geometry(test: &ui::TestUi, expected: &ItemRc) -> String {
    let mut lines = Vec::new();
    let mut describe = |name: &str, mut current: Option<ItemRc>| {
        lines.push(format!("{name}:"));
        while let Some(item) = current {
            lines.push(format!(
                "role={:?}, label={:?}, visible={}, flickable={}, bounds={:?}",
                item.accessible_role(),
                item.accessible_string_property(
                    i_slint_core::accessibility::AccessibleStringProperty::Label
                ),
                item.is_visible(),
                item.downcast::<Flickable>().is_some(),
                bounds(&item)
            ));
            current = item.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
    };
    let inner = WindowInner::from_pub(test.window.window());
    let actual = inner.focus_item.borrow().upgrade();
    describe("actual focus and ancestors", actual);
    describe("expected field and ancestors", Some(expected.clone()));
    lines.join("\n")
}

fn inside_clips(test: &ui::TestUi, control: &ItemRc) {
    let area = bounds(control);
    assert!(
        area.0 >= 0.0
            && area.1 >= 0.0
            && area.2 <= test.width as f32 + 1.0
            && area.3 <= test.height as f32 + 1.0,
        "control outside window: {area:?}"
    );
    let mut parent = control.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(item) = parent {
        if item.downcast::<Flickable>().is_some() {
            let clip = bounds(&item);
            assert!(
                area.0 >= clip.0 - 1.0
                    && area.1 >= clip.1 - 1.0
                    && area.2 <= clip.2 + 1.0
                    && area.3 <= clip.3 + 1.0,
                "control {area:?} is clipped by {clip:?}"
            );
        }
        parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
}

/// The dialog footer keeps its actions at full size and scrolls them when they do not
/// fit beside Cancel (the spec asks for a reachable footer whose touch targets never
/// shrink), so wider system fonts can leave OK partly scrolled out at 320×200. Bring an
/// action that a scroll area clips into view with horizontal wheel input, as a user can.
fn reveal_action(test: &mut ui::TestUi, action: &ItemRc) {
    for _ in 0..8 {
        let area = bounds(action);
        let mut scroll = None;
        let mut parent = action.parent_item(ParentItemTraversalMode::StopAtPopups);
        while let Some(item) = parent {
            if item.downcast::<Flickable>().is_some() {
                let clip = bounds(&item);
                if area.2 > clip.2 + 1.0 {
                    scroll = Some((clip, clip.2 - area.2));
                } else if area.0 < clip.0 - 1.0 {
                    scroll = Some((clip, clip.0 - area.0));
                }
                if scroll.is_some() {
                    break;
                }
            }
            parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        let Some((clip, delta)) = scroll else {
            return;
        };
        test.window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: LogicalPosition::new((clip.0 + clip.2) / 2.0, (clip.1 + clip.3) / 2.0),
                delta_x: delta,
                delta_y: 0.0,
            });
        settle(test);
    }
}

#[test]
fn built_in_coverage_uses_supplied_metadata_and_refresh_does_not_edit_rules() {
    let mut test = ui::TestUi::new();
    open(&mut test, false);
    let window = test.window.clone_strong();
    let view = window.global::<ComparatorOptionsView>();
    view.set_labels(strings(&["Other regular files → Automatic (fixed)"]));
    view.set_rule_count(0);
    view.set_selected(0);
    view.set_image_coverage("*.png;*.avif".into());
    view.set_coverage_state("Enabled image decoders for this session".into());
    let edits = Rc::new(RefCell::new(0));
    let recorded = edits.clone();
    view.on_edit(move |_| *recorded.borrow_mut() += 1);
    let refreshes = Rc::new(RefCell::new(0));
    let recorded = refreshes.clone();
    view.on_refresh_coverage(move || *recorded.borrow_mut() += 1);
    settle(&mut test);
    assert_eq!(
        text(&test, "Comparator enabled image types"),
        "*.png;*.avif → Automatic → Built-in image"
    );
    assert!(text(&test, "Comparator text fallback").contains("Automatic → Built-in text"));
    assert!(text(&test, "Comparator empty overrides").contains("No overrides"));
    render::snapshot(&test, "comparator-options-normal-coverage");
    activate(
        &mut test,
        "Refresh comparator image types",
        AccessibleRole::Button,
    );
    assert_eq!(*refreshes.borrow(), 1);
    assert_eq!(*edits.borrow(), 0);
    assert!(!view.get_dirty());
    view.set_image_coverage("".into());
    view.set_coverage_state("No enabled non-document image decoders".into());
    view.set_coverage_error("Metadata could not be refreshed".into());
    settle(&mut test);
    assert_eq!(
        text(&test, "Comparator image coverage status"),
        "No enabled non-document image decoders"
    );
    assert!(
        ui::try_popup_item(
            &test.window,
            "Comparator enabled image types",
            AccessibleRole::Text
        )
        .is_none()
    );
    assert_eq!(
        text(&test, "Comparator image coverage error"),
        "Metadata could not be refreshed"
    );
}

#[test]
fn rule_actions_are_bounded_and_fixed_fallback_cannot_be_removed_or_moved() {
    let mut test = ui::TestUi::new();
    open(&mut test, false);
    let window = test.window.clone_strong();
    let view = window.global::<ComparatorOptionsView>();
    let actions = Rc::new(RefCell::new(Vec::new()));
    let recorded = actions.clone();
    view.on_action(move |action| recorded.borrow_mut().push(action));
    let selections = Rc::new(RefCell::new(Vec::new()));
    let recorded = selections.clone();
    view.on_select(move |row| recorded.borrow_mut().push(row));
    activate(&mut test, "Add comparator rule", AccessibleRole::Button);
    activate(&mut test, "Remove comparator rule", AccessibleRole::Button);
    activate(&mut test, "Move comparator rule up", AccessibleRole::Button);
    activate(
        &mut test,
        "Move comparator rule down",
        AccessibleRole::Button,
    );
    assert_eq!(actions.borrow().as_slice(), &[0, 1]);
    actions.borrow_mut().clear();
    activate(
        &mut test,
        "Other regular files → Automatic (fixed)",
        AccessibleRole::ListItem,
    );
    assert_eq!(selections.borrow().as_slice(), &[1]);
    view.set_selected(1);
    settle(&mut test);
    for label in [
        "Remove comparator rule",
        "Move comparator rule up",
        "Move comparator rule down",
    ] {
        activate(&mut test, label, AccessibleRole::Button);
    }
    ui::key(&test.window, Key::Delete);
    ui::chord(&test.window, &[Key::Alt], Key::UpArrow);
    ui::chord(&test.window, &[Key::Alt], Key::DownArrow);
    settle(&mut test);
    assert!(actions.borrow().is_empty());
    ui::key(&test.window, Key::Home);
    ui::key(&test.window, Key::End);
    assert_eq!(selections.borrow().as_slice(), &[1, 0, 1]);
}

#[test]
fn inline_fields_emit_complete_literal_drafts_and_browse_only_requests_a_picker() {
    let mut test = ui::TestUi::new();
    open(&mut test, true);
    let window = test.window.clone_strong();
    let view = window.global::<ComparatorOptionsView>();
    let edits = Rc::new(RefCell::new(Vec::new()));
    let recorded = edits.clone();
    view.on_edit(move |draft| recorded.borrow_mut().push(draft));
    let picks = Rc::new(RefCell::new(Vec::new()));
    let recorded = picks.clone();
    view.on_browse(move || recorded.borrow_mut().push("program"));
    let recorded = picks.clone();
    view.on_browse_directory(move || recorded.borrow_mut().push("directory"));
    let program = "D:\\Tools 文档\\Diff \"literal\"\\diff.exe";
    let directory = "D:\\Work 文档\\two words";
    set_value(
        &mut test,
        "Comparator filename pattern",
        "*.c;*.cpp|generated*",
    );
    set_value(&mut test, "Comparator executable", program);
    set_value(&mut test, "Comparator working directory", directory);
    render::snapshot(&test, "comparator-options-normal-external");
    activate(
        &mut test,
        "Enable this comparator rule",
        AccessibleRole::Checkbox,
    );
    activate(
        &mut test,
        "Comparator case sensitive",
        AccessibleRole::Checkbox,
    );
    let drafts = edits.borrow();
    let draft = drafts.last().unwrap();
    assert_eq!(draft.expression.as_str(), "*.c;*.cpp|generated*");
    assert_eq!(draft.executable.as_str(), program);
    assert_eq!(draft.working_directory.as_str(), directory);
    assert!(!draft.enabled);
    assert!(draft.case_sensitive);
    assert_eq!(draft.target, 3);
    drop(drafts);
    activate(
        &mut test,
        "Browse comparator executable",
        AccessibleRole::Button,
    );
    activate(
        &mut test,
        "Browse comparator working directory",
        AccessibleRole::Button,
    );
    assert_eq!(picks.borrow().as_slice(), &["program", "directory"]);
    assert_eq!(
        text(&test, "Comparator program value"),
        format!("Program: {program}")
    );
    assert_eq!(
        text(&test, "Comparator working directory value"),
        format!("Working directory: {directory}")
    );
    choose(&mut test, "Comparator matching mode", &[Key::DownArrow]);
    assert_eq!(view.get_draft().mode, 1);
    choose(
        &mut test,
        "Comparator target",
        &[Key::UpArrow, Key::UpArrow],
    );
    assert_eq!(view.get_draft().target, 1);
    assert!(
        !ui::popup_item(
            &test.window,
            "Comparator executable",
            AccessibleRole::TextInput
        )
        .is_visible()
    );
    choose(
        &mut test,
        "Comparator target",
        &[Key::DownArrow, Key::DownArrow],
    );
    assert_eq!(view.get_draft().target, 3);
    assert_eq!(view.get_draft().executable.as_str(), program);
    assert_eq!(view.get_draft().working_directory.as_str(), directory);
}

#[test]
fn comparator_argument_insertions_append_complete_items_and_empty_enter_stays_in_options() {
    let mut test = ui::TestUi::new();
    open(&mut test, true);
    let window = test.window.clone_strong();
    let view = window.global::<ComparatorOptionsView>();
    let rows = Rc::new(VecModel::from(vec![SharedString::from("文档 two words")]));
    view.set_argument_rows(rows.clone().into());
    let operations = Rc::new(RefCell::new(Vec::new()));
    let recorded = operations.clone();
    let model = rows.clone();
    view.on_argument(move |operation, row, value| {
        recorded.borrow_mut().push((operation, row, value.clone()));
        match operation {
            0 => model.set_row_data(row as usize, value),
            1 => model.push(value),
            2 => {
                model.remove(row as usize);
            }
            _ => panic!("unexpected operation in insertion test: {operation}"),
        }
    });
    settle(&mut test);
    activate(&mut test, "Insert left file", AccessibleRole::Button);
    activate(&mut test, "Insert right file", AccessibleRole::Button);
    set_value(
        &mut test,
        "Comparator argument 3",
        " --literal=\"文档\" {left} & ",
    );
    activate(&mut test, "Add argument", AccessibleRole::Button);
    assert_eq!(rows.row_count(), 3, "a pending add is not an argv item");
    ui::key(&test.window, Key::Return);
    settle(&mut test);
    assert!(
        ui::has_popup(&test.window),
        "Enter on an argument must not accept Options"
    );
    assert_eq!(
        rows.iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>(),
        [
            "文档 two words",
            "{left}",
            " --literal=\"文档\" {left} & ",
            ""
        ]
    );
    assert_eq!(operations.borrow()[0], (1, 0, "{left}".into()));
    assert_eq!(operations.borrow()[1], (1, 0, "{right}".into()));
    assert_eq!(
        text(&test, "Comparator argument help 3"),
        "Argument 3:  --literal=\"文档\" {left} & "
    );
    assert_eq!(
        text(&test, "Comparator argument help 4"),
        "Argument 4: \"\" (empty string)"
    );
    assert!(
        ui::try_popup_item(
            &test.window,
            "Insert editor file argument",
            AccessibleRole::Button
        )
        .is_none()
    );
    assert!(
        ui::try_popup_item(
            &test.window,
            "Insert argument value",
            AccessibleRole::Button
        )
        .is_none()
    );
}

#[test]
fn resizing_preserves_the_existing_external_field_and_its_caret() {
    let mut test = ui::TestUi::new();
    open(&mut test, true);
    let window = test.window.clone_strong();
    let view = window.global::<ComparatorOptionsView>();
    view.set_error("Choose a valid absolute executable path".into());
    view.set_error_field(3);
    view.set_focus_revision(1);
    settle(&mut test);
    ui::key(&test.window, Key::Home);
    ui::key(&test.window, Key::RightArrow);
    test.resize(320, 600);
    ui::key(&test.window, "X");
    assert_eq!(view.get_draft().executable.as_str(), "AXBC");
    test.resize(1100, 740);
    ui::key(&test.window, "Y");
    assert_eq!(view.get_draft().executable.as_str(), "AXYBC");
}

#[test]
fn diagnostic_init_and_revision_focus_the_specific_field_inside_the_scrolling_body() {
    let mut test = ui::TestUi::new();
    let window = test.window.clone_strong();
    let view = window.global::<ComparatorOptionsView>();
    view.set_error("Invalid filename pattern".into());
    view.set_error_field(2);
    view.set_focus_revision(1);
    open(&mut test, true);
    let pattern = ui::popup_item(
        &test.window,
        "Comparator filename pattern",
        AccessibleRole::TextInput,
    );
    render::snapshot(&test, "comparator-options-existing-error-init-focus");
    assert!(
        focused_within(&test, &pattern),
        "init must honor an existing error revision\n{}",
        focus_geometry(&test, &pattern)
    );
    inside_clips(&test, &pattern);
    let revision = view.get_focus_revision();
    ui::key(&test.window, Key::Home);
    ui::key(&test.window, Key::RightArrow);
    ui::key(&test.window, "X");
    settle(&mut test);
    assert_eq!(view.get_draft().expression.as_str(), "*X.c;*.cpp");
    assert_eq!(view.get_focus_revision(), revision);
    assert!(
        focused_within(&test, &pattern),
        "typing keeps the diagnostic caret"
    );
    ui::key(&test.window, Key::Tab);
    settle(&mut test);
    assert!(
        WindowInner::from_pub(test.window.window())
            .focus_item
            .borrow()
            .upgrade()
            .is_some(),
        "Tab must focus another Options control"
    );
    assert!(
        !focused_within(&test, &pattern),
        "the initial diagnostic request must not grab focus again after Tab"
    );
    for (field, label, role) in [
        (1, "Comparator matching mode", AccessibleRole::Combobox),
        (2, "Comparator filename pattern", AccessibleRole::TextInput),
        (3, "Comparator executable", AccessibleRole::TextInput),
        (4, "Comparator argument 1", AccessibleRole::TextInput),
        (5, "Comparator working directory", AccessibleRole::TextInput),
    ] {
        view.set_error_field(field);
        view.set_focus_revision(view.get_focus_revision() + 1);
        settle(&mut test);
        let control = ui::popup_item(&test.window, label, role);
        assert!(
            focused_within(&test, &control),
            "wrong diagnostic focus for {label}"
        );
        inside_clips(&test, &control);
    }

    test.window.invoke_cancel_options();
    settle(&mut test);
    view.set_error("".into());
    open(&mut test, true);
    let save = ui::popup_item(&test.window, "OK", AccessibleRole::Button);
    assert!(
        focused_within(&test, &save),
        "a clean comparator page must keep the ordinary Options default focus"
    );

    test.window.invoke_cancel_options();
    settle(&mut test);
    view.set_error("Diagnostic on a different page".into());
    view.set_error_field(2);
    test.window.set_options_topic_index(0);
    test.window.invoke_show_options();
    settle(&mut test);
    let save = ui::popup_item(&test.window, "OK", AccessibleRole::Button);
    assert!(
        focused_within(&test, &save),
        "a comparator diagnostic must not change another Options topic's default focus"
    );
}

#[test]
fn a_late_layout_change_reveals_the_error_field_without_another_focus_revision() {
    let mut test = ui::TestUi::new();
    test.resize(320, 200);
    open(&mut test, true);
    let window = test.window.clone_strong();
    let view = window.global::<ComparatorOptionsView>();
    view.set_error("Choose a valid working directory".into());
    view.set_error_field(5);
    view.set_focus_revision(1);
    settle(&mut test);
    let directory = ui::popup_item(
        &test.window,
        "Comparator working directory",
        AccessibleRole::TextInput,
    );
    assert!(focused_within(&test, &directory));
    inside_clips(&test, &directory);
    let revision = view.get_focus_revision();
    view.set_image_coverage("*.long-image-type;".repeat(80).into());
    settle(&mut test);
    assert_eq!(view.get_focus_revision(), revision);
    assert!(focused_within(&test, &directory));
    inside_clips(&test, &directory);
}

#[test]
fn long_values_wrap_and_error_fields_remain_reachable_across_sizes_fonts_and_themes() {
    let mut test = ui::TestUi::new();
    open(&mut test, true);
    let window = test.window.clone_strong();
    let view = window.global::<ComparatorOptionsView>();
    let pattern = "非常长的文件名".repeat(30);
    let program = format!("D:\\{}\\diff.exe", "文档 long folder ".repeat(24));
    let argument = "\"literal 文档 with spaces\"".repeat(24);
    let mut draft = view.get_draft();
    draft.expression = pattern.clone().into();
    draft.executable = program.clone().into();
    view.set_draft(draft);
    view.set_argument_rows(strings(&[&argument, "{left}", "{right}"]));
    view.set_error("Choose a valid absolute executable path".into());
    for (width, height, theme, density, font) in [
        (1100, 740, 0, 0, 12),
        (320, 200, 0, 0, 12),
        (320, 200, 1, 1, 32),
        (1100, 740, 1, 1, 32),
    ] {
        test.window.invoke_apply_preferences(theme, density);
        test.window.invoke_apply_fonts("".into(), font, font);
        test.resize(width, height);
        view.set_error_field(3);
        view.set_focus_revision(view.get_focus_revision() + 1);
        settle(&mut test);
        render::snapshot(
            &test,
            &format!(
                "comparator-options-{width}x{height}-theme{theme}-density{density}-font{font}-error"
            ),
        );
        assert_eq!(text(&test, "Selected comparator pattern"), pattern);
        assert_eq!(
            text(&test, "Comparator program value"),
            format!("Program: {program}")
        );
        assert_eq!(
            text(&test, "Comparator argument help 1"),
            format!("Argument 1: {argument}")
        );
        for label in [
            "Selected comparator pattern",
            "Comparator program value",
            "Comparator argument help 1",
        ] {
            let value = ui::popup_item(&test.window, label, AccessibleRole::Text);
            assert!(
                value.geometry().height() > (font * 2) as f32,
                "{label} must wrap"
            );
            assert!(
                value.geometry().width() <= width as f32,
                "{label} must reflow"
            );
        }
        inside_clips(
            &test,
            &ui::popup_item(
                &test.window,
                "Comparator executable",
                AccessibleRole::TextInput,
            ),
        );
        for label in ["OK", "Cancel"] {
            let action = ui::popup_item(&test.window, label, AccessibleRole::Button);
            reveal_action(&mut test, &action);
            inside_clips(&test, &action);
        }
    }
}
