//! Compare/filter dialogs retain drafts on validation and keep actions in the viewport.
#[path = "support/render.rs"]
mod render;
use crate::support::ui;

use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
use midnight_ui::{ComparisonChoices, ComparisonView};
use slint::{ComponentHandle, platform::Key};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

fn settle(test: &mut ui::TestUi) {
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        test.draw();
    }
}

fn action_in_viewport(test: &ui::TestUi, label: &str) {
    let item = ui::popup_item(&test.window, label, AccessibleRole::Button);
    let bounds = item.geometry();
    let origin = item.map_to_native_window(bounds.origin);
    assert!(
        origin.x >= 0.0
            && origin.y >= 0.0
            && origin.x + bounds.width() <= test.width as f32
            && origin.y + bounds.height() <= test.height as f32,
        "{label} outside viewport: {origin:?} {bounds:?}"
    );
}

fn footer_actions_aligned(test: &ui::TestUi, action: &str, dismissal: &str) {
    let action = ui::popup_item(&test.window, action, AccessibleRole::Button);
    let dismissal = ui::popup_item(&test.window, dismissal, AccessibleRole::Button);
    let action_bounds = action.geometry();
    let dismiss_bounds = dismissal.geometry();
    let action_top = action.map_to_native_window(action_bounds.origin).y;
    let dismiss_top = dismissal.map_to_native_window(dismiss_bounds.origin).y;
    assert!(
        (action_top - dismiss_top).abs() < 1.0,
        "footer actions must share a top edge: {action_top} vs {dismiss_top}"
    );
    assert!((action_bounds.height() - dismiss_bounds.height()).abs() < 1.0);
}

// Tab through the real native focus chain. At the minimum size some body
// controls can be clipped; only send Space when its checkbox is in view.
// Returns `None` when Tab never focuses the checkbox, else whether Space toggled it.
fn toggle_reachable_checkbox_by_keyboard(test: &mut ui::TestUi, label: &str) -> Option<bool> {
    let mut reached = false;
    use i_slint_core::{item_tree::ParentItemTraversalMode, items::Flickable, window::WindowInner};
    for _ in 0..24 {
        ui::key(&test.window, Key::Tab);
        settle(test);
        let checkbox = ui::popup_item(&test.window, label, AccessibleRole::Checkbox);
        let inner = WindowInner::from_pub(test.window.window());
        let mut focused = inner.focus_item.borrow().upgrade();
        let mut matches = false;
        while let Some(item) = focused {
            if item == checkbox {
                matches = true;
                break;
            }
            focused = item.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        if !matches {
            continue;
        }
        reached = true;
        let bounds = checkbox.geometry();
        let origin = checkbox.map_to_native_window(bounds.origin);
        let mut visible = checkbox.is_visible()
            && origin.x >= 0.0
            && origin.y >= 0.0
            && origin.x + bounds.width() <= test.width as f32 + 0.5
            && origin.y + bounds.height() <= test.height as f32 + 0.5;
        let mut parent = checkbox.parent_item(ParentItemTraversalMode::StopAtPopups);
        while let Some(item) = parent {
            if item.downcast::<Flickable>().is_some() {
                let clip = item.map_to_native_window(item.geometry().origin);
                visible &= origin.x >= clip.x
                    && origin.y >= clip.y
                    && origin.x + bounds.width() <= clip.x + item.geometry().width() + 0.5
                    && origin.y + bounds.height() <= clip.y + item.geometry().height() + 0.5;
            }
            parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
        }
        if visible {
            ui::key(&test.window, Key::Space);
            settle(test);
            return Some(true);
        }
    }
    reached.then_some(false)
}

#[test]
fn comparison_opened_at_minimum_size_keeps_keyboard_draft_and_escape() {
    let mut test = ui::TestUi::new();
    test.resize(320, 200);
    let submissions = Rc::new(RefCell::new(Vec::new()));
    let captured = submissions.clone();
    test.window.on_submit_comparison(move |choices| {
        captured.borrow_mut().push((
            choices.by_size,
            choices.by_modified,
            choices.case_sensitive,
            choices.tolerance_seconds,
            choices.unmatched_directories,
        ));
    });
    let weak = test.window.as_weak();
    test.window.on_cancel_comparison(move || {
        let window = weak.upgrade().unwrap();
        let mut model = window.get_comparison_view();
        model.open = false;
        window.set_comparison_view(model);
    });
    test.window.set_comparison_view(ComparisonView {
        open: true,
        left_path: "C:\\文档\\left".into(),
        right_path: "C:\\文档\\right".into(),
        choices: ComparisonChoices {
            by_size: false,
            by_modified: true,
            case_sensitive: true,
            tolerance_seconds: "17".into(),
            unmatched_directories: true,
            ..Default::default()
        },
        ..Default::default()
    });
    settle(&mut test);
    let toggled = toggle_reachable_checkbox_by_keyboard(&mut test, "Compare file size")
        .expect("Tab must reach the Compare file size checkbox at 320x200");
    assert!(
        submissions.borrow().is_empty(),
        "Tab and Space never submit a comparison"
    );
    assert_eq!(
        ui::popup_item(
            &test.window,
            "Time tolerance in seconds",
            AccessibleRole::TextInput
        )
        .accessible_string_property(AccessibleStringProperty::Value)
        .unwrap(),
        "17"
    );
    assert_eq!(
        ui::popup_item(&test.window, "Compare file size", AccessibleRole::Checkbox)
            .accessible_string_property(AccessibleStringProperty::Checked)
            .unwrap()
            .as_str(),
        if toggled { "true" } else { "false" }
    );
    // No pointer event repairs focus before this Escape.
    ui::key(&test.window, Key::Escape);
    settle(&mut test);
    assert!(!test.window.get_comparison_view().open);
}

#[test]
fn comparison_dialog_keeps_draft_and_actions_visible_at_large_fonts() {
    let mut test = ui::TestUi::new();
    test.resize(1040, 820);
    let submissions = Rc::new(RefCell::new(Vec::new()));
    let captured = submissions.clone();
    test.window.on_submit_comparison(move |choices| {
        captured.borrow_mut().push((
            choices.by_size,
            choices.by_modified,
            choices.case_sensitive,
            choices.tolerance_seconds,
            choices.unmatched_directories,
        ));
    });
    let weak = test.window.as_weak();
    test.window.on_cancel_comparison(move || {
        let window = weak.upgrade().unwrap();
        let mut model = window.get_comparison_view();
        model.open = false;
        window.set_comparison_view(model);
    });
    for density in [0, 1] {
        for font in [8, 12, 32] {
            test.window.invoke_apply_fonts("".into(), font, font);
            test.window.invoke_apply_preferences(0, density);
            test.window.set_comparison_view(ComparisonView {
                open: true,
                left_path: "C:\\文档\\left".into(),
                right_path: "C:\\文档\\right".into(),
                choices: ComparisonChoices {
                    by_size: true,
                    by_modified: true,
                    tolerance_seconds: "2".into(),
                    ..Default::default()
                },
                ..Default::default()
            });
            settle(&mut test);
            action_in_viewport(&test, "Compare");
            action_in_viewport(&test, "Close");
            footer_actions_aligned(&test, "Compare", "Close");
            ui::click_popup(
                &test.window,
                "Time tolerance in seconds",
                AccessibleRole::TextInput,
            );
            ui::chord(&test.window, &[Key::Control], "a");
            ui::key(&test.window, "86401");
            let mut model = test.window.get_comparison_view();
            model.error = "Validation feedback preserves the typed draft.".into();
            test.window.set_comparison_view(model);
            settle(&mut test);
            render::snapshot(&test, &format!("comparison-{density}-{font}"));
            let mut model = test.window.get_comparison_view();
            model.busy = true;
            model.running = true;
            model.progress_counts =
                "Folder 1 of 3 · 10 folders and 200 files compared · 4 differences found".into();
            model.current_left = "C:\\文档\\left\\a very long folder name\\deeper".into();
            model.current_left_tail = "\\deeper".into();
            model.current_right = "C:\\文档\\right\\a very long folder name\\deeper".into();
            model.current_right_tail = "\\deeper".into();
            model.fraction = 0.33;
            test.window.set_comparison_view(model);
            settle(&mut test);
            action_in_viewport(&test, "Cancel");
            render::snapshot(&test, &format!("comparison-progress-{density}-{font}"));
            let mut model = test.window.get_comparison_view();
            model.busy = false;
            model.running = false;
            test.window.set_comparison_view(model);
            settle(&mut test);
            ui::click_popup(&test.window, "Compare", AccessibleRole::Button);
            assert_eq!(submissions.borrow().last().unwrap().3, "86401");
            ui::key(&test.window, Key::Escape);
            settle(&mut test);
            assert!(!test.window.get_comparison_view().open);
        }
    }
}

/// Every visible text of the open dialog popup.
fn popup_texts(window: &midnight_ui::MainWindow) -> Vec<String> {
    use i_slint_core::{
        item_tree::ItemRc,
        window::{WindowInner, WindowKind},
    };
    let inner = WindowInner::from_pub(window.window());
    let popups = inner.active_popups();
    let popup = popups
        .iter()
        .rev()
        .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .expect("an open dialog popup");
    ui::visible_text_values(ItemRc::new_root(popup.component.clone()))
        .into_iter()
        .map(|text| text.to_string())
        .collect()
}

/// A check box's accessible property in the open dialog popup.
fn checkbox(
    window: &midnight_ui::MainWindow,
    label: &str,
    which: AccessibleStringProperty,
) -> String {
    ui::popup_item(window, label, AccessibleRole::Checkbox)
        .accessible_string_property(which)
        .unwrap()
        .to_string()
}

#[test]
fn contents_and_subfolders_are_submitted_and_subfolders_disable_one_sided_folders() {
    let mut test = ui::TestUi::new();
    test.resize(1040, 820);
    let submissions = Rc::new(RefCell::new(Vec::new()));
    let captured = submissions.clone();
    test.window.on_submit_comparison(move |choices| {
        captured.borrow_mut().push((
            choices.unmatched_directories,
            choices.by_content,
            choices.recursive,
        ));
    });
    test.window.set_comparison_view(ComparisonView {
        open: true,
        choices: ComparisonChoices {
            by_size: true,
            by_modified: true,
            by_content: true,
            unmatched_directories: true,
            tolerance_seconds: "2".into(),
            ..Default::default()
        },
        ..Default::default()
    });
    settle(&mut test);
    assert_eq!(
        checkbox(
            &test.window,
            "Compare contents",
            AccessibleStringProperty::Checked
        ),
        "true"
    );
    assert_eq!(
        checkbox(
            &test.window,
            "Compare subfolders",
            AccessibleStringProperty::Checked
        ),
        "false"
    );
    assert_eq!(
        checkbox(
            &test.window,
            "Select one-sided folders",
            AccessibleStringProperty::Enabled
        ),
        "true"
    );
    assert!(
        popup_texts(&test.window)
            .iter()
            .any(|text| text == "Files with different content are selected."),
        "each criterion says what it selects"
    );
    ui::click_popup(&test.window, "Compare subfolders", AccessibleRole::Checkbox);
    settle(&mut test);
    assert_eq!(
        checkbox(
            &test.window,
            "Select one-sided folders",
            AccessibleStringProperty::Enabled
        ),
        "false",
        "with subfolders, one-sided folders always count"
    );
    ui::click_popup(&test.window, "Compare", AccessibleRole::Button);
    assert_eq!(submissions.borrow().last(), Some(&(true, true, true)));
    // A running comparison shows what it compares now, its counts and its bytes.
    let mut model = test.window.get_comparison_view();
    model.busy = true;
    model.running = true;
    model.current_left = "C:\\left\\src".into();
    model.current_right = "D:\\right\\src".into();
    model.progress_counts =
        "Folder 1 of 4 · 3 folders and 40 files compared · 0 differences found".into();
    model.progress_detail = "Contents: 1 of 4 file pairs read · 2 KiB of 8 KiB".into();
    test.window.set_comparison_view(model);
    settle(&mut test);
    let texts = popup_texts(&test.window);
    for expected in [
        "Comparing",
        "with",
        "Folder 1 of 4",
        "Contents: 1 of 4 file pairs read",
    ] {
        assert!(
            texts.iter().any(|text| text.starts_with(expected)),
            "the dialog shows {expected:?}: {texts:?}"
        );
    }
    assert!(
        !texts
            .iter()
            .any(|text| text == "Files with different content are selected."),
        "the options give way to the progress"
    );
    // Escape and Alt+C cancel a running comparison, as the Cancel button does.
    let cancelled = Rc::new(Cell::new(0));
    let count = cancelled.clone();
    test.window
        .on_cancel_comparison(move || count.set(count.get() + 1));
    ui::key(&test.window, Key::Escape);
    settle(&mut test);
    ui::chord(&test.window, &[Key::Alt], "c");
    settle(&mut test);
    ui::click_popup(&test.window, "Cancel", AccessibleRole::Button);
    settle(&mut test);
    assert_eq!(cancelled.get(), 3);
}

/// The options and the progress of Compare directories in every theme: the controls
/// keep their colours and the progress view names the pair being compared.
#[test]
fn comparison_dialog_and_progress_in_every_theme() {
    use midnight_core::PreferenceTheme;
    let mut test = ui::TestUi::new();
    test.resize(1040, 820);
    for theme in PreferenceTheme::ALL {
        test.window.set_theme_index(theme.index());
        test.window.set_comparison_view(ComparisonView {
            open: true,
            left_path: "C:\\Projects\\physics".into(),
            right_path: "D:\\Backup\\physics".into(),
            choices: ComparisonChoices {
                by_size: true,
                by_modified: true,
                recursive: true,
                tolerance_seconds: "2".into(),
                ignore_folders: true,
                ignore_folders_masks: ".git".into(),
                ..Default::default()
            },
            ..Default::default()
        });
        settle(&mut test);
        action_in_viewport(&test, "Compare");
        render::snapshot(&test, &format!("comparison-theme-{theme:?}"));
        let mut model = test.window.get_comparison_view();
        model.busy = true;
        model.running = true;
        model.current_left = "C:\\Projects\\physics\\source\\solver\\contacts".into();
        model.current_left_tail = "\\contacts".into();
        model.current_right = "D:\\Backup\\physics\\source\\solver\\contacts".into();
        model.current_right_tail = "\\contacts".into();
        model.progress_counts =
            "Folder 7 of 16 · 2 210 folders and 19 874 files compared · 12 differences found"
                .into();
        model.progress_detail =
            "Contents: 1 532 of 4 410 file pairs read · 180 MiB of 2.1 GiB".into();
        model.fraction = 0.43;
        test.window.set_comparison_view(model);
        settle(&mut test);
        action_in_viewport(&test, "Cancel");
        render::snapshot(&test, &format!("comparison-progress-theme-{theme:?}"));
        let mut model = test.window.get_comparison_view();
        model.open = false;
        model.busy = false;
        model.running = false;
        test.window.set_comparison_view(model);
        settle(&mut test);
    }
}
