//! Real presenter callbacks and asynchronous metadata responses must survive review.
#[path = "support/ui.rs"]
mod ui;
use i_slint_core::{
    accessibility::AccessibilityAction,
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use midnight_core::*;
use midnight_ui::Presenter;
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Hints,
    Dropdown,
    OpenDropdown,
    EditDropdown,
    PendingHint,
    Unchanged,
}

fn hover_hint(test: &mut ui::TestUi, label: &str, role: AccessibleRole) {
    hover_due(test, label, role);
    test.draw();
    assert!(
        WindowInner::from_pub(test.window.window())
            .active_popups()
            .iter()
            .any(|popup| matches!(popup.window_kind, WindowKind::ToolTip)),
        "visible tooltip precondition: {label}"
    );
}

fn hover_due(test: &mut ui::TestUi, label: &str, role: AccessibleRole) {
    let hovered = {
        let item = ui::popup_item(&test.window, label, role);
        let rect = item.geometry();
        let origin = item.map_to_native_window(rect.origin);
        slint::LogicalPosition::new(origin.x + rect.width() / 2., origin.y + rect.height() / 2.)
    };
    test.window
        .window()
        .dispatch_event(slint::platform::WindowEvent::PointerMoved { position: hovered });
    std::thread::sleep(Duration::from_millis(650));
}

#[test]
fn installed_metadata_callbacks_review_then_confirm_the_exact_draft() {
    review_workflow(Scenario::Hints);
}

#[test]
fn pending_review_does_not_leave_a_dropdown_owned_by_destroyed_fields() {
    review_workflow(Scenario::Dropdown);
}

#[test]
fn accessible_review_closes_an_already_open_field_dropdown() {
    review_workflow(Scenario::OpenDropdown);
}

#[test]
fn editing_a_dropdown_with_arrows_keeps_its_choices_visible() {
    review_workflow(Scenario::EditDropdown);
}

#[test]
fn due_tooltip_cannot_appear_after_review_replaces_its_field() {
    review_workflow(Scenario::PendingHint);
}

#[test]
fn reviewing_unchanged_attributes_is_safe_and_cannot_apply() {
    review_workflow(Scenario::Unchanged);
}

fn review_workflow(scenario: Scenario) {
    let mut test = ui::TestUi::new();
    let root = std::env::temp_dir();
    let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    midnight_ui::install(
        &test.window,
        state.clone(),
        Rc::new({
            let commands = commands.clone();
            move |command| {
                commands.borrow_mut().push(command);
                true
            }
        }),
    );
    let navigation = commands
        .borrow()
        .iter()
        .find_map(|c| match c {
            Command::Navigate {
                pane: PaneId::Left,
                navigation,
                ..
            } => Some(*navigation),
            _ => None,
        })
        .unwrap();
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(1),
        root.join("Žluťoučký.TXT"),
        EntryKind::File,
    )]
    .into();
    state.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    state
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    midnight_ui::refresh(&test.window, &state.borrow());
    test.draw();
    test.window.invoke_row_click(0, 1, false, false);
    test.window
        .invoke_open_metadata(0, scenario != Scenario::Unchanged);
    let id = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::PrepareOperation(request) if request.kind == MutationKind::EditMetadata => {
                Some(request.id)
            }
            _ => None,
        })
        .unwrap();
    let preview = |revision, patch| {
        Event::Operation(OperationEvent::MetadataPreview(Arc::new(MetadataPreview {
            id,
            revision,
            patch,
            capabilities: MetadataCapabilities {
                windows_attributes: EDIT_READ_ONLY | EDIT_HIDDEN | EDIT_SYSTEM | EDIT_ARCHIVE,
                compressed: true,
                modified: true,
                rename: true,
                ..Default::default()
            },
            total: 1,
            conflicts: 0,
            unsupported: 0,
            hard_linked: 0,
            sample: vec![],
            errors: vec![],
        })))
    };
    state
        .borrow_mut()
        .apply(preview(0, MetadataPatch::default()));
    midnight_ui::refresh(&test.window, &state.borrow());
    test.draw();
    if scenario == Scenario::EditDropdown {
        ui::click_popup(&test.window, "Conversion", AccessibleRole::Combobox);
        test.draw();
        for _ in 0..2 {
            ui::key(&test.window, slint::platform::Key::DownArrow);
            test.draw();
            assert!(
                WindowInner::from_pub(test.window.window())
                    .active_popups()
                    .len()
                    >= 2,
                "editing a choice must not close its dropdown"
            );
        }
        ui::key(&test.window, slint::platform::Key::Return);
        test.draw();
        ui::click_popup(&test.window, "Cancel", AccessibleRole::Button);
        return;
    }
    if scenario == Scenario::OpenDropdown {
        let review = ui::popup_item(&test.window, "Review changes", AccessibleRole::Button);
        ui::click_popup(&test.window, "Conversion", AccessibleRole::Combobox);
        test.draw();
        assert!(
            WindowInner::from_pub(test.window.window())
                .active_popups()
                .len()
                >= 2
        );
        review.accessible_action(&AccessibilityAction::Default);
        drop(review);
    } else {
        ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    }
    let (revision, patch) = commands
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::PreviewMetadata {
                revision, patch, ..
            } => Some((*revision, patch.clone())),
            _ => None,
        })
        .unwrap();
    assert_eq!(patch.is_empty(), scenario == Scenario::Unchanged);
    // While the worker prepares the review, a field tooltip can become visible.
    // Its owner is destroyed when the confirmation replaces the editing section.
    if scenario == Scenario::Dropdown {
        ui::click_popup(&test.window, "Conversion", AccessibleRole::Combobox);
        test.draw();
    } else if scenario == Scenario::PendingHint {
        hover_due(&mut test, "Change filename case", AccessibleRole::Checkbox);
    } else if scenario != Scenario::OpenDropdown {
        hover_hint(
            &mut test,
            if scenario == Scenario::Unchanged {
                "Read-only"
            } else {
                "Change filename case"
            },
            AccessibleRole::Checkbox,
        );
    }
    state.borrow_mut().apply(preview(revision, patch.clone()));
    midnight_ui::refresh(&test.window, &state.borrow());
    test.draw();
    if scenario != Scenario::Hints {
        let apply = ui::popup_item(&test.window, "Apply", AccessibleRole::Button);
        if scenario == Scenario::Unchanged {
            assert_eq!(
                apply
                    .accessible_string_property(
                        i_slint_core::accessibility::AccessibleStringProperty::Enabled
                    )
                    .unwrap(),
                "false"
            );
            apply.accessible_action(&AccessibilityAction::Default);
        }
        drop(apply);
        ui::click_popup(&test.window, "Cancel", AccessibleRole::Button);
        assert!(
            !commands
                .borrow()
                .iter()
                .any(|command| matches!(command, Command::ConfirmMetadata { .. }))
        );
        return;
    }
    hover_hint(&mut test, "Back to editing", AccessibleRole::Button);
    ui::popup_item(&test.window, "Back to editing", AccessibleRole::Button)
        .accessible_action(&AccessibilityAction::Default);
    test.draw();
    hover_hint(&mut test, "Change filename case", AccessibleRole::Checkbox);
    ui::popup_item(&test.window, "Attributes", AccessibleRole::Button)
        .accessible_action(&AccessibilityAction::Default);
    test.draw();
    ui::click_popup(&test.window, "Review changes", AccessibleRole::Button);
    let revision = commands
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::PreviewMetadata { revision, .. } => Some(*revision),
            _ => None,
        })
        .unwrap();
    state.borrow_mut().apply(preview(revision, patch));
    midnight_ui::refresh(&test.window, &state.borrow());
    test.draw();
    ui::click_popup(&test.window, "Apply", AccessibleRole::Button);
    assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::ConfirmMetadata { id: confirmed, revision: reviewed, .. }
        if *confirmed == id && *reviewed == revision)));
}
