//! Comparator form bindings. Native paths are labels here; services own their use.
use crate::{ComparatorOptionsView, MainWindow, Presenter};
use midnight_core::{Command, FileComparatorRule, FileComparatorTarget};
use slint::{ComponentHandle, SharedString};
use std::{cell::RefCell, rc::Rc};

pub(crate) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let view = window.global::<ComparatorOptionsView>();
    view.set_ready(presenter.comparator_settings_ready());
    view.set_busy(
        presenter.settings_picker.busy()
            || presenter.preferences.options.accepting
            || presenter.preferences.options.saving,
    );
    let revision = presenter.comparators.revision.to_string();
    if view.get_revision() == revision {
        return;
    }
    let state = &presenter.comparators;
    view.set_dirty(state.dirty);
    view.set_error(state.error.as_str().into());
    view.set_error_field(state.error_field);
    let (mut labels, selected, form, arguments) = if let Some(draft) = &state.draft {
        let selected = draft.selected_index().unwrap_or(draft.rules.len());
        let rule = draft.selected_rule();
        (
            draft
                .rules
                .iter()
                .enumerate()
                .map(|(index, rule)| rule.label(index))
                .collect::<Vec<_>>(),
            selected,
            rule.map(|rule| rule.form()).unwrap_or_default(),
            rule.map(|rule| rule.external.arguments.as_slice())
                .unwrap_or(&[]),
        )
    } else {
        let rules = &presenter.preferences.values.file_comparators.rules;
        (
            rules
                .iter()
                .enumerate()
                .map(|(index, rule)| rule_label(index, rule))
                .collect(),
            rules.len(),
            Default::default(),
            &[][..],
        )
    };
    view.set_rule_count(labels.len() as i32);
    labels.push("Other regular files → Automatic (fixed)".into());
    view.set_labels(crate::argument_rows::sync(view.get_labels(), &labels));
    view.set_selected(selected as i32);
    view.set_draft(form);
    view.set_argument_rows(crate::argument_rows::sync(
        view.get_argument_rows(),
        arguments,
    ));
    view.set_coverage_busy(state.capabilities.pending);
    let (coverage, status, error) = image_coverage(presenter);
    view.set_image_coverage(coverage);
    view.set_coverage_state(status);
    view.set_coverage_error(error);
    // Publish after row/field data so focus callbacks see this exact draft.
    view.set_focus_revision(state.focus_revision);
    view.set_revision(revision.into());
}

fn rule_label(index: usize, rule: &FileComparatorRule) -> String {
    let target = match &rule.target {
        FileComparatorTarget::Automatic => "Automatic".into(),
        FileComparatorTarget::BuiltInText => "Built-in text".into(),
        FileComparatorTarget::BuiltInImages => "Built-in image".into(),
        FileComparatorTarget::External(program) => {
            format!("External: {}", program.executable.to_string_lossy())
        }
    };
    format!(
        "{}. {}{} → {}",
        index + 1,
        if rule.enabled { "" } else { "[Off] " },
        rule.expression,
        target
    )
}

fn image_coverage(presenter: &Presenter) -> (SharedString, SharedString, SharedString) {
    let state = &presenter.comparators.capabilities;
    if !state.error.is_empty() {
        return (
            Default::default(),
            "Image support is unavailable.".into(),
            state.error.as_str().into(),
        );
    }
    let Some(display) = &state.display else {
        return (
            Default::default(),
            if state.pending {
                "Checking enabled image support…"
            } else if presenter.comparator_settings_ready() {
                "Image support metadata has not been received."
            } else {
                "Image support is unavailable until saved settings are loaded."
            }
            .into(),
            Default::default(),
        );
    };
    (
        display.coverage.clone(),
        display.status.clone(),
        display.error.clone(),
    )
}

pub(crate) fn install(
    window: &MainWindow,
    state: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) {
    let weak = window.as_weak();
    let p = state.clone();
    window
        .global::<ComparatorOptionsView>()
        .on_edit(move |form| {
            p.borrow_mut().edit_comparator_form(form);
            if let Some(window) = weak.upgrade() {
                refresh(&window, &p.borrow());
            }
        });
    let weak = window.as_weak();
    let p = state.clone();
    window
        .global::<ComparatorOptionsView>()
        .on_select(move |row| {
            p.borrow_mut().select_comparator_rule(row);
            if let Some(window) = weak.upgrade() {
                refresh(&window, &p.borrow());
            }
        });
    let weak = window.as_weak();
    let p = state.clone();
    window
        .global::<ComparatorOptionsView>()
        .on_action(move |action| {
            p.borrow_mut().comparator_rule_action(action);
            if let Some(window) = weak.upgrade() {
                refresh(&window, &p.borrow());
            }
        });
    let weak = window.as_weak();
    let p = state.clone();
    window
        .global::<ComparatorOptionsView>()
        .on_argument(move |operation, row, value| {
            p.borrow_mut()
                .edit_comparator_argument(operation, row, value.as_str());
            if let Some(window) = weak.upgrade() {
                refresh(&window, &p.borrow());
            }
        });
    for (directory, target) in [
        (
            false,
            crate::presenter::settings_picker::Target::ComparatorProgram,
        ),
        (
            true,
            crate::presenter::settings_picker::Target::ComparatorWorkingDirectory,
        ),
    ] {
        let weak = window.as_weak();
        let p = state.clone();
        let sender = send.clone();
        let browse = move || {
            if let Some(window) = weak.upgrade() {
                if window.get_options_topic_index()
                    != crate::presenter::settings_picker::OPTIONS_TOPIC_COMPARATORS
                {
                    return;
                }
                let owner = crate::picker_window::owner(&window);
                p.borrow_mut()
                    .request_settings_picker(target, owner, sender.as_ref());
                refresh(&window, &p.borrow());
                crate::settings_picker_presentation::wake(&window, &p, sender.clone());
            }
        };
        if directory {
            window
                .global::<ComparatorOptionsView>()
                .on_browse_directory(browse);
        } else {
            window.global::<ComparatorOptionsView>().on_browse(browse);
        }
    }
    let weak = window.as_weak();
    window
        .global::<ComparatorOptionsView>()
        .on_refresh_coverage(move || {
            state
                .borrow_mut()
                .request_comparator_capabilities(true, send.as_ref());
            if let Some(window) = weak.upgrade() {
                refresh(&window, &state.borrow());
            }
        });
}
